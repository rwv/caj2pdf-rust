// SPDX-License-Identifier: MIT

//! CAJ to PDF conversion using bounded PDF fragment reconstruction.

use super::{CajMetadata, parse_metadata};
use crate::fallible::{reserve, reserve_exact};
use crate::pdf::input::stream_substitution::Plan as SubstitutionPlan;
use crate::pdf::input::{
    FragmentCandidate, FragmentKind, FragmentScan, LinkRepairCandidate, LinkRepairKind,
    LinkRepairTarget, PatchedSource, collect_fragment_candidates, inspect_generated_object,
    inspect_link_destination_candidate, inspect_link_missing_target_candidate,
    scan_damaged_fragment, scan_fragment_with_candidates, substitute_damaged_pages,
    validate_source_path_repair,
};
use crate::pdf::{
    FragmentObject, InspectedObject, InspectedPlan, PdfRange, PdfRef, append_replacement,
    reconstruct_inspected,
};
use crate::{
    Cancellation, ConversionOptions, ConversionReport, CountingSource, Error, ErrorKind, Limits,
    RangedSource, Result,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Write;

const OVERREAD: &str = "CAJ source reported more bytes than requested";

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
            .ok_or(Error::invalid("CAJ synthetic page-tree range overflows"))?;
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

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        if offset < self.base {
            let available = (self.base - offset).min(destination.len() as u64) as usize;
            self.source.read_at(offset, &mut destination[..available])
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
        } else {
            // Match CAJViewer's observed Letter fallback for unavailable
            // inherited page boxes. Explicit descendant boxes still take precedence.
            body.write_str("/MediaBox [0 0 612 792] ")?;
        }
        write!(body, "/Count {} /Kids [", node.count)?;
        for child in node.kids {
            write!(body, "{} 0 R ", child.number)?;
        }
        body.write_str("] >>\nendobj\n")
    };
    format().map_err(|_| Error::invalid("CAJ synthetic page tree exceeds reserved bound"))
}

fn malformed(offset: u64, reason: &'static str) -> Error {
    Error::malformed(offset, reason).in_caj(None)
}

fn missing_reference(objects: &[InspectedObject], owner: PdfRef) -> Error {
    Error::pdf(
        ErrorKind::Malformed,
        objects
            .iter()
            .find(|inspected| inspected.object.reference == owner)
            .map_or(0, |inspected| inspected.object.range.offset),
        Some((owner.number, owner.generation)),
        "indirect reference targets a missing object",
    )
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
        let node = nodes.get_mut(&current).ok_or(Error::invalid(
            "resolved page-tree path changed during caching",
        ))?;
        if node.resolved_root.is_some() {
            break;
        }
        let parent = node.parent;
        node.resolved_root = Some(resolved);
        if current == terminal {
            break;
        }
        current = parent.ok_or(Error::invalid("resolved page-tree path lost its parent"))?;
    }
    Ok(resolved)
}

/// Replace an object's span with a generated body appended to `suffix`.
fn replace_object(
    objects: &mut [InspectedObject],
    suffix: &mut Vec<u8>,
    base: u64,
    (reference, replacement): (PdfRef, &[u8]),
    limits: &Limits,
) -> Result<()> {
    let next_size = suffix
        .len()
        .checked_add(replacement.len())
        .ok_or(Error::invalid("CAJ repair suffix size overflows"))?;
    limits.check_allocation(next_size as u64)?;
    let refused = limits.allocation_refused("CAJ link repair allocation", next_size as u64);
    reserve(suffix, replacement.len(), refused)?;
    let old = objects
        .iter_mut()
        .find(|inspected| inspected.object.reference == reference)
        .ok_or(Error::invalid(
            "CAJ link repair object is absent from fragment plan",
        ))?;
    old.object.range = append_replacement(suffix, base, replacement)?;
    old.inspection = inspect_generated_object(replacement, limits)?;
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
        .ok_or(Error::invalid("CAJ retained repair count overflows"))?;
    let next_bytes = retained_bytes
        .checked_sub(
            repaired
                .get(&candidate.object)
                .map_or(0, |old| old.replacement.len()),
        )
        .and_then(|size| size.checked_add(candidate.replacement.len()))
        .ok_or(Error::invalid("CAJ retained repair size overflows"))?;
    let retained = next_bytes
        .checked_add(suffix_len)
        .and_then(|size| {
            count
                .checked_mul(128)
                .and_then(|overhead| size.checked_add(overhead))
        })
        .ok_or(Error::invalid("CAJ retained repair size overflows"))?;
    if retained as u64 > limits.max_allocation_bytes {
        return Err(Error::limit(
            "retained link repairs",
            limits.max_allocation_bytes,
            retained as u64,
        )
        .at(offset)
        .in_caj(None));
    }
    repaired.insert(candidate.object, candidate);
    *retained_bytes = next_bytes;
    Ok(())
}

