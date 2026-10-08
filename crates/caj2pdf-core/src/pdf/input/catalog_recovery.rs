// SPDX-License-Identifier: MIT

//! A measured retained catalog whose optional targets have no complete value.
//! Keep its available page-label tree; rebuild only the disconnected page tree.

use super::fragment_scan::ScannedObject;
use super::parser::{
    ObjectHead, ObjectTail, Syntax, exact_name, exact_reference, exact_unsigned, parse_object_head,
};
use super::{FragmentKind, Reader};
use crate::pdf::{FragmentObject, PdfRef};
use crate::{Cancellation, RangedSource, Result};

const MAX_HEADER: u64 = 256;

pub(super) struct Recovery {
    pub catalog: PdfRef,
    pub root: PdfRef,
    pub labels: PdfRef,
    pub metadata: PdfRef,
}

fn object(objects: &[ScannedObject], reference: PdfRef) -> Option<&ScannedObject> {
    objects.iter().find(|s| s.object.reference == reference)
}

fn small_head<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    scanned: &ScannedObject,
) -> Result<Option<ObjectHead>> {
    let o = scanned.object;
    if o.range.length > MAX_HEADER || o.reference.generation != 0 {
        return Ok(None);
    }
    let bytes = reader.bytes(
        o.range.offset - reader.range.offset,
        o.range.length as usize,
    )?;
    Ok(parse_object_head(bytes).ok().filter(|h| {
        h.reference == o.reference
            && matches!(h.tail, ObjectTail::EndObject { end } if end as u64 == o.range.length)
            && scanned
                .inspection
                .as_ref()
                .is_ok_and(|i| h.references == i.references)
    }))
}

fn label_reference(bytes: &[u8]) -> Option<PdfRef> {
    let mut s = Syntax::new(bytes);
    s.skip_space();
    if bytes.get(s.pos) != Some(&b'[') {
        return None;
    }
    s.pos += 1;
    if s.unsigned().ok()? != 0 {
        return None;
    }
    let reference = s.reference().ok()?;
    s.skip_space();
    if bytes.get(s.pos) != Some(&b']') {
        return None;
    }
    s.pos += 1;
    s.at_end().then_some(reference)
}

/// No stream bytes occur in this declaration. The framing CRLF and the next
/// fully indexed object fix its end, independently of a payload byte search.
fn metadata_prefix<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    objects: &[ScannedObject],
    prefix: FragmentObject,
) -> Result<bool> {
    let length = prefix.range.length + 2;
    if length > MAX_HEADER || prefix.reference.generation != 0 {
        return Ok(false);
    }
    let bytes = reader.bytes(prefix.range.offset - reader.range.offset, length as usize)?;
    let opener = format!("{} 0 obj<</Length ", prefix.reference.number);
    let Some(digits) = bytes
        .strip_prefix(opener.as_bytes())
        .and_then(|b| b.strip_suffix(b"/Type/Metada\r\n"))
    else {
        return Ok(false);
    };
    let Some(n) = exact_unsigned(digits).filter(|n| *n > 0) else {
        return Ok(false);
    };
    let framed = digits == n.to_string().as_bytes()
        && objects
            .iter()
            .any(|s| s.object.range.offset == prefix.range.offset + length);
    if framed && reader.bytes(prefix.range.offset - reader.range.offset, length as usize)? != bytes
    {
        return Err(reader.malformed(
            prefix.range.offset - reader.range.offset,
            Some(prefix.reference),
            "retained catalog metadata prefix changed while reading",
        ));
    }
    Ok(framed)
}

