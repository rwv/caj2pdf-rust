// SPDX-License-Identifier: MIT

//! Source-derived HN-A/C8 text and layout metadata, one page at a time.
//! This development tool emits facts and predictions, never source bytes or
//! PDFs. It does not enable production HN/C8 conversion.

use caj2pdf_core::{
    Limits, NeverCancel, RangedSource,
    hnc8::{
        Budget, Hnc8Reader, JpegBudget, TextBudget, Type0PdfOptions, empirical_image_transform,
        empirical_page_from_pixels, empirical_page_from_type0, read_text_coordinates,
        read_type2_jpeg_info,
    },
    jbig1::{Type0Budget, read_type0_info},
    native::SeekableSource,
};
use sha2::{Digest, Sha256};
use std::{
    env,
    error::Error,
    fmt::Write as _,
    fs::File,
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};

type ToolResult<T> = Result<T, Box<dyn Error>>;
const IO_CHUNK: usize = 4096;

fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("native file adapter unexpectedly yielded"),
    }
}

struct Counted<S> {
    source: S,
    read_bytes: u64,
    max_request: usize,
}

impl<S: RangedSource> RangedSource for Counted<S> {
    fn size(&self) -> u64 {
        self.source.size()
    }

    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        self.max_request = self.max_request.max(destination.len());
        let count = self.source.read_at(offset, destination).await?;
        self.read_bytes = self.read_bytes.saturating_add(count as u64);
        Ok(count)
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut result, "{byte:02x}").expect("writing to a String");
    }
    result
}

fn hash_image<S: RangedSource>(
    source: &mut S,
    offset: u64,
    length: u64,
    limits: &Limits,
) -> ToolResult<String> {
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; IO_CHUNK];
    let mut done = 0;
    while done < length {
        let count = (length - done).min(IO_CHUNK as u64) as usize;
        ready(caj2pdf_core::read_exact_at(
            source,
            offset
                .checked_add(done)
                .ok_or("image hash offset overflows")?,
            &mut buffer[..count],
            limits,
            &NeverCancel,
        ))?;
        hash.update(&buffer[..count]);
        done += count as u64;
    }
    Ok(hex(&hash.finalize()))
}

fn run() -> ToolResult<()> {
    let mut args = env::args_os().skip(1);
    let path = args
        .next()
        .ok_or("usage: hnc8_text_placement SOURCE_PATH")?;
    if args.next().is_some() {
        return Err("usage: hnc8_text_placement SOURCE_PATH".into());
    }
    let limits = Limits {
        io_chunk_bytes: IO_CHUNK,
        max_input_bytes: 1024 * 1024 * 1024,
        ..Limits::default()
    };
    let mut source = Counted {
        source: SeekableSource::new(File::open(path)?)?,
        read_bytes: 0,
        max_request: 0,
    };
    let mut reader = ready(Hnc8Reader::open(
        &mut source,
        &limits,
        &NeverCancel,
        Budget::default(),
    ))?;
    let header = reader.header();
    println!("H\t{}\t{}", header.variant.as_str(), header.page_count);
    let mut max_text_working = 0;
    let mut max_text_owned = 0;
    while let Some(page) = ready(reader.next_page())? {
        let text = ready(read_text_coordinates(
            reader.source_mut(),
            header,
            page,
            &limits,
            &NeverCancel,
            TextBudget::default(),
        ))?;
        max_text_working = max_text_working.max(text.working_memory_bytes);
        max_text_owned = max_text_owned.max(text.owned_buffer_bytes);
        println!(
            "P\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            page.page_number,
            page.image_count,
            page.text.offset,
            page.text.length,
            text.decoded_length,
            text.record_count,
            hex(&text.encoded_sha256),
            hex(&text.decoded_sha256),
            text.max_source_request_bytes,
            text.max_decoder_output_chunk_bytes,
            text.owned_buffer_bytes,
            text.working_memory_bytes,
        );
        let mut geometry = None;
        while let Some(image) = ready(reader.next_image())? {
            let (visible_width, display_width, height) = match image.record_type {
                0 => {
                    let info = ready(read_type0_info(
                        reader.source_mut(),
                        image.type0_span().ok_or("type-0 span missing")?,
                        &limits,
                        &NeverCancel,
                        Type0PdfOptions::default().arithmetic,
                        Type0Budget::default(),
                    ))?;
                    let display_width = u32::try_from(
                        (info.dib_stride as u64)
                            .checked_mul(8)
                            .ok_or("stride overflows")?,
                    )?;
                    if geometry.is_none() {
                        geometry = Some(empirical_page_from_type0(info, [0.0, 0.0])?);
                    }
                    (info.width, display_width, info.height)
                }
                2 => {
                    let info = ready(read_type2_jpeg_info(
                        reader.source_mut(),
                        image,
                        &limits,
                        &NeverCancel,
                        JpegBudget::default(),
                    ))?;
                    let width = u32::from(info.width);
                    let height = u32::from(info.height);
                    if geometry.is_none() {
                        geometry = Some(empirical_page_from_pixels(width, height, [0.0, 0.0])?);
                    }
                    (width, width, height)
                }
                _ => {
                    return Err(
                        "layout diagnostic supports only observed type-0/type-2 images".into(),
                    );
                }
            };
            let geometry = geometry.ok_or("page has no first-image geometry")?;
            if image.image_number == 1 {
                println!(
                    "B\t{}\t0\t0\t{:.12}\t{:.12}",
                    page.page_number, geometry.size.width_points, geometry.size.height_points,
                );
            }
            let coordinate = text
                .coordinates
                .get(image.image_number as usize - 1)
                .copied()
                .ok_or("missing image coordinate")?;
            let matrix = empirical_image_transform(geometry, display_width, height, coordinate)?;
            let digest = hash_image(
                reader.source_mut(),
                image.payload.offset,
                image.payload.length,
                &limits,
            )?;
            println!(
                "I\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.12}\t{:.12}\t{:.12}\t{:.12}\t{:.12}\t{:.12}",
                image.page_number,
                image.image_number,
                image.record_type,
                image.descriptor_offset,
                image.payload.offset,
                image.payload.length,
                visible_width,
                display_width,
                height,
                coordinate.x,
                coordinate.y,
                digest,
                matrix[0],
                matrix[1],
                matrix[2],
                matrix[3],
                matrix[4],
                matrix[5],
            );
        }
        if geometry.is_none() {
            return Err("layout diagnostic requires at least one image per page".into());
        }
    }
    println!(
        "R\t{}\t{}\t{}\t{}\t0",
        source.read_bytes, source.max_request, max_text_owned, max_text_working,
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