fn push_synthetic(
    suffix: &mut Vec<u8>,
    objects: &mut Vec<InspectedObject>,
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
        .ok_or(Error::invalid("CAJ synthetic page tree estimate overflows"))?;
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
    let inspection = inspect_generated_object(&suffix[start..], limits)?;
    objects.push(InspectedObject {
        object: FragmentObject {
            reference: PdfRef {
                number,
                generation: 0,
            },
            range: PdfRange {
                offset: base
                    .checked_add(start as u64)
                    .ok_or(Error::invalid("CAJ synthetic page tree offset overflows"))?,
                length: body_len as u64,
            },
        },
        inspection,
    });
    Ok(())
}

/// Retry a malformed fragment using only independently parsed page-table
/// spans, from last to first so later anchors can prove earlier prefixes.
/// Each row is scanned once. No payload is searched for headers, and the full
/// scan must confirm every candidate used.
fn scan_caj_objects<S: RangedSource, C: Cancellation>(
    source: &mut S,
    metadata: &super::CajMetadata,
    limits: &Limits,
    cancellation: &C,
    allow_damaged: bool,
) -> Result<FragmentScan> {
    let original_error = match scan_fragment_with_candidates(
        source,
        metadata.body_start,
        metadata.body_end_hint,
        limits,
        cancellation,
        &mut [],
    ) {
        Ok(scan) => return Ok(scan),
        Err(error) if error.is_malformed_pdf() => error,
        Err(error) => return Err(error),
    };
    let mut candidates = Vec::new();
    for row in metadata
        .page_rows
        .iter()
        .skip(1)
        .rev()
        .filter(|row| row.length != 0)
    {
        let mut objects = match collect_fragment_candidates(
            source,
            row.offset,
            row.offset + row.length,
            limits,
            cancellation,
            &mut candidates,
        ) {
            Ok(objects) => objects,
            Err(error) if error.is_pdf_problem() => continue,
            Err(error) => return Err(error),
        };
        if !objects.first().is_some_and(|object| {
            object.reference.number == row.page_object_id && object.reference.generation == 0
        }) {
            continue;
        }
        // Rows are disjoint. A last object's tail can cross into the next
        // row, but no object starting there belongs in this row's index.
        objects.retain(|object| object.range.offset < row.offset + row.length);
        let bytes = ((candidates.len() + objects.len()) as u64)
            .saturating_mul(size_of::<FragmentCandidate>() as u64);
        limits.check_allocation(bytes)?;
        let refused = limits.allocation_refused("CAJ recovery candidate index", bytes);
        reserve(&mut candidates, objects.len(), refused)?;
        candidates.extend(objects.into_iter().map(|object| FragmentCandidate {
            object,
            used: false,
        }));
    }
    if candidates.is_empty() && !allow_damaged {
        return Err(original_error);
    }
    candidates.sort_unstable_by_key(|candidate| candidate.object.range.offset);
    candidates.dedup_by_key(|candidate| candidate.object.range.offset);
    let result = scan_fragment_with_candidates(
        source,
        metadata.body_start,
        metadata.body_end_hint,
        limits,
        cancellation,
        &mut candidates,
    );
    match result {
        Err(error) if allow_damaged && error.is_malformed_pdf() => scan_damaged_fragment(
            source,
            &metadata.page_rows,
            metadata.body_end_hint,
            limits,
            cancellation,
            &mut candidates,
        ),
        other => other,
    }
}

