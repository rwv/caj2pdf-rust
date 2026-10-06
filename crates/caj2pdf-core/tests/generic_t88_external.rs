// SPDX-License-Identifier: MIT

//! Opt-in generic-only pixel check. No official state rows or corpus bytes
//! enter Git; the private fixture and source files are required at run time.

use caj2pdf_core::{
    Limits, NeverCancel,
    jbig2::{
        SegmentSpan,
        generic::GenericRegionDecoder,
        mq::{ContextBank, MQ_STATE_COUNT, MqState, MqTable},
        read_segment_header,
    },
    native::SeekableSource,
    read_payload,
};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::{env, fs::File, io::Read, path::Path};

const TABLE_FIXTURE_SHA: &str = "bdf6eeeca3bc5d5a8dc1a13acc7698ec356c886b27f6526f3e09fc2c8520ac57";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha_file(path: &Path) -> String {
    let mut file = File::open(path).expect("missing external source");
    let mut hasher = Sha256::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut chunk).unwrap();
        if n == 0 {
            break;
        }
        hasher.update(&chunk[..n]);
    }
    hex(&hasher.finalize())
}

fn external_table() -> MqTable {
    let path = env::var("CAJ2PDF_T88_H2_FIXTURE_FILE")
        .expect("NOT_RUN: set external official table fixture path");
    let canonical = Path::new(&path)
        .canonicalize()
        .expect("missing external table fixture");
    assert!(
        canonical.starts_with("/tmp"),
        "private table fixture must stay in /tmp"
    );
    assert!(canonical.metadata().unwrap().len() <= 16 * 1024);
    assert_eq!(
        sha_file(&canonical),
        TABLE_FIXTURE_SHA,
        "official table fixture changed"
    );
    let text = std::fs::read_to_string(canonical).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("T88-2000-H2"));
    assert_eq!(lines.next(), Some("47"));
    let mut states = Vec::new();
    for _ in 0..MQ_STATE_COUNT {
        let fields: Vec<_> = lines
            .next()
            .expect("missing external state row")
            .split_whitespace()
            .collect();
        assert_eq!(fields.len(), 4);
        let switch: u8 = fields[3].parse().unwrap();
        assert!(switch <= 1);
        states.push(MqState {
            qe: fields[0].parse().unwrap(),
            next_mps: fields[1].parse().unwrap(),
            next_lps: fields[2].parse().unwrap(),
            switch_mps: switch == 1,
        });
    }
    assert_eq!(states, caj2pdf_core::jbig2::mq::STANDARD_STATES);
    MqTable::standard()
}

struct HashSink {
    hash: Sha256,
    bytes: u64,
}
impl HashSink {
    fn new() -> Self {
        Self {
            hash: Sha256::new(),
            bytes: 0,
        }
    }
}
impl Write for HashSink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.hash.update(bytes);
        self.bytes += bytes.len() as u64;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
#[ignore = "NOT_RUN without private official T.88 table and external CAJSamples corpus"]
fn generic_only_spots_match_black_box_pixel_hashes() {
    let root = env::var("CAJ2PDF_GENERIC_CORPUS_DIR").expect("NOT_RUN: set external corpus root");
    let limits = Limits::default();
    let table = external_table();
    // Source hashes and segment coordinates come from #42's SHA-checked
    // directory inventory. Pixel hashes come from a temporary page-info +
    // generic-only PDF decoded by the #43 black-box oracle. The tools may
    // share one decoder backend; their agreement is not implementation
    // independence. #50 covers all 546 images, beyond this spot check.
    let spots = [
        (
            "issue-58/混凝土道面评价指标分析_谢永亮.caj",
            "8974d024e0cbb54009419aa8c91c9ba286dd74f056c3b19524ee5c626c947c85",
            15974,
            32535,
            2366,
            3368,
            "dae0fec2ea4c15de4b70f590a6bb3629f8bf17c225f0d0d4427743a04084fcb6",
        ),
        (
            "issue-43/Windows9x_NT操作系统的磁盘备份与恢复的研究与实现_张宗伟.caj",
            "826608ef34b850926d1291ddba1773305bd44a0bf4170b4a9ae8c7d83c0b7134",
            666802,
            8422,
            2368,
            3431,
            "72170496b556f7628b436b8e924e9bc4aa2815dc8d31106ab64e8ea0dbecde8f",
        ),
    ];
    for (name, source_sha, offset, length, width, height, pixel_sha) in spots {
        let path = Path::new(&root).join(name);
        assert_eq!(sha_file(&path), source_sha, "source changed: {name}");
        let mut source = SeekableSource::new(File::open(path).unwrap()).unwrap();
        let header = read_segment_header(
            &mut source,
            SegmentSpan { offset, length },
            &limits,
            &NeverCancel,
        )
        .unwrap();
        assert_eq!(header.segment_type, 38);
        let mut contexts = ContextBank::new(1024, &limits).unwrap();
        let mut sink = HashSink::new();
        let mut buffer = Vec::new();
        let payload = read_payload(
            &mut source,
            offset,
            length,
            &mut buffer,
            &limits,
            &NeverCancel,
        )
        .unwrap();
        let mut decoder = GenericRegionDecoder::new(
            payload,
            &header,
            &table,
            &mut contexts,
            &mut sink,
            &limits,
            &NeverCancel,
        )
        .unwrap();
        assert_eq!(
            (
                decoder.progress().info.width,
                decoder.progress().info.height
            ),
            (width, height)
        );
        for _ in 0..height {
            assert!(decoder.decode_next_row().unwrap());
        }
        assert!(!decoder.decode_next_row().unwrap());
        let report = decoder.finish().unwrap();
        assert_eq!(
            report.progress.pixels_decoded,
            u64::from(width) * u64::from(height)
        );
        assert_eq!(sink.bytes, u64::from(width.div_ceil(8)) * u64::from(height));
        assert_eq!(
            hex(&sink.hash.finalize()),
            pixel_sha,
            "pixel mismatch: {name}"
        );
        eprintln!(
            "PASS generic-only {name}: {} rows, {} pixels, semantic MQ byte {}",
            report.progress.rows_written,
            report.progress.pixels_decoded,
            report.progress.mq.input_offset,
        );
    }
}
