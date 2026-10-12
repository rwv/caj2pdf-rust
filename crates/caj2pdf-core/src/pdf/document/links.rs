// SPDX-License-Identifier: MIT

//! Bounded URI link annotations with self-contained appearance Forms.
//! Nothing is appended to the page's body stream or resource dictionary.

use super::*;

const MAX_LINKS: usize = 8192;
const MAX_URI_BYTES: usize = 4096;
const GAP: f64 = 1.5;
const WEIGHT: f64 = 0.5;
const STEPS: [f64; 6] = [6.75, 2.25, 2.25, 2.25, 2.25, 2.25];

#[derive(Clone, Copy)]
pub(crate) struct LinkAnnotation<'a> {
    /// PDF points, bottom-left origin: left, bottom, right, top.
    pub rect: [f64; 4],
    pub rgb: [u8; 3],
    pub uri: &'a str,
}

impl LinkAnnotation<'_> {
    fn validate(self) -> Result<()> {
        let [l, b, r, t] = self.rect;
        if self
            .rect
            .iter()
            .any(|v| !v.is_finite() || v.abs() > 2.0 * MAX_PAGE_POINTS)
            || l >= r
            || b >= t
            || r - l > 2.0 * MAX_PAGE_POINTS
        {
            return Err(Error::invalid("PDF link rectangle is invalid"));
        }
        if self.uri.is_empty()
            || self.uri.len() > MAX_URI_BYTES
            || !self.uri.is_ascii()
            || self.uri.bytes().any(|b| b <= b' ' || b == 127)
        {
            return Err(Error::invalid("PDF link URI is invalid"));
        }
        Ok(())
    }
}

impl<W: Write, C: Cancellation> PdfDocument<'_, W, C> {
    pub(super) fn write_link_annotations<'u>(
        &mut self,
        links: impl Iterator<Item = LinkAnnotation<'u>> + Clone,
    ) -> Result<Option<ObjectId>> {
        let count = links.clone().take(MAX_LINKS + 1).count();
        if count == 0 {
            return Ok(None);
        }
        if count > MAX_LINKS {
            return Err(Error::limit(
                "PDF page links",
                MAX_LINKS as u64,
                len_u64(count),
            ));
        }
        for link in links.clone() {
            link.validate()?;
        }
        let bytes = len_u64(count * size_of::<ObjectId>());
        self.limits.check_allocation(bytes)?;
        let mut ids = Vec::new();
        reserve_exact(
            &mut ids,
            count,
            self.limits
                .allocation_refused("PDF page link references", bytes),
        )?;
        self.writer.prepare_objects(3 * count + 1)?;
        for link in links {
            let [left, bottom, right, top] = link.rect;
            // Use the same precision as the serialized BBox. Otherwise a
            // decimal-coordinate subtraction can create a zero-width last dash.
            let width = ((right - left) * 1_000_000.0).round() / 1_000_000.0;
            let height = top - bottom + GAP;
            let appearance = self.writer.reserve_object()?;
            let length = self.writer.reserve_object()?;
            let annotation = self.writer.reserve_object()?;
            self.writer.begin_stream(appearance, length, format!(
                "/Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 {width:.6} {height:.6}] /Resources << >>"
            ).as_bytes())?;
            self.writer.write_stream_bytes(
                format!(
                    "q {:.6} {:.6} {:.6} rg\n",
                    f64::from(link.rgb[0]) / 255.0,
                    f64::from(link.rgb[1]) / 255.0,
                    f64::from(link.rgb[2]) / 255.0
                )
                .as_bytes(),
            )?;
            // Explicit filled dashes avoid Wine's PDF dash-array/hairline
            // differences. These are physical dimensions, independent of zoom.
            let mut x = 0.0;
            let mut step = 0;
            while x < width {
                let end = (x + STEPS[step % STEPS.len()]).min(width);
                if step % 2 == 0 {
                    self.writer.write_stream_bytes(
                        format!("{x:.6} 0 {:.6} {WEIGHT} re f\n", end - x).as_bytes(),
                    )?;
                }
                x = end;
                step += 1;
            }
            self.writer.write_stream_bytes(b"Q\n")?;
            self.writer.end_stream()?;
            self.writer.begin_object(annotation)?;
            self.writer.write_bytes(format!(
                "<< /Type /Annot /Subtype /Link /Rect [{left:.6} {:.6} {right:.6} {top:.6}] /Border [0 0 0] /AP << /N {} 0 R >> /A << /S /URI /URI <",
                bottom - GAP, appearance.number()).as_bytes())?;
            // ASCII URI bytes as a PDF hex string: delimiters cannot escape it.
            const HEX: &[u8] = b"0123456789ABCDEF";
            let mut encoded = [0; 256];
            for chunk in link.uri.as_bytes().chunks(128) {
                for (index, byte) in chunk.iter().enumerate() {
                    encoded[2 * index] = HEX[usize::from(byte >> 4)];
                    encoded[2 * index + 1] = HEX[usize::from(byte & 15)];
                }
                self.writer.write_bytes(&encoded[..chunk.len() * 2])?;
            }
            self.writer.write_bytes(b"> >> >>")?;
            self.writer.end_object()?;
            ids.push(annotation);
        }
        let array = self.writer.reserve_object()?;
        self.writer.begin_object(array)?;
        self.writer.write_bytes(b"[")?;
        for id in ids {
            self.writer
                .write_bytes(format!(" {} 0 R", id.number()).as_bytes())?;
        }
        self.writer.write_bytes(b" ]")?;
        self.writer.end_object()?;
        Ok(Some(array))
    }
}
