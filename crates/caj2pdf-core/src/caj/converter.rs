// SPDX-License-Identifier: MIT

//! CAJ to PDF conversion using bounded PDF fragment reconstruction.

use super::parse_metadata;
use crate::fallible::{reserve, reserve_exact};
use crate::pdf::input::{
    FragmentKind, LinkDestinationTarget, LinkRepairCandidate, LinkRepairKind, PatchedSource,
    inspect_fragment_object, inspect_link_destination_candidate, scan_fragment_objects,
};
use crate::pdf::{
    FragmentObject, FragmentPlan, PdfRange, PdfRef, reconstruct_fragment_with_bookmarks,
};
use crate::{
    Cancellation, ConversionOptions, ConversionReport, Error, Limits, PdfErrorKind, RangedSource,
    Result, SequentialSink,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

struct CountingSource<'a, S> {
    source: &'a mut S,
    bytes_read: u64,
}

impl<S: RangedSource> RangedSource for CountingSource<'_, S> {
    fn size(&self) -> u64 {
        self.source.size()
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        let read = self.source.read_at(offset, destination).await?;
        if read > destination.len() {
            return Err(Error::InvalidInput {
                reason: "CAJ source reported more bytes than requested",
            });
        }
        self.bytes_read = self
            .bytes_read
            .checked_add(read as u64)
            .ok_or(Error::InvalidInput {
                reason: "CAJ input byte counter overflows",
            })?;
        Ok(read)
    }
}

/// Expose small generated page-tree objects after the immutable CAJ source.
/// Reads never copy an original PDF object into memory.
struct ExtendedSource<'a, S> {
    source: &'a mut S,
    base: u64,
    suffix: &'a [u8],
}

impl<'a, S: RangedSource> ExtendedSource<'a, S> {
    fn new(source: &'a mut S, suffix: &'a [u8]) -> Result<Self> {
        let base = source.size();
        base.checked_add(suffix.len() as u64)
            .ok_or(Error::InvalidInput {
                reason: "CAJ synthetic page-tree range overflows",
            })?;
        Ok(Self {
            source,
            base,
            suffix,
        })
    }
}

impl<S: RangedSource> RangedSource for ExtendedSource<'_, S> {
    fn size(&self) -> u64 {
        self.base + self.suffix.len() as u64
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        if offset < self.base {
            let available = (self.base - offset).min(destination.len() as u64) as usize;
            self.source
                .read_at(offset, &mut destination[..available])
                .await
        } else if offset < self.size() {
            let start = (offset - self.base) as usize;
            let length = (self.suffix.len() - start).min(destination.len());
            destination[..length].copy_from_slice(&self.suffix[start..start + length]);
            Ok(length)
        } else {
            Ok(0)
        }
    }
}

#[derive(Clone, Copy)]
struct TreeNode {
    parent: Option<PdfRef>,
    resolved_root: Option<PageRoot>,
}

#[derive(Clone, Copy)]
enum PageRoot {
    Existing(PdfRef),
    Missing {
        parent: PdfRef,
        direct_child: PdfRef,
    },
}

#[derive(Clone, Copy)]
struct MissingReference {
    owner: PdfRef,
    target: PdfRef,
    page_parent_only: bool,
}

#[derive(Default)]
struct MissingGroup {
    count: u32,
    kids: Vec<PdfRef>,
    seen: BTreeSet<PdfRef>,
}

#[derive(Clone, Copy)]
struct SyntheticPageTree<'a> {
    number: u32,
    parent: Option<u32>,
    kids: &'a [PdfRef],
    count: u32,
}

/// Append formatted PDF syntax without allocating a second page-tree body.
/// The caller reserves and caps the entire synthetic suffix first.
struct BoundedSuffix<'a> {
    bytes: &'a mut Vec<u8>,
    maximum_len: usize,
}

impl std::fmt::Write for BoundedSuffix<'_> {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        let end = self
            .bytes
            .len()
            .checked_add(text.len())
            .ok_or(std::fmt::Error)?;
        if end > self.maximum_len {
            return Err(std::fmt::Error);
        }
        self.bytes.extend_from_slice(text.as_bytes());
        Ok(())
    }
}