/// Only one five-entry catalog, one disconnected three-entry Pages root,
/// absent optional targets, and the measured two-object decimal label tree.
/// Every leaf supplies all inherited properties. The caller still proves a
/// complete graph without opaque streams, all interruptions, and CAJ order.
pub(super) fn inspect<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    objects: &[ScannedObject],
    prefixes: &[(FragmentObject, Option<FragmentObject>)],
    orphans: &[FragmentObject],
) -> Result<Option<Recovery>> {
    if objects.iter().any(|s| s.inspection.is_err()) {
        return Ok(None);
    }
    let mut catalogs = objects
        .iter()
        .filter(|s| matches!(s.inspection.as_ref().unwrap().kind, FragmentKind::Catalog));
    let Some(catalog) = catalogs.next() else {
        return Ok(None);
    };
    if catalogs.next().is_some() {
        return Ok(None);
    }
    let Some(head) = small_head(reader, catalog)? else {
        return Ok(None);
    };
    let Some(d) = head.dictionary.as_ref() else {
        return Ok(None);
    };
    if d.entries.len() != 5 || d.value(b"Type").and_then(exact_name).as_deref() != Some(b"Catalog")
    {
        return Ok(None);
    }
    let (Some(root), Some(labels), Some(form), Some(metadata)) = (
        d.value(b"Pages").and_then(exact_reference),
        d.value(b"PageLabels").and_then(exact_reference),
        d.value(b"AcroForm").and_then(exact_reference),
        d.value(b"Metadata").and_then(exact_reference),
    ) else {
        return Ok(None);
    };
    let ids = [catalog.object.reference, root, labels, form, metadata];
    if ids.iter().any(|r| r.number == 0 || r.generation != 0)
        || ids.iter().enumerate().any(|(i, r)| ids[..i].contains(r))
        || object(objects, form).is_some()
        || object(objects, metadata).is_some()
        || prefixes.iter().any(|(p, _)| {
            p.reference == form || p.reference == root || p.reference == catalog.object.reference
        })
        || orphans.iter().any(|p| ids.contains(&p.reference))
    {
        return Ok(None);
    }
    let mut interrupted = prefixes.iter().filter(|(p, _)| p.reference == metadata);
    let Some((prefix, _)) = interrupted.next() else {
        return Ok(None);
    };
    if interrupted.next().is_some() || !metadata_prefix(reader, objects, *prefix)? {
        return Ok(None);
    }
    let Some(root_object) = object(objects, root) else {
        return Ok(None);
    };
    let Some(root_head) = small_head(reader, root_object)? else {
        return Ok(None);
    };
    let Some(rd) = root_head.dictionary.as_ref() else {
        return Ok(None);
    };
    let FragmentKind::Pages {
        parent: None,
        count,
        kids,
        ..
    } = &root_object.inspection.as_ref().unwrap().kind
    else {
        return Ok(None);
    };
    if rd.entries.len() != 3
        || rd.value(b"Type").and_then(exact_name).as_deref() != Some(b"Pages")
        || rd.value(b"Count").and_then(exact_unsigned) != Some(u64::from(*count))
        || rd
            .value(b"Kids")
            .and_then(|v| super::parser::reference_array(v, 64))
            .as_ref()
            != Some(kids)
        || kids.is_empty()
        || kids.len() > 64
        || kids.iter().enumerate().any(|(i, r)| {
            r.generation != 0
                || r.number == 0
                || kids[..i].contains(r)
                || object(objects, *r).is_some()
        })
    {
        return Ok(None);
    }
    let Some(label_tree) = object(objects, labels) else {
        return Ok(None);
    };
    let Some(lh) = small_head(reader, label_tree)? else {
        return Ok(None);
    };
    let Some(ld) = lh.dictionary.as_ref().filter(|d| d.entries.len() == 1) else {
        return Ok(None);
    };
    let Some(label) = ld.value(b"Nums").and_then(label_reference) else {
        return Ok(None);
    };
    let Some(label_object) = object(objects, label) else {
        return Ok(None);
    };
    let Some(lv) = small_head(reader, label_object)? else {
        return Ok(None);
    };
    if !lv.dictionary.as_ref().is_some_and(|d| {
        d.entries.len() == 1 && d.value(b"S").and_then(exact_name).as_deref() == Some(b"D")
    }) {
        return Ok(None);
    }
    let mut pages = 0;
    for scanned in objects {
        if reader.cancellation.is_cancelled() {
            return Err(crate::ErrorKind::Cancelled.into());
        }
        let i = scanned.inspection.as_ref().unwrap();
        if i.references.contains(&catalog.object.reference)
            || (scanned.object.reference != catalog.object.reference
                && i.references
                    .iter()
                    .any(|r| [root, form, metadata].contains(r)))
        {
            return Ok(None);
        }
        if let FragmentKind::Page { parent, .. } = i.kind {
            if !super::orphan::independent_leaf(reader, scanned, parent)? {
                return Ok(None);
            }
            let h = reader.load_head(
                scanned.object.range.offset - reader.range.offset,
                Some(scanned.object.reference),
            )?;
            if h.dictionary
                .as_ref()
                .is_none_or(|d| d.value(b"Annots").is_some())
            {
                return Ok(None);
            }
            pages += 1;
        }
    }
    if pages != *count {
        return Ok(None);
    }
    for (scanned, h) in [(catalog, &head), (root_object, &root_head)] {
        if reader.bytes(
            scanned.object.range.offset - reader.range.offset,
            h.bytes.len(),
        )? != h.bytes
        {
            return Err(reader.malformed(
                scanned.object.range.offset - reader.range.offset,
                Some(scanned.object.reference),
                "retained catalog or root changed while reading",
            ));
        }
    }
    Ok(Some(Recovery {
        catalog: catalog.object.reference,
        root,
        labels,
        metadata,
    }))
}
