// SPDX-License-Identifier: MIT

//! Dependency-based blank substitutions for explicitly requested partial output.

use super::converter::replace_object;
use crate::pdf::PdfRef;
use crate::pdf::input::{
    FragmentKind, FragmentScan, PatchedSource, blank_fragment_page, inspect_fragment_object,
};
use crate::{Cancellation, Error, Limits, OmittedPage, PdfErrorKind, RangedSource, Result};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub(super) async fn substitute<S: RangedSource, C: Cancellation>(
    source: &mut S,
    metadata: &super::CajMetadata,
    scan: &mut FragmentScan,
    limits: &Limits,
    cancellation: &C,
) -> Result<(Vec<u8>, Vec<OmittedPage>)> {
    limits.check_allocation(
        (scan.objects.len() as u64)
            .saturating_mul(256)
            .saturating_add(metadata.page_rows.len() as u64 * 128),
    )?;
    let pages: BTreeSet<_> = metadata
        .page_rows
        .iter()
        .map(|row| PdfRef {
            number: row.page_object_id,
            generation: 0,
        })
        .collect();
    let present: BTreeSet<_> = scan.objects.iter().map(|object| object.reference).collect();
    let mut failed = BTreeMap::<PdfRef, u64>::new();
    let mut dependents = BTreeMap::<PdfRef, Vec<PdfRef>>::new();
    let mut edges = 0_u64;
    let mut patched = PatchedSource::new(source, &scan.patches);
    for object in &scan.objects {
        if cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let inspection = match inspect_fragment_object(
            &mut patched,
            object.range,
            object.reference,
            limits,
            cancellation,
            |reference| scan.resolve_length(reference),
        )
        .await
        {
            Ok(inspection) => inspection,
            Err(Error::Pdf {
                offset,
                kind: PdfErrorKind::Malformed,
                ..
            }) => {
                failed.insert(object.reference, offset);
                continue;
            }
            Err(error) => return Err(error),
        };
        let parent = match inspection.kind {
            FragmentKind::Page { parent, .. } => Some(parent),
            FragmentKind::Pages { parent, .. } => parent,
            _ => None,
        };
        for dependency in inspection.references {
            // Page-tree parent links and references to retained (possibly blank)
            // pages are structural, not rendering dependencies.
            if Some(dependency) == parent || pages.contains(&dependency) {
                continue;
            }
            edges += 1;
            limits.check_allocation(
                edges
                    .saturating_mul(64)
                    .saturating_add(scan.objects.len() as u64 * 128),
            )?;
            if !present.contains(&dependency) {
                failed.insert(object.reference, object.range.offset);
            }
            dependents
                .entry(dependency)
                .or_default()
                .push(object.reference);
        }
    }
    for &(reference, offset) in &scan.damaged {
        if let Some(reference) = reference
            && !present.contains(&reference)
        {
            failed.insert(reference, offset);
        }
        if let Some(row) = metadata
            .page_rows
            .iter()
            .rev()
            .find(|row| row.offset <= offset && row.length != 0)
        {
            failed.insert(
                PdfRef {
                    number: row.page_object_id,
                    generation: 0,
                },
                offset,
            );
        }
    }
    let mut queue: VecDeque<_> = failed
        .iter()
        .map(|(&reference, &offset)| (reference, offset))
        .collect();
    while let Some((reference, offset)) = queue.pop_front() {
        if cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        // A blank page still satisfies incoming bookmark/link references.
        if pages.contains(&reference) {
            continue;
        }
        if let Some(owners) = dependents.get(&reference) {
            for &owner in owners {
                if let std::collections::btree_map::Entry::Vacant(entry) = failed.entry(owner) {
                    entry.insert(offset);
                    queue.push_back((owner, offset));
                }
            }
        }
    }
    let mut suffix = Vec::new();
    let mut omitted = Vec::new();
    for (index, row) in metadata.page_rows.iter().enumerate() {
        let reference = PdfRef {
            number: row.page_object_id,
            generation: 0,
        };
        let object = scan
            .objects
            .iter()
            .find(|object| object.reference == reference)
            .copied()
            .ok_or(Error::Caj {
                offset: row.offset,
                record: Some(index as u32 + 1),
                reason: "damaged page has no validated geometry",
            })?;
        if let Some(&offset) = failed.get(&reference) {
            let replacement =
                blank_fragment_page(&mut patched, object, limits, cancellation).await?;
            let replacement = (reference, replacement.as_slice());
            let base = patched.size();
            replace_object(&mut scan.objects, &mut suffix, base, replacement, limits)?;
            let refused =
                limits.allocation_refused("omitted page report", (omitted.len() as u64 + 1) * 16);
            crate::fallible::reserve(&mut omitted, 1, refused)?;
            omitted.push(OmittedPage {
                page_index: index as u32,
                offset,
            });
        }
    }
    scan.objects.retain(|object| {
        pages.contains(&object.reference) || !failed.contains_key(&object.reference)
    });
    Ok((suffix, omitted))
}

#[cfg(test)]
mod tests;