/// Format one synthetic `/Pages` object. Formatting a `u32` cannot fail, so
/// the only error is text past the suffix's reserved bound.
fn write_page_tree(body: &mut BoundedSuffix<'_>, node: &SyntheticPageTree<'_>) -> Result<()> {
    let mut format = || {
        write!(body, "{} 0 obj\n<< /Type /Pages ", node.number)?;
        if let Some(parent) = node.parent {
            write!(body, "/Parent {parent} 0 R ")?;
        }
        write!(body, "/Count {} /Kids [", node.count)?;
        for child in node.kids {
            write!(body, "{} 0 R ", child.number)?;
        }
        body.write_str("] >>\nendobj\n")
    };
    format().map_err(|_| Error::InvalidInput {
        reason: "CAJ synthetic page tree exceeds reserved bound",
    })
}

fn malformed(offset: u64, reason: &'static str) -> Error {
    Error::Caj {
        offset,
        record: None,
        reason,
    }
}

fn missing_reference(objects: &[FragmentObject], owner: PdfRef) -> Error {
    Error::Pdf {
        offset: objects
            .iter()
            .find(|object| object.reference == owner)
            .map_or(0, |object| object.range.offset),
        object: Some((owner.number, owner.generation)),
        kind: PdfErrorKind::Malformed,
        reason: "indirect reference targets a missing object",
    }
}

fn resolve_page_root(
    nodes: &mut BTreeMap<PdfRef, TreeNode>,
    occupied: &[PdfRef],
    page: PdfRef,
    page_offset: u64,
    body_start: u64,
) -> Result<PageRoot> {
    let mut child = page;
    let mut steps = 0usize;
    let resolved = loop {
        steps += 1;
        if steps > nodes.len().saturating_add(1) {
            return Err(malformed(body_start, "PDF page-tree parent cycle"));
        }
        let failure = malformed(page_offset, "CAJ page object is missing or is not a Page");
        let node = nodes.get(&child).ok_or(failure)?;
        if let Some(root) = node.resolved_root {
            break root;
        }
        match node.parent {
            Some(parent) if nodes.contains_key(&parent) => child = parent,
            Some(parent) => {
                if occupied.binary_search(&parent).is_ok() {
                    return Err(malformed(
                        page_offset,
                        "PDF page parent is not a Pages object",
                    ));
                }
                break PageRoot::Missing {
                    parent,
                    direct_child: child,
                };
            }
            None => break PageRoot::Existing(child),
        }
    };

    // A second walk fills the existing node index without retaining a path
    // vector. Later pages under the same group stop at the first cached node.
    let terminal = match resolved {
        PageRoot::Existing(root) => root,
        PageRoot::Missing { direct_child, .. } => direct_child,
    };
    let mut current = page;
    loop {
        let node = nodes.get_mut(&current).ok_or(Error::InvalidInput {
            reason: "resolved page-tree path changed during caching",
        })?;
        if node.resolved_root.is_some() {
            break;
        }
        let parent = node.parent;
        node.resolved_root = Some(resolved);
        if current == terminal {
            break;
        }
        current = parent.ok_or(Error::InvalidInput {
            reason: "resolved page-tree path lost its parent",
        })?;
    }
    Ok(resolved)
}

fn replace_object(
    objects: &mut [FragmentObject],
    suffix: &mut Vec<u8>,
    base: u64,
    candidate: &mut LinkRepairCandidate,
    limits: &Limits,
) -> Result<()> {
    let next_size = suffix
        .len()
        .checked_add(candidate.replacement.len())
        .ok_or(Error::InvalidInput {
            reason: "CAJ repair suffix size overflows",
        })?;
    limits.check_allocation(next_size as u64)?;
    let refused = limits.allocation_refused("CAJ link repair allocation", next_size as u64);
    reserve(suffix, candidate.replacement.len(), refused)?;
    let old = objects
        .iter_mut()
        .find(|object| object.reference == candidate.object)
        .ok_or(Error::InvalidInput {
            reason: "CAJ link repair object is absent from fragment plan",
        })?;
    old.range = PdfRange {
        offset: base
            .checked_add(suffix.len() as u64)
            .ok_or(Error::InvalidInput {
                reason: "CAJ link repair object offset overflows",
            })?,
        length: candidate.replacement.len() as u64,
    };
    suffix.extend_from_slice(&candidate.replacement);
    candidate.replacement = Vec::new();
    Ok(())
}

