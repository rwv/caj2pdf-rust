// SPDX-License-Identifier: MIT

use caj2pdf_core::{
    Cancellation, Error, Limits, NeverCancel, Result, SequentialSink, pdf::PdfWriter,
};
use std::{
    cell::Cell,
    future::Future,
    io,
    pin::pin,
    rc::Rc,
    task::{Context, Poll, Waker},
};

fn run<F: Future>(future: F) -> F::Output {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = pin!(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("in-memory sink unexpectedly yielded"),
    }
}

#[derive(Default)]
struct ProbeSink {
    bytes: Vec<u8>,
    requested: Vec<usize>,
    max_write: usize,
    fail_after: Option<usize>,
    zero_write: bool,
    cancel_after_write: Option<Rc<Cell<bool>>>,
    fail_flush: bool,
    flushes: usize,
}

impl SequentialSink for ProbeSink {
    async fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        self.requested.push(bytes.len());
        if self
            .fail_after
            .is_some_and(|limit| self.bytes.len() >= limit)
        {
            return Err(Error::Io(io::Error::other("deliberate sink failure")));
        }
        if self.zero_write {
            return Ok(0);
        }
        let written = bytes.len().min(self.max_write);
        self.bytes.extend_from_slice(&bytes[..written]);
        if let Some(flag) = &self.cancel_after_write {
            flag.set(true);
        }
        Ok(written)
    }

    async fn flush(&mut self) -> Result<()> {
        self.flushes += 1;
        if self.fail_flush {
            return Err(Error::Io(io::Error::other("deliberate flush failure")));
        }
        Ok(())
    }
}

struct Flag(Rc<Cell<bool>>);

