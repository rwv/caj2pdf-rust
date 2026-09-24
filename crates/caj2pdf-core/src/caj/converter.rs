// SPDX-License-Identifier: MIT

//! CAJ to PDF conversion using bounded PDF fragment reconstruction.

use super::parse_metadata;
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

struct SyntheticPageTree<'a> {
    number: u32,
    parent: Option<u32>,
    kids: &'a [PdfRef],
    count: u32,
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
    suffix
        .try_reserve(candidate.replacement.len())
        .map_err(|_| Error::LimitExceeded {
            resource: "CAJ link repair allocation",
            limit: limits.max_allocation_bytes,
            attempted: next_size as u64,
        })?;
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
    let mut body = String::new();
    let number = node.number;
    write!(&mut body, "{number} 0 obj\n<< /Type /Pages ").map_err(|_| Error::InvalidInput {
        reason: "CAJ synthetic page tree formatting failed",
    })?;
    if let Some(parent) = node.parent {
        write!(&mut body, "/Parent {parent} 0 R ").map_err(|_| Error::InvalidInput {
            reason: "CAJ synthetic page tree formatting failed",
        })?;
    }
    write!(&mut body, "/Count {} /Kids [", node.count).map_err(|_| Error::InvalidInput {
        reason: "CAJ synthetic page tree formatting failed",
    })?;
    for child in node.kids {
        write!(&mut body, "{} 0 R ", child.number).map_err(|_| Error::InvalidInput {
            reason: "CAJ synthetic page tree formatting failed",
        })?;
    }
    body.push_str("] >>\nendobj\n");
    let next_size = suffix
        .len()
        .checked_add(body.len())
        .ok_or(Error::InvalidInput {
            reason: "CAJ synthetic page tree size overflows",
        })?;
    limits.check_allocation(next_size as u64)?;
    suffix
        .try_reserve(body.len())
        .map_err(|_| Error::LimitExceeded {
            resource: "CAJ synthetic page tree allocation",
            limit: limits.max_allocation_bytes,
            attempted: next_size as u64,
        })?;
    let start = suffix.len() as u64;
    suffix.extend_from_slice(body.as_bytes());
    objects.push(FragmentObject {
        reference: PdfRef {
            number,
            generation: 0,
        },
        range: PdfRange {
            offset: base.checked_add(start).ok_or(Error::InvalidInput {
                reason: "CAJ synthetic page tree offset overflows",
            })?,
            length: body.len() as u64,
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
    limits.check_allocation(
        (metadata.page_rows.len() as u64) * std::mem::size_of::<PdfRef>() as u64,
    )?;
    let mut page_refs = Vec::new();
    page_refs
        .try_reserve_exact(metadata.page_rows.len())
        .map_err(|_| Error::LimitExceeded {
            resource: "CAJ ordered page index allocation",
            limit: limits.max_allocation_bytes,
            attempted: (metadata.page_rows.len() as u64) * std::mem::size_of::<PdfRef>() as u64,
        })?;
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
    occupied
        .try_reserve_exact(objects.len())
        .map_err(|_| Error::LimitExceeded {
            resource: "CAJ object reference index allocation",
            limit: limits.max_allocation_bytes,
            attempted: occupied_bytes as u64,
        })?;
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
                missing_references
                    .try_reserve(1)
                    .map_err(|_| Error::CajLimitExceeded {
                        offset: object.range.offset,
                        record: None,
                        resource: "missing PDF references",
                        limit: limits.max_allocation_bytes,
                        attempted: attempted as u64,
                    })?;
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
                .insert(object.reference, TreeNode { parent })
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
        let mut child = page;
        let mut steps = 0usize;
        loop {
            steps += 1;
            if steps > nodes.len().saturating_add(1) {
                return Err(malformed(metadata.body_start, "PDF page-tree parent cycle"));
            }
            let node = nodes.get(&child).ok_or_else(|| {
                malformed(
                    metadata.page_rows[index].offset,
                    "CAJ page object is missing or is not a Page",
                )
            })?;
            match node.parent {
                Some(parent) if nodes.contains_key(&parent) => child = parent,
                Some(parent) => {
                    if occupied.binary_search(&parent).is_ok() {
                        return Err(malformed(
                            metadata.page_rows[index].offset,
                            "PDF page parent is not a Pages object",
                        ));
                    }
                    let group = missing.entry(parent).or_insert_with(|| {
                        missing_order.push(parent);
                        MissingGroup::default()
                    });
                    group.count = group.count.checked_add(1).ok_or_else(|| {
                        malformed(
                            metadata.page_rows[index].offset,
                            "CAJ page-tree count overflows",
                        )
                    })?;
                    if group.seen.insert(child) {
                        group.kids.push(child);
                    }
                    break;
                }
                None => {
                    if present_root
                        .replace(child)
                        .is_some_and(|root| root != child)
                    {
                        return Err(malformed(
                            metadata.page_rows[index].offset,
                            "CAJ pages have multiple existing roots",
                        ));
                    }
                    break;
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
        let number = highest.checked_add(1).ok_or_else(|| {
            malformed(
                metadata.body_start,
                "CAJ synthetic page-tree object number overflows",
            )
        })?;
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
