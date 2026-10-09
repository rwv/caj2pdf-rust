// SPDX-License-Identifier: MIT

use caj2pdf_core::{
    ConversionOptions, Limits, NeverCancel, convert_with_ttkn_response, pdf::TtknResponse,
};

const PDF: &[u8] = include_bytes!("fixtures/ttkn/authored.pdf");
const RESPONSE: &[u8] = include_bytes!("fixtures/ttkn/response.txt");

#[test]
fn authored_stream_and_outline_decrypt_with_bounded_reads() {
    let response = TtknResponse::new(RESPONSE.trim_ascii()).unwrap();
    let mut output = Vec::new();
    let report = convert_with_ttkn_response(
        &mut &PDF[..],
        &mut output,
        ConversionOptions::default(),
        &response,
        &Limits::default(),
        &mut NeverCancel,
    )
    .unwrap();
    assert_eq!(report.pages_converted, 1);
    assert_eq!(report.bookmarks_written, 0); // The existing outline is preserved, not newly imported.
    assert!(output.windows(16).any(|w| w == b"FEFF004100750074"));
    assert!(!output.windows(8).any(|w| w == b"/Encrypt"));
    let start = output.windows(7).position(|w| w == b"stream\n").unwrap() + 7;
    let mut decoded = Vec::new();
    std::io::Read::read_to_end(
        &mut flate2::read::ZlibDecoder::new(&output[start..]),
        &mut decoded,
    )
    .unwrap();
    assert_eq!(decoded, include_bytes!("fixtures/ttkn/content.txt"));
}

use caj2pdf_core::{ErrorKind, Progress, RangedSource, Result};

struct ShortSource {
    bytes: Vec<u8>,
    cap: usize,
    read: u64,
}
impl RangedSource for ShortSource {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize> {
        assert!(out.len() <= self.cap);
        let start = offset as usize;
        let n = out.len().min(3).min(self.bytes.len().saturating_sub(start));
        out[..n].copy_from_slice(&self.bytes[start..start + n]);
        self.read += n as u64;
        Ok(n)
    }
}

fn converted(bytes: &[u8], response: &[u8], limits: &Limits) -> Result<Vec<u8>> {
    let response = TtknResponse::new(response)?;
    let mut out = Vec::new();
    convert_with_ttkn_response(
        &mut &bytes[..],
        &mut out,
        ConversionOptions::default(),
        &response,
        limits,
        &mut NeverCancel,
    )?;
    Ok(out)
}

#[test]
fn chunk_boundaries_short_reads_and_accounting_are_exact() {
    let baseline = converted(PDF, RESPONSE.trim_ascii(), &Limits::default()).unwrap();
    for cap in [1, 7, 31, 257] {
        let mut source = ShortSource {
            bytes: PDF.to_vec(),
            cap,
            read: 0,
        };
        let response = TtknResponse::new(RESPONSE.trim_ascii()).unwrap();
        let limits = Limits {
            io_chunk_bytes: cap,
            ..Limits::default()
        };
        let mut out = Vec::new();
        let report = convert_with_ttkn_response(
            &mut source,
            &mut out,
            ConversionOptions::default(),
            &response,
            &limits,
            &mut NeverCancel,
        )
        .unwrap();
        assert_eq!(out, baseline);
        assert_eq!(report.input_bytes_read, source.read);
        assert_eq!(report.output_bytes_written, out.len() as u64);
    }
}

#[test]
fn wrong_missing_and_case_changed_credentials_fail_without_output() {
    for response in [
        b"00000000000000000000000000000000".as_slice(),
        &RESPONSE.trim_ascii().to_ascii_uppercase(),
    ] {
        let error = converted(PDF, response, &Limits::default()).unwrap_err();
        assert!(matches!(error.kind, ErrorKind::Encrypted));
        assert!(
            !error
                .to_string()
                .contains(std::str::from_utf8(response).unwrap())
        );
    }
    let mut out = Vec::new();
    assert!(
        caj2pdf_core::convert(
            &mut &PDF[..],
            &mut out,
            ConversionOptions::default(),
            &Limits::default(),
            &mut NeverCancel
        )
        .is_err()
    );
    assert!(out.is_empty());
    for invalid in [
        b"".as_slice(),
        b"0123456789abcdef",
        b"zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
        RESPONSE,
    ] {
        assert!(TtknResponse::new(invalid).is_err());
    }
}

#[test]
fn truncated_wrappers_bad_base64_and_wrong_profiles_are_refused() {
    for cut in [0, 8, 100, PDF.len() - 100, PDF.len() - 1] {
        assert!(converted(&PDF[..cut], RESPONSE.trim_ascii(), &Limits::default()).is_err());
    }
    for (needle, replacement) in [
        (b"<version>2.0".as_slice(), b"<version>3.0".as_slice()),
        (b"auth type=\"1\"", b"auth type=\"2\""),
        (b"WebFastLoad", b"WebFastLoaX"),
        (b"<password>", b"<password>!"),
    ] {
        let mut input = PDF.to_vec();
        let at = input
            .windows(needle.len())
            .position(|v| v == needle)
            .unwrap();
        input.splice(at..at + needle.len(), replacement.iter().copied());
        assert!(converted(&input, RESPONSE.trim_ascii(), &Limits::default()).is_err());
    }
    let mut damaged = PDF.to_vec();
    let stream = damaged.windows(7).position(|s| s == b"stream\n").unwrap() + 7;
    let end = damaged[stream..]
        .windows(10)
        .position(|s| s == b"\nendstream")
        .unwrap()
        + stream;
    damaged[end - 1] ^= 0xff;
    assert!(matches!(
        converted(&damaged, RESPONSE.trim_ascii(), &Limits::default())
            .unwrap_err()
            .kind,
        ErrorKind::Encrypted
    ));
}

#[test]
fn limits_cancellation_and_sink_failure_propagate() {
    for limits in [
        Limits {
            max_allocation_bytes: 128,
            ..Limits::default()
        },
        Limits {
            max_input_bytes: 100,
            ..Limits::default()
        },
        Limits {
            max_output_bytes: 100,
            ..Limits::default()
        },
    ] {
        assert!(matches!(
            converted(PDF, RESPONSE.trim_ascii(), &limits)
                .unwrap_err()
                .kind,
            ErrorKind::LimitExceeded { .. }
        ));
    }
    struct Cancel;
    impl Progress for Cancel {
        fn is_cancelled(&self) -> bool {
            true
        }
    }
    let response = TtknResponse::new(RESPONSE.trim_ascii()).unwrap();
    let mut out = Vec::new();
    let error = convert_with_ttkn_response(
        &mut &PDF[..],
        &mut out,
        ConversionOptions::default(),
        &response,
        &Limits::default(),
        &mut Cancel,
    )
    .unwrap_err();
    assert!(matches!(error.kind, ErrorKind::Cancelled));
    assert!(out.is_empty());
    struct Refuse;
    impl std::io::Write for Refuse {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("test sink"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    assert!(matches!(
        convert_with_ttkn_response(
            &mut &PDF[..],
            &mut Refuse,
            ConversionOptions::default(),
            &response,
            &Limits::default(),
            &mut NeverCancel
        )
        .unwrap_err()
        .kind,
        ErrorKind::Io(_)
    ));
}
