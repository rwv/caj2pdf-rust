// SPDX-License-Identifier: MIT

use super::native_document::{hnb_fixture, native_text, roles};
use super::*;

// A parser that repeatedly reads without advancing must fail the test itself,
// rather than returning an I/O error that could masquerade as a valid rejection.
struct BudgetSource {
    source: Source,
    remaining: usize,
}

impl BudgetSource {
    fn new(bytes: Vec<u8>) -> Self {
        let mut source = Source::new(bytes);
        source.short = 3;
        Self {
            source,
            remaining: 4096,
        }
    }
}

impl RangedSource for BudgetSource {
    fn size(&self) -> u64 {
        self.source.size()
    }

    fn read_at(&mut self, offset: u64, bytes: &mut [u8]) -> crate::Result<usize> {
        assert!(
            self.remaining > 0,
            "conversion exhausted its finite read budget"
        );
        self.remaining -= 1;
        self.source.read_at(offset, bytes)
    }
}

fn paired_raw(records: &[Record]) -> Vec<u8> {
    let mut bytes = vec![3, 0x80, 100, 0, 3, 0x80, 200, 0];
    bytes.extend(image_records(records));
    bytes
}

#[test]
fn admitted_framing_rejects_late_mutations_with_bounded_read_progress() {
    // Each case has a positive control through the same public entry point,
    // limits and short-I/O adapters as its one-field negative mutations.
    for profile in 0..5 {
        for mutation in 0..5 {
            let pages = vec![vec![Record::jpeg(3, 2, 120, 20, 40)]; 2];
            let mut fixture = match profile {
                0 => fixture_with_text(Variant::HnA, &pages, paired_raw),
                1 => fixture_with_text(Variant::HnA, &pages, text),
                2 => hnb_fixture(12, 2),
                3 => hnb_fixture(20, 2),
                _ => fixture_with_text(Variant::C8, &pages, native_text),
            };
            let row = fixture.index + if profile == 2 { 12 } else { 20 };
            let length = u32::from_le_bytes(fixture.bytes[row + 4..row + 8].try_into().unwrap());
            match mutation {
                1 => fixture.bytes[row + 4..row + 8].copy_from_slice(&u32::MAX.to_le_bytes()),
                2 => fixture.bytes[row + 4..row + 8].copy_from_slice(&1_u32.to_le_bytes()),
                // Removes part of the raw end or compressed trailer from the
                // indexed span, without permitting reads into the next section.
                3 => fixture.bytes[row + 4..row + 8].copy_from_slice(&(length - 1).to_le_bytes()),
                _ => (),
            }
            let mut source = BudgetSource::new(fixture.bytes);
            if mutation == 4 {
                source.source.fault_at = Some((fixture.text_offsets[1] as u64, Fault::Zero));
            }
            let mut fonts = [C8FontSource {
                source: BudgetSource::new(crate::pdf::drawing_font()),
                face: 0,
            }];
            let mut sink = Sink {
                short: Some(7),
                ..Default::default()
            };
            let limits = Limits {
                io_chunk_bytes: 64,
                ..Default::default()
            };
            let result = if profile < 2 {
                convert_source_pages_pdf(
                    &mut source,
                    &mut sink,
                    None,
                    no_stores(),
                    &mut Visitor::default(),
                    ComposeOptions::default(),
                    &limits,
                    &NeverCancel,
                )
            } else {
                convert_c8_native_pdf(
                    &mut source,
                    &mut sink,
                    C8FontSources {
                        sources: &mut fonts,
                        roles: roles(),
                    },
                    None,
                    no_stores(),
                    ComposeOptions::default(),
                    &limits,
                    &NeverCancel,
                )
            };
            if mutation == 0 {
                assert_eq!(result.unwrap().output_pages, 2, "profile {profile}");
                assert!(sink.bytes.ends_with(b"%%EOF\n"));
            } else {
                let error = result.unwrap_err();
                assert_eq!(
                    error.page,
                    Some(2),
                    "profile {profile}, mutation {mutation}: {error}"
                );
                assert!(!sink.bytes.ends_with(b"%%EOF\n"));
                assert!(
                    contains(&sink.bytes, b"/Type /Page /Parent "),
                    "first page was not emitted"
                );
            }
            assert!(source.source.max_request <= 64);
            assert!(fonts[0].source.source.max_request <= 64);
            assert!(sink.max_request <= 64);
        }
    }
}