/// Convert a CAJ source to a forward-only PDF sink. The source must support
/// stable positioned reads. Page payloads are never materialized in a `Vec`;
/// only page/outline metadata, object positions, and small missing page-tree
/// dictionaries are retained. All PDF object validation precedes output.
/// Each ordinary source object is parsed once. A checksum-confirmed stream
/// substitution with independent page-table evidence requires one rescan of
/// its sparse repaired view before page-tree reconstruction and link repair.
pub fn convert_caj<S: RangedSource, W: Write, C: Cancellation>(
    source: &mut S,
    sink: &mut W,
    options: &ConversionOptions<'_>,
    limits: &Limits,
    cancellation: &C,
) -> Result<ConversionReport> {
    let mut input_bytes_read = 0;
    let mut counted =
        CountingSource::new(source, &mut input_bytes_read).rejecting_overread(OVERREAD);
    let metadata = parse_metadata(&mut counted, limits, cancellation)?;
    let mut scan = scan_caj_objects(
        &mut counted,
        &metadata,
        limits,
        cancellation,
        options.allow_damaged,
    )?;
    let mut report = if let Some(plan) = SubstitutionPlan::from_scan(&mut scan, &metadata, limits)?
    {
        // The plan owns only selected positions and hashes. Release the first
        // indexes before building the ordinary graph over its sparse view.
        drop(scan);
        drop(metadata);
        plan.verify(&mut counted, limits, cancellation)?;
        let result = {
            let mut recovered = plan.source(&mut counted, limits, cancellation)?;
            let result = (|| {
                let metadata = parse_metadata(&mut recovered, limits, cancellation)?;
                let scan = scan_caj_objects(
                    &mut recovered,
                    &metadata,
                    limits,
                    cancellation,
                    options.allow_damaged,
                )?;
                if !scan.substitutions.is_empty() {
                    return Err(Error::invalid(
                        "CAJ substitution did not restore stream framing",
                    ));
                }
                convert_scanned(
                    &mut recovered,
                    sink,
                    metadata,
                    scan,
                    options,
                    limits,
                    cancellation,
                )
            })();
            result.map_err(|error| recovered.locate(error))
        };
        let report = result?;
        plan.verify(&mut counted, limits, cancellation)?;
        report
    } else {
        convert_scanned(
            &mut counted,
            sink,
            metadata,
            scan,
            options,
            limits,
            cancellation,
        )?
    };
    report.input_bytes_read = input_bytes_read;
    Ok(report)
}