impl Cancellation for Flag {
    fn is_cancelled(&self) -> bool {
        self.0.get()
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
        .expect("expected PDF marker")
}

#[test]
fn classic_xref_points_to_each_object_and_has_exact_entry_width() {
    let mut sink = ProbeSink {
        max_write: 2,
        ..ProbeSink::default()
    };
    let limits = Limits {
        io_chunk_bytes: 3,
        ..Limits::default()
    };
    let bytes_written = run(async {
        let mut pdf = PdfWriter::new(&mut sink, &limits, &NeverCancel).await?;
        let catalog = pdf.reserve_object()?;
        let pages = pdf.reserve_object()?;
        let page = pdf.reserve_object()?;
        pdf.write_object(catalog, b"<< /Type /Catalog /Pages 2 0 R >>")
            .await?;
        pdf.write_object(pages, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
            .await?;
        pdf.write_object(
            page,
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 20] >>",
        )
        .await?;
        pdf.finish(catalog).await
    })
    .expect("synthetic PDF");
    assert_eq!(bytes_written, sink.bytes.len() as u64);
    assert_eq!(sink.flushes, 1);
    assert!(sink.requested.iter().all(|&count| count <= 3));
    assert!(sink.requested.contains(&2));

    let xref_start = find(&sink.bytes, b"xref\n0 4\n");
    let entries_start = xref_start + b"xref\n0 4\n".len();
    assert_eq!(
        &sink.bytes[entries_start..entries_start + 20],
        b"0000000000 65535 f \n"
    );
    for number in 1..=3 {
        let start = entries_start + number * 20;
        let entry = &sink.bytes[start..start + 20];
        assert_eq!(&entry[10..], b" 00000 n \n");
        let offset = std::str::from_utf8(&entry[..10])
            .unwrap()
            .parse::<usize>()
            .unwrap();
        let header = format!("{number} 0 obj\n");
        assert!(sink.bytes[offset..].starts_with(header.as_bytes()));
    }
    let startxref = find(&sink.bytes, b"startxref\n") + b"startxref\n".len();
    let endxref = sink.bytes[startxref..]
        .iter()
        .position(|&byte| byte == b'\n')
        .unwrap()
        + startxref;
    assert_eq!(
        std::str::from_utf8(&sink.bytes[startxref..endxref])
            .unwrap()
            .parse::<usize>()
            .unwrap(),
        xref_start
    );
}

#[test]
fn stream_length_counts_binary_payload_but_not_endstream_delimiter() {
    let mut sink = ProbeSink {
        max_write: usize::MAX,
        ..ProbeSink::default()
    };
    let payload = b"one\nendstream\nendobj\nxref\0two\xff";
    run(async {
        let limits = Limits::default();
        let mut pdf = PdfWriter::new(&mut sink, &limits, &NeverCancel).await?;
        let catalog = pdf.reserve_object()?;
        let image = pdf.reserve_object()?;
        let length = pdf.reserve_object()?;
        pdf.write_object(catalog, b"<< /Type /Catalog >>").await?;
        assert!(matches!(
            pdf.begin_stream(image, image, b"").await,
            Err(Error::InvalidInput { .. })
        ));
        pdf.begin_stream(image, length, b"/Subtype /Image").await?;
        assert!(matches!(
            pdf.write_bytes(b"not a plain object").await,
            Err(Error::InvalidInput { .. })
        ));
        pdf.write_stream_bytes(&payload[..10]).await?;
        pdf.write_stream_bytes(&payload[10..]).await?;
        pdf.end_stream().await?;
        assert!(matches!(
            pdf.end_stream().await,
            Err(Error::InvalidInput { .. })
        ));
        pdf.finish(catalog).await
    })
    .expect("stream PDF");
    assert!(
        sink.bytes
            .windows(payload.len())
            .any(|part| part == payload)
    );
    assert!(
        sink.bytes
            .windows(b"/Length 3 0 R\n/Subtype /Image\n".len())
            .any(|part| part == b"/Length 3 0 R\n/Subtype /Image\n")
    );
    let length_object = format!("3 0 obj\n{}\nendobj\n", payload.len());
    assert!(
        sink.bytes
            .windows(length_object.len())
            .any(|part| part == length_object.as_bytes())
    );
}

#[test]
fn empty_stream_gets_zero_length_object() {
    let mut sink = ProbeSink {
        max_write: usize::MAX,
        ..ProbeSink::default()
    };
    run(async {
        let limits = Limits::default();
        let mut pdf = PdfWriter::new(&mut sink, &limits, &NeverCancel).await?;
        let root = pdf.reserve_object()?;
        let stream = pdf.reserve_object()?;
        let length = pdf.reserve_object()?;
        pdf.begin_stream(stream, length, b"").await?;
        pdf.write_stream_bytes(b"").await?;
        pdf.end_stream().await?;
        pdf.write_object(root, b"<< /Type /Catalog >>").await?;
        pdf.finish(root).await
    })
    .expect("empty stream PDF");
    assert!(
        sink.bytes
            .windows(b"3 0 obj\n0\nendobj\n".len())
            .any(|part| part == b"3 0 obj\n0\nendobj\n")
    );
}

#[test]
fn missing_duplicate_and_open_objects_are_rejected() {
    let mut sink = ProbeSink {
        max_write: usize::MAX,
        ..ProbeSink::default()
    };
    let limits = Limits::default();
    run(async {
        let mut pdf = PdfWriter::new(&mut sink, &limits, &NeverCancel).await?;
        let root = pdf.reserve_object()?;
        let missing = pdf.reserve_object()?;
        assert!(matches!(
            pdf.end_object().await,
            Err(Error::InvalidInput { .. })
        ));
        assert!(matches!(
            pdf.write_stream_bytes(b"x").await,
            Err(Error::InvalidInput { .. })
        ));
        pdf.begin_object(root).await?;
        assert!(matches!(
            pdf.begin_object(missing).await,
            Err(Error::InvalidInput { .. })
        ));
        pdf.write_bytes(b"<< /Type /Catalog >>").await?;
        pdf.end_object().await?;
        assert!(matches!(
            pdf.begin_object(root).await,
            Err(Error::InvalidInput { .. })
        ));
        assert!(matches!(
            pdf.finish(root).await,
            Err(Error::InvalidInput { .. })
        ));
        Ok::<(), Error>(())
    })
    .unwrap();
    assert!(!sink.bytes.windows(5).any(|part| part == b"xref\n"));
}

#[test]
fn finish_requires_a_written_catalog_reserved_by_this_writer() {
    let limits = Limits::default();
    let mut other_sink = ProbeSink {
        max_write: usize::MAX,
        ..ProbeSink::default()
    };
    let mut sink = ProbeSink {
        max_write: usize::MAX,
        ..ProbeSink::default()
    };
    run(async {
        let mut other = PdfWriter::new(&mut other_sink, &limits, &NeverCancel).await?;
        other.reserve_object()?;
        let foreign = other.reserve_object()?;

        let mut pdf = PdfWriter::new(&mut sink, &limits, &NeverCancel).await?;
        let root = pdf.reserve_object()?;
        assert!(matches!(
            pdf.begin_object(foreign).await,
            Err(Error::InvalidInput {
                reason: "PDF object number was not reserved"
            })
        ));
        assert!(matches!(
            pdf.finish(root).await,
            Err(Error::InvalidInput {
                reason: "PDF catalog object has not been written"
            })
        ));
        Ok::<(), Error>(())
    })
    .unwrap();
    assert!(!sink.bytes.windows(5).any(|part| part == b"xref\n"));
}

#[test]
fn xref_preflight_fails_before_emitting_any_xref_bytes() {
    let mut sink = ProbeSink {
        max_write: usize::MAX,
        ..ProbeSink::default()
    };
    let limits = Limits {
        max_output_bytes: 100,
        ..Limits::default()
    };
    let written_before_finish = run(async {
        let mut pdf = PdfWriter::new(&mut sink, &limits, &NeverCancel).await?;
        let root = pdf.reserve_object()?;
        pdf.write_object(root, b"<< /Type /Catalog >>").await?;
        let written = pdf.position();
        assert!(matches!(
            pdf.finish(root).await,
            Err(Error::LimitExceeded { .. })
        ));
        Ok::<u64, Error>(written)
    })
    .unwrap();
    assert_eq!(sink.bytes.len() as u64, written_before_finish);
    assert!(!sink.bytes.windows(5).any(|part| part == b"xref\n"));
}

#[test]
fn object_index_growth_obeys_allocation_limit() {
    let mut sink = ProbeSink {
        max_write: usize::MAX,
        ..ProbeSink::default()
    };
    let limits = Limits {
        io_chunk_bytes: 1,
        max_allocation_bytes: 32,
        ..Limits::default()
    };
    run(async {
        let mut pdf = PdfWriter::new(&mut sink, &limits, &NeverCancel).await?;
        for expected in 1..=4 {
            assert_eq!(pdf.reserve_object()?.number(), expected);
        }
        assert!(matches!(
            pdf.reserve_object(),
            Err(Error::LimitExceeded {
                resource: "allocation bytes",
                ..
            })
        ));
        Ok::<(), Error>(())
    })
    .unwrap();
}

#[test]
fn zero_and_failing_sinks_return_io_errors_and_poison_partial_writer() {
    let mut zero = ProbeSink {
        max_write: usize::MAX,
        zero_write: true,
        ..ProbeSink::default()
    };
    assert!(matches!(
        run(PdfWriter::new(&mut zero, &Limits::default(), &NeverCancel)),
        Err(Error::Io(ref error)) if error.kind() == io::ErrorKind::WriteZero
    ));

    let mut failing = ProbeSink {
        max_write: 1,
        fail_after: Some(16),
        ..ProbeSink::default()
    };
    run(async {
        let limits = Limits::default();
        let mut pdf = PdfWriter::new(&mut failing, &limits, &NeverCancel).await?;
        let root = pdf.reserve_object()?;
        assert!(matches!(pdf.begin_object(root).await, Err(Error::Io(_))));
        assert_eq!(pdf.position(), 16);
        assert!(matches!(
            pdf.reserve_object(),
            Err(Error::InvalidInput { .. })
        ));
        Ok::<(), Error>(())
    })
    .unwrap();
}

#[test]
fn cancellation_and_flush_failure_propagate() {
    let cancelled = Rc::new(Cell::new(false));
    let flag = Flag(cancelled.clone());
    let mut cancel_sink = ProbeSink {
        max_write: 1,
        cancel_after_write: Some(cancelled),
        ..ProbeSink::default()
    };
    assert!(matches!(
        run(PdfWriter::new(&mut cancel_sink, &Limits::default(), &flag)),
        Err(Error::Cancelled)
    ));
    assert_eq!(cancel_sink.bytes.len(), 1);

    let mut flush_sink = ProbeSink {
        max_write: usize::MAX,
        fail_flush: true,
        ..ProbeSink::default()
    };
    assert!(matches!(
        run(async {
            let limits = Limits::default();
            let mut pdf = PdfWriter::new(&mut flush_sink, &limits, &NeverCancel).await?;
            let root = pdf.reserve_object()?;
            pdf.write_object(root, b"<< /Type /Catalog >>").await?;
            pdf.finish(root).await
        }),
        Err(Error::Io(_))
    ));
    assert_eq!(flush_sink.flushes, 1);
}