fn retain_repair(
    repaired: &mut BTreeMap<PdfRef, LinkRepairCandidate>,
    retained_bytes: &mut usize,
    candidate: LinkRepairCandidate,
    suffix_len: usize,
    limits: &Limits,
    offset: u64,
) -> Result<()> {
    let is_new = !repaired.contains_key(&candidate.object);
    let count = repaired
        .len()
        .checked_add(usize::from(is_new))
        .ok_or(Error::InvalidInput {
            reason: "CAJ retained repair count overflows",
        })?;
    let next_bytes = retained_bytes
        .checked_sub(
            repaired
                .get(&candidate.object)
                .map_or(0, |old| old.replacement.len()),
        )
        .and_then(|size| size.checked_add(candidate.replacement.len()))
        .ok_or(Error::InvalidInput {
            reason: "CAJ retained repair size overflows",
        })?;
    let retained = next_bytes
        .checked_add(suffix_len)
        .and_then(|size| {
            count
                .checked_mul(128)
                .and_then(|overhead| size.checked_add(overhead))
        })
        .ok_or(Error::InvalidInput {
            reason: "CAJ retained repair size overflows",
        })?;
    if retained as u64 > limits.max_allocation_bytes {
        return Err(Error::CajLimitExceeded {
            offset,
            record: None,
            resource: "retained link repairs",
            limit: limits.max_allocation_bytes,
            attempted: retained as u64,
        });
    }
    repaired.insert(candidate.object, candidate);
    *retained_bytes = next_bytes;
    Ok(())
}

fn push_synthetic(
    suffix: &mut Vec<u8>,
    objects: &mut Vec<FragmentObject>,
    base: u64,
    node: SyntheticPageTree<'_>,
    limits: &Limits,
) -> Result<()> {
    let estimated = node
        .kids
        .len()
        .checked_mul(24)
        .and_then(|bytes| bytes.checked_add(160))
        .and_then(|bytes| bytes.checked_add(suffix.len()))
        .ok_or(Error::InvalidInput {
            reason: "CAJ synthetic page tree estimate overflows",
        })?;
    limits.check_allocation(estimated as u64)?;
    let refused = limits.allocation_refused("CAJ synthetic page tree allocation", estimated as u64);
    let additional = estimated - suffix.len();
    reserve_exact(suffix, additional, refused)?;
    let start = suffix.len();
    let mut body = BoundedSuffix {
        bytes: suffix,
        maximum_len: estimated,
    };
    let number = node.number;
    write_page_tree(&mut body, &node)?;
    let body_len = body.bytes.len() - start;
    objects.push(FragmentObject {
        reference: PdfRef {
            number,
            generation: 0,
        },
        range: PdfRange {
            offset: base.checked_add(start as u64).ok_or(Error::InvalidInput {
                reason: "CAJ synthetic page tree offset overflows",
            })?,
            length: body_len as u64,
        },
    });
    Ok(())
}