fn convert_scanned<S: RangedSource, W: Write, C: Cancellation>(
    source: &mut S,
    sink: &mut W,
    metadata: CajMetadata,
    mut scan: FragmentScan,
    options: &ConversionOptions<'_>,
    limits: &Limits,
    cancellation: &C,
) -> Result<ConversionReport> {
    let mut source_paths = std::mem::take(&mut scan.source_paths);
    let mut sorted_page_ids = Vec::new();
    if !source_paths.is_empty() {
        // The larger page-row index has already passed the allocation limit.
        let refused = limits.allocation_refused(
            "CAJ source path page index",
            (metadata.page_rows.len() * size_of::<u32>()) as u64,
        );
        reserve_exact(&mut sorted_page_ids, metadata.page_rows.len(), refused)?;
        sorted_page_ids.extend(metadata.page_rows.iter().map(|page| page.page_object_id));
        sorted_page_ids.sort_unstable();
    }
    for repair in &source_paths {
        if validate_source_path_repair(
            source,
            repair,
            &scan.objects,
            &sorted_page_ids,
            limits,
            cancellation,
        )? {
            continue;
        }
        if !options.allow_damaged {
            return Err(Error::pdf(
                ErrorKind::Malformed,
                repair.object.range.offset,
                Some((
                    repair.object.reference.number,
                    repair.object.reference.generation,
                )),
                "malformed source path is not exclusively retained Page QITE metadata",
            ));
        }
        let refused = limits.allocation_refused(
            "CAJ damaged metadata index",
            ((scan.damaged.len() + 1) * size_of::<(Option<PdfRef>, u64)>()) as u64,
        );
        reserve(&mut scan.damaged, 1, refused)?;
        scan.damaged
            .push((Some(repair.object.reference), repair.object.range.offset));
        scan.objects
            .retain(|object| object.object.reference != repair.object.reference);
    }
    source_paths.retain(|repair| {
        scan.objects
            .iter()
            .any(|object| object.object == repair.object)
    });
    let (damaged_suffix, omitted_pages) = if options.allow_damaged && !scan.damaged.is_empty() {
        substitute_damaged_pages(source, &metadata, &mut scan, limits, cancellation)?
    } else {
        (Vec::new(), Vec::new())
    };
    let mut working = ExtendedSource::new(source, &damaged_suffix)?;
    // Report the first structural inspection error in source order.
    let mut objects = Vec::new();
    let refused = limits.allocation_refused(
        "CAJ object index allocation",
        (scan.objects.len() as u64).saturating_mul(size_of::<InspectedObject>() as u64),
    );
    reserve_exact(&mut objects, scan.objects.len(), refused)?;
    for scanned in std::mem::take(&mut scan.objects) {
        objects.push(InspectedObject {
            object: scanned.object,
            inspection: scanned.inspection?,
        });
    }
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

    // Classify page-tree nodes from the scan's inspections. The source page
    // table supplies page order but not the missing /Pages nodes.
    let mut nodes = BTreeMap::<PdfRef, TreeNode>::new();
    let occupied_bytes = objects
        .len()
        .checked_mul(std::mem::size_of::<PdfRef>())
        .ok_or(Error::invalid("CAJ object reference index overflows"))?;
    limits.check_allocation(occupied_bytes as u64)?;
    let mut occupied = Vec::<PdfRef>::new();
    let refused = limits.allocation_refused(
        "CAJ object reference index allocation",
        occupied_bytes as u64,
    );
    reserve_exact(&mut occupied, objects.len(), refused)?;
    occupied.extend(objects.iter().map(|inspected| inspected.object.reference));
    occupied.sort_unstable();
    let mut highest_referenced_object = occupied
        .iter()
        .map(|reference| reference.number)
        .max()
        .unwrap_or(0);
    let mut missing_references = Vec::<MissingReference>::new();
    for InspectedObject { object, inspection } in &objects {
        highest_referenced_object = highest_referenced_object.max(inspection.max_referenced_object);
        let page_parent = inspection.page_parent();
        let parent_occurrences = inspection
            .references
            .iter()
            .filter(|reference| Some(**reference) == page_parent)
            .count();
        for missing in inspection
            .references
            .iter()
            .filter(|r| occupied.binary_search(r).is_err())
        {
            let attempted = missing_references
                .len()
                .checked_add(1)
                .and_then(|count| count.checked_mul(std::mem::size_of::<MissingReference>()))
                .ok_or(Error::invalid("CAJ missing reference index overflows"))?;
            let refused = Error::limit(
                "missing PDF references",
                limits.max_allocation_bytes,
                attempted as u64,
            )
            .at(object.range.offset)
            .in_caj(None);
            if attempted as u64 > limits.max_allocation_bytes {
                return Err(refused);
            }
            reserve(&mut missing_references, 1, refused)?;
            missing_references.push(MissingReference {
                owner: object.reference,
                target: *missing,
                page_parent_only: Some(*missing) == page_parent && parent_occurrences == 1,
            });
        }
        if matches!(
            inspection.kind,
            FragmentKind::Page { .. } | FragmentKind::Pages { .. }
        ) {
            let previous = nodes.insert(
                object.reference,
                TreeNode {
                    parent: page_parent,
                    resolved_root: None,
                },
            );
            debug_assert!(previous.is_none(), "fragment scanner deduplicates objects");
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

    let base = working.size();
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
    if objects
        .iter()
        .all(|inspected| inspected.object.reference != root)
    {
        return Err(malformed(
            metadata.body_start,
            "CAJ page-tree root is missing",
        ));
    }

    // A few observed fragments contain link annotations aimed at pages that
    // were omitted from the CAJ page table, or one absent optional appearance
    // while their destination page remains live. Remove only the proven /Dest
    // or /AP pair. An indirect destination array is nullified after every object
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
        let mut patched = PatchedSource::new(&mut working, &scan.patches);
        let mut first = 0usize;
        while first < missing_references.len() {
            let owner = missing_references[first].owner;
            let failure = || missing_reference(&objects, owner);
            let mut last = first + 1;
            while last < missing_references.len() && missing_references[last].owner == owner {
                last += 1;
            }
            let targets = &missing_references[first..last];
            let fragment = objects[..source_object_count]
                .iter()
                .find(|inspected| inspected.object.reference == owner)
                .map(|inspected| inspected.object)
                .ok_or_else(failure)?;
            if targets.len() != 1 {
                return Err(failure());
            }
            let candidate = inspect_link_missing_target_candidate(
                &mut patched,
                fragment,
                targets[0].target,
                limits,
                cancellation,
            )?
            .ok_or_else(failure)?;
            let eligible = match candidate.target {
                LinkRepairTarget::DirectPage(target) => {
                    targets[0].target == target && !page_refs.contains(&target)
                }
                LinkRepairTarget::AbsentAppearance { missing, page } => {
                    targets[0].target == missing && page_refs.contains(&page)
                }
                LinkRepairTarget::IndirectArray(_) => false,
            };
            if !eligible {
                return Err(failure());
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
            for InspectedObject {
                object: fragment,
                inspection,
            } in &objects[..source_object_count]
            {
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
                )?
                .ok_or_else(|| missing_reference(&objects, fragment.reference))?;
                if candidate.kind != LinkRepairKind::Link
                    || candidate.target != LinkRepairTarget::IndirectArray(scalar)
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
            let replacement = (candidate.object, candidate.replacement.as_slice());
            replace_object(&mut objects, &mut suffix, base, replacement, limits)?;
            candidate.replacement = Vec::new();
        }
    }

    for repair in source_paths {
        replace_object(
            &mut objects,
            &mut suffix,
            base,
            (repair.object.reference, &repair.replacement),
            limits,
        )?;
    }

    let bookmarks = if options.include_bookmarks {
        metadata.bookmarks.as_slice()
    } else {
        &[]
    };
    let mut patched = PatchedSource::new(&mut working, &scan.patches);
    let mut extended = ExtendedSource::new(&mut patched, &suffix)?;
    let plan = InspectedPlan {
        objects,
        pages: &page_refs,
        pages_root: root,
    };
    let mut report =
        reconstruct_inspected(&mut extended, sink, plan, bookmarks, limits, cancellation)?;
    report.omitted_pages = omitted_pages;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Context;
    use crate::native::SeekableSource;
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
        assert_eq!(objects[0].object.range.offset, 106);
        assert_eq!(objects[0].object.range.length, body.len() as u64);

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
            Err(Error {
                kind: ErrorKind::LimitExceeded { .. },
                ..
            })
        ));
        assert_eq!(too_small, b"prefix");
        assert!(rejected_objects.is_empty());
    }

    #[test]
    fn a_repeated_link_repair_is_charged_in_place_of_the_first() {
        let object = PdfRef {
            number: 9,
            generation: 0,
        };
        let candidate = |length| LinkRepairCandidate {
            object,
            kind: LinkRepairKind::Link,
            target: LinkRepairTarget::DirectPage(PdfRef {
                number: 3,
                generation: 0,
            }),
            retains_destination_reference: false,
            replacement: vec![b' '; length],
        };
        // One retained repair costs its bytes plus 128 bytes of overhead.
        let limits = Limits {
            io_chunk_bytes: 1,
            max_allocation_bytes: 128 + 100,
            ..Limits::default()
        };
        let mut repaired = BTreeMap::new();
        let mut retained = 0;
        retain_repair(&mut repaired, &mut retained, candidate(100), 0, &limits, 7).unwrap();
        assert_eq!(retained, 100);
        // Charged on top of the first, the second would exceed the limit.
        retain_repair(&mut repaired, &mut retained, candidate(60), 0, &limits, 7).unwrap();
        assert_eq!(retained, 60);
        assert_eq!(repaired.len(), 1);
        assert_eq!(repaired[&object].replacement.len(), 60);
        assert!(matches!(
            retain_repair(&mut repaired, &mut retained, candidate(101), 0, &limits, 7),
            Err(Error {
                kind: ErrorKind::LimitExceeded {
                    resource: "retained link repairs",
                    attempted: 229,
                    ..
                },
                offset: Some(7),
                context: Context::Caj { .. },
                ..
            })
        ));
        assert_eq!(retained, 60);
    }

    #[test]
    fn extended_source_splits_reads_at_the_suffix_and_ends_cleanly() {
        let mut base = SeekableSource::new(Cursor::new(b"abc")).unwrap();
        let suffix = b"XYZ";
        let mut extended = ExtendedSource::new(&mut base, suffix).unwrap();
        assert_eq!(extended.size(), 6);

        let mut buffer = [0u8; 8];
        // A read crossing the boundary stops at the immutable source end.
        assert_eq!(extended.read_at(1, &mut buffer).unwrap(), 2);
        assert_eq!(&buffer[..2], b"bc");
        assert_eq!(extended.read_at(4, &mut buffer).unwrap(), 2);
        assert_eq!(&buffer[..2], b"YZ");
        assert_eq!(extended.read_at(6, &mut buffer).unwrap(), 0);
        assert_eq!(extended.read_at(u64::MAX, &mut buffer).unwrap(), 0);
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
                Err(Error {
                    kind: ErrorKind::Malformed,
                    reason: "CAJ synthetic page tree exceeds reserved bound",
                    ..
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