/// Convert a CAJ source to a forward-only PDF sink. The source must support
/// stable positioned reads. Page payloads are never materialized in a `Vec`;
/// only page/outline metadata, object positions, and small missing page-tree
/// dictionaries are retained. All PDF object validation precedes output.
pub async fn convert_caj<S: RangedSource, W: SequentialSink, C: Cancellation>(
    source: &mut S,
    sink: &mut W,
    options: ConversionOptions,
    limits: &Limits,
    cancellation: &C,
) -> Result<ConversionReport> {
    let mut counted = CountingSource {
        source,
        bytes_read: 0,
    };
    let metadata = parse_metadata(&mut counted, limits, cancellation).await?;
    let scan = scan_fragment_objects(
        &mut counted,
        metadata.body_start,
        metadata.body_end_hint,
        limits,
        cancellation,
    )
    .await?;
    let mut objects = scan.objects;
    let source_object_count = objects.len();
    // `parse_metadata` admitted the larger page-row index under the same
    // allocation limit, so this smaller index needs no second check.
    const _: () = assert!(size_of::<PdfRef>() <= size_of::<super::CajPageRow>());
    let page_ref_bytes = (metadata.page_rows.len() as u64) * size_of::<PdfRef>() as u64;
    debug_assert!(limits.check_allocation(page_ref_bytes).is_ok());
    let mut page_refs = Vec::new();
    let refused = limits.allocation_refused("CAJ ordered page index allocation", page_ref_bytes);
    reserve_exact(&mut page_refs, metadata.page_rows.len(), refused)?;
    page_refs.extend(metadata.page_rows.iter().map(|row| PdfRef {
        number: row.page_object_id,
        generation: 0,
    }));

    // Classify page-tree nodes using the already bounded PDF parser. The
    // source page table supplies page order but not the missing /Pages nodes.
    let mut nodes = BTreeMap::<PdfRef, TreeNode>::new();
    let occupied_bytes = objects
        .len()
        .checked_mul(std::mem::size_of::<PdfRef>())
        .ok_or(Error::InvalidInput {
            reason: "CAJ object reference index overflows",
        })?;
    limits.check_allocation(occupied_bytes as u64)?;
    let mut occupied = Vec::<PdfRef>::new();
    let refused = limits.allocation_refused(
        "CAJ object reference index allocation",
        occupied_bytes as u64,
    );
    reserve_exact(&mut occupied, objects.len(), refused)?;
    occupied.extend(objects.iter().map(|object| object.reference));
    occupied.sort_unstable();
    let mut highest_referenced_object = occupied
        .iter()
        .map(|reference| reference.number)
        .max()
        .unwrap_or(0);
    let mut missing_references = Vec::<MissingReference>::new();
    {
        let mut patched = PatchedSource::new(&mut counted, &scan.patches);
        for object in &objects {
            let inspected = inspect_fragment_object(
                &mut patched,
                object.range,
                object.reference,
                limits,
                cancellation,
                |_| None,
            )
            .await?;
            highest_referenced_object =
                highest_referenced_object.max(inspected.max_referenced_object);
            let page_parent = match &inspected.kind {
                FragmentKind::Page { parent, .. } => Some(*parent),
                FragmentKind::Pages { parent, .. } => *parent,
                _ => None,
            };
            let parent_occurrences = inspected
                .references
                .iter()
                .filter(|reference| Some(**reference) == page_parent)
                .count();
            for missing in inspected
                .references
                .iter()
                .filter(|r| occupied.binary_search(r).is_err())
            {
                let attempted = missing_references
                    .len()
                    .checked_add(1)
                    .and_then(|count| count.checked_mul(std::mem::size_of::<MissingReference>()))
                    .ok_or(Error::InvalidInput {
                        reason: "CAJ missing reference index overflows",
                    })?;
                if attempted as u64 > limits.max_allocation_bytes {
                    return Err(Error::CajLimitExceeded {
                        offset: object.range.offset,
                        record: None,
                        resource: "missing PDF references",
                        limit: limits.max_allocation_bytes,
                        attempted: attempted as u64,
                    });
                }
                let refused = Error::CajLimitExceeded {
                    offset: object.range.offset,
                    record: None,
                    resource: "missing PDF references",
                    limit: limits.max_allocation_bytes,
                    attempted: attempted as u64,
                };
                reserve(&mut missing_references, 1, refused)?;
                missing_references.push(MissingReference {
                    owner: object.reference,
                    target: *missing,
                    page_parent_only: Some(*missing) == page_parent && parent_occurrences == 1,
                });
            }
            let parent = match inspected.kind {
                FragmentKind::Page { parent, .. } => Some(parent),
                FragmentKind::Pages { parent, .. } => parent,
                _ => continue,
            };
            if nodes
                .insert(
                    object.reference,
                    TreeNode {
                        parent,
                        resolved_root: None,
                    },
                )
                .is_some()
            {
                return Err(malformed(
                    object.range.offset,
                    "duplicate PDF page-tree object",
                ));
            }
        }
    }

    let mut missing = BTreeMap::<PdfRef, MissingGroup>::new();
    let mut missing_order = Vec::<PdfRef>::new();
    let mut present_root = None;
    for (index, page) in page_refs.iter().copied().enumerate() {
        match resolve_page_root(
            &mut nodes,
            &occupied,
            page,
            metadata.page_rows[index].offset,
            metadata.body_start,
        )? {
            PageRoot::Missing {
                parent,
                direct_child,
            } => {
                let group = missing.entry(parent).or_insert_with(|| {
                    missing_order.push(parent);
                    MissingGroup::default()
                });
                let failure = malformed(
                    metadata.page_rows[index].offset,
                    "CAJ page-tree count overflows",
                );
                group.count = group.count.checked_add(1).ok_or(failure)?;
                if group.seen.insert(direct_child) {
                    group.kids.push(direct_child);
                }
            }
            PageRoot::Existing(child) => {
                if present_root
                    .replace(child)
                    .is_some_and(|root| root != child)
                {
                    return Err(malformed(
                        metadata.page_rows[index].offset,
                        "CAJ pages have multiple existing roots",
                    ));
                }
            }
        }
    }
    if present_root.is_some() && !missing.is_empty() {
        return Err(malformed(
            metadata.body_start,
            "CAJ pages have mixed page-tree roots",
        ));
    }

    let base = counted.size();
    let mut suffix = Vec::new();
    let root = if let Some(root) = present_root {
        root
    } else if missing_order.len() == 1 {
        missing_order[0]
    } else {
        let highest = missing_order
            .iter()
            .fold(highest_referenced_object, |highest, reference| {
                highest.max(reference.number)
            });
        let failure = malformed(
            metadata.body_start,
            "CAJ synthetic page-tree object number overflows",
        );
        let number = highest.checked_add(1).ok_or(failure)?;
        PdfRef {
            number,
            generation: 0,
        }
    };
    for reference in &missing_order {
        let group = &missing[reference];
        let parent = (missing_order.len() > 1).then_some(root.number);
        push_synthetic(
            &mut suffix,
            &mut objects,
            base,
            SyntheticPageTree {
                number: reference.number,
                parent,
                kids: &group.kids,
                count: group.count,
            },
            limits,
        )?;
    }
    if missing_order.len() > 1 {
        push_synthetic(
            &mut suffix,
            &mut objects,
            base,
            SyntheticPageTree {
                number: root.number,
                parent: None,
                kids: &missing_order,
                count: metadata.page_count,
            },
            limits,
        )?;
    }
    if objects.iter().all(|object| object.reference != root) {
        return Err(malformed(
            metadata.body_start,
            "CAJ page-tree root is missing",
        ));
    }

    // A few observed fragments contain link annotations aimed at pages that
    // were omitted from the CAJ page table. Remove only those broken /Dest
    // entries. An indirect destination array is nullified after every object
    // referring to it has been confirmed to be a matching link annotation.
    let mut dangling_count = 0usize;
    for index in 0..missing_references.len() {
        let reference = missing_references[index];
        if missing.contains_key(&reference.target) {
            if !reference.page_parent_only {
                return Err(missing_reference(&objects, reference.owner));
            }
        } else {
            missing_references[dangling_count] = reference;
            dangling_count += 1;
        }
    }
    missing_references.truncate(dangling_count);
    missing_references.sort_unstable_by_key(|reference| (reference.owner, reference.target));
    missing_references.dedup_by_key(|reference| (reference.owner, reference.target));
    if !missing_references.is_empty() {
        let mut repaired = BTreeMap::<PdfRef, LinkRepairCandidate>::new();
        let mut retained_repair_bytes = 0usize;
        let mut scalar_destinations = BTreeSet::<PdfRef>::new();
        let mut patched = PatchedSource::new(&mut counted, &scan.patches);
        let mut first = 0usize;
        while first < missing_references.len() {
            let owner = missing_references[first].owner;
            let mut last = first + 1;
            while last < missing_references.len() && missing_references[last].owner == owner {
                last += 1;
            }
            let targets = &missing_references[first..last];
            let fragment = objects[..source_object_count]
                .iter()
                .find(|object| object.reference == owner)
                .copied()
                .ok_or_else(|| missing_reference(&objects, owner))?;
            let candidate =
                inspect_link_destination_candidate(&mut patched, fragment, limits, cancellation)
                    .await?
                    .ok_or_else(|| missing_reference(&objects, owner))?;
            let LinkDestinationTarget::DirectPage(target) = candidate.target else {
                return Err(missing_reference(&objects, owner));
            };
            if targets.len() != 1 || targets[0].target != target || page_refs.contains(&target) {
                return Err(missing_reference(&objects, owner));
            }
            if candidate.kind == LinkRepairKind::ScalarDestination {
                scalar_destinations.insert(owner);
            }
            retain_repair(
                &mut repaired,
                &mut retained_repair_bytes,
                candidate,
                suffix.len(),
                limits,
                fragment.range.offset,
            )?;
            first = last;
        }
        let mut linked_destinations = BTreeSet::<PdfRef>::new();
        if !scalar_destinations.is_empty() {
            for fragment in &objects[..source_object_count] {
                let inspection = inspect_fragment_object(
                    &mut patched,
                    fragment.range,
                    fragment.reference,
                    limits,
                    cancellation,
                    |_| None,
                )
                .await?;
                let mut referenced_scalar = None;
                for reference in &inspection.references {
                    if scalar_destinations.contains(reference)
                        && referenced_scalar
                            .replace(*reference)
                            .is_some_and(|prior| prior != *reference)
                    {
                        return Err(missing_reference(&objects, fragment.reference));
                    }
                }
                let Some(scalar) = referenced_scalar else {
                    continue;
                };
                let candidate = inspect_link_destination_candidate(
                    &mut patched,
                    *fragment,
                    limits,
                    cancellation,
                )
                .await?
                .ok_or_else(|| missing_reference(&objects, fragment.reference))?;
                if candidate.kind != LinkRepairKind::Link
                    || candidate.target != LinkDestinationTarget::IndirectArray(scalar)
                    || candidate.retains_destination_reference
                {
                    return Err(missing_reference(&objects, fragment.reference));
                }
                retain_repair(
                    &mut repaired,
                    &mut retained_repair_bytes,
                    candidate,
                    suffix.len(),
                    limits,
                    fragment.range.offset,
                )?;
                linked_destinations.insert(scalar);
            }
            if let Some(unlinked) = scalar_destinations.difference(&linked_destinations).next() {
                return Err(missing_reference(&objects, *unlinked));
            }
        }
        for candidate in repaired.values_mut() {
            replace_object(&mut objects, &mut suffix, base, candidate, limits)?;
        }
    }

    let plan = FragmentPlan {
        objects: &objects,
        pages: &page_refs,
        pages_root: root,
        catalog: None,
    };
    let bookmarks = if options.include_bookmarks {
        metadata.bookmarks.as_slice()
    } else {
        &[]
    };
    let mut patched = PatchedSource::new(&mut counted, &scan.patches);
    let mut extended = ExtendedSource::new(&mut patched, &suffix)?;
    let mut report = reconstruct_fragment_with_bookmarks(
        &mut extended,
        sink,
        &plan,
        bookmarks,
        limits,
        cancellation,
    )
    .await?;
    report.input_bytes_read = counted.bytes_read;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::SeekableSource;
    use crate::test_support::ready;
    use std::io::Cursor;

    #[test]
    fn synthetic_page_tree_appends_within_budget_at_maximum_object_width() {
        let kids = [
            PdfRef {
                number: u32::MAX - 1,
                generation: 0,
            },
            PdfRef {
                number: u32::MAX,
                generation: 0,
            },
        ];
        let node = SyntheticPageTree {
            number: u32::MAX,
            parent: Some(u32::MAX - 1),
            kids: &kids,
            count: 2,
        };
        let mut suffix = b"prefix".to_vec();
        let mut objects = Vec::new();
        let estimated = suffix.len() + 160 + 24 * kids.len();
        let limits = Limits {
            io_chunk_bytes: 1,
            max_allocation_bytes: estimated as u64,
            ..Limits::default()
        };
        limits.validate().unwrap();
        push_synthetic(&mut suffix, &mut objects, 100, node, &limits).unwrap();
        let body = &suffix[b"prefix".len()..];
        assert_eq!(
            body,
            b"4294967295 0 obj\n<< /Type /Pages /Parent 4294967294 0 R /Count 2 /Kids [4294967294 0 R 4294967295 0 R ] >>\nendobj\n"
        );
        assert!(suffix.len() <= estimated);
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].range.offset, 106);
        assert_eq!(objects[0].range.length, body.len() as u64);

        let mut too_small = b"prefix".to_vec();
        let mut rejected_objects = Vec::new();
        let limits = Limits {
            io_chunk_bytes: 1,
            max_allocation_bytes: estimated as u64 - 1,
            ..Limits::default()
        };
        limits.validate().unwrap();
        assert!(matches!(
            push_synthetic(&mut too_small, &mut rejected_objects, 100, node, &limits),
            Err(Error::LimitExceeded { .. })
        ));
        assert_eq!(too_small, b"prefix");
        assert!(rejected_objects.is_empty());
    }

    #[test]
    fn extended_source_splits_reads_at_the_suffix_and_ends_cleanly() {
        let mut base = SeekableSource::new(Cursor::new(b"abc")).unwrap();
        let suffix = b"XYZ";
        let mut extended = ExtendedSource::new(&mut base, suffix).unwrap();
        assert_eq!(extended.size(), 6);

        let mut buffer = [0u8; 8];
        // A read crossing the boundary stops at the immutable source end.
        assert_eq!(ready(extended.read_at(1, &mut buffer)).unwrap(), 2);
        assert_eq!(&buffer[..2], b"bc");
        assert_eq!(ready(extended.read_at(4, &mut buffer)).unwrap(), 2);
        assert_eq!(&buffer[..2], b"YZ");
        assert_eq!(ready(extended.read_at(6, &mut buffer)).unwrap(), 0);
        assert_eq!(ready(extended.read_at(u64::MAX, &mut buffer)).unwrap(), 0);
    }

    #[test]
    fn bounded_suffix_refuses_text_past_its_reserved_length() {
        let mut bytes = b"12".to_vec();
        let mut suffix = BoundedSuffix {
            bytes: &mut bytes,
            maximum_len: 5,
        };
        suffix.write_str("345").unwrap();
        assert!(suffix.write_str("6").is_err());
        assert!(write!(&mut suffix, "{}", 7).is_err());
        suffix.write_str("").unwrap();
        assert_eq!(bytes, b"12345");
    }

    #[test]
    fn synthetic_page_tree_formatting_stops_at_the_reserved_bound() {
        let kids = [PdfRef {
            number: 7,
            generation: 0,
        }];
        let node = SyntheticPageTree {
            number: 9,
            parent: Some(3),
            kids: &kids,
            count: 1,
        };
        let expected =
            b"9 0 obj\n<< /Type /Pages /Parent 3 0 R /Count 1 /Kids [7 0 R ] >>\nendobj\n";
        for maximum_len in [0, 20, 30, 45, 50, expected.len() - 1] {
            let mut bytes = Vec::new();
            let mut suffix = BoundedSuffix {
                bytes: &mut bytes,
                maximum_len,
            };
            assert!(matches!(
                write_page_tree(&mut suffix, &node),
                Err(Error::InvalidInput {
                    reason: "CAJ synthetic page tree exceeds reserved bound"
                })
            ));
            assert!(bytes.len() <= maximum_len);
        }
        let mut bytes = Vec::new();
        let mut suffix = BoundedSuffix {
            bytes: &mut bytes,
            maximum_len: expected.len(),
        };
        write_page_tree(&mut suffix, &node).unwrap();
        assert_eq!(bytes, expected);
    }
}
