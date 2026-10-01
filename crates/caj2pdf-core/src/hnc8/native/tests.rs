// SPDX-License-Identifier: MIT

use super::*;
use crate::{Error, Limits};
use std::{
    cell::Cell,
    future::Future,
    rc::Rc,
    task::{Context, Poll, Waker},
};

fn run<T>(future: impl Future<Output = T>) -> T {
    let mut future = std::pin::pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("unexpected suspension"),
    }
}

#[derive(Clone, Default)]
struct Cancel(Rc<Cell<bool>>);
impl Cancellation for Cancel {
    fn is_cancelled(&self) -> bool {
        self.0.get()
    }
}

struct Source {
    bytes: Vec<u8>,
    short: usize,
    max_request: usize,
    reads: Rc<Cell<usize>>,
    fault: bool,
    cancel_on_read: Option<Cancel>,
}
impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }
    async fn read_at(&mut self, offset: u64, out: &mut [u8]) -> crate::Result<usize> {
        self.reads.set(self.reads.get() + 1);
        self.max_request = self.max_request.max(out.len());
        if self.fault {
            return Err(Error::InvalidInput {
                reason: "synthetic source failure",
            });
        }
        let at = offset as usize;
        let n = self
            .short
            .min(out.len())
            .min(self.bytes.len().saturating_sub(at));
        out[..n].copy_from_slice(&self.bytes[at..at + n]);
        if let Some(cancel) = &self.cancel_on_read {
            cancel.0.set(true);
        }
        Ok(n)
    }
}
fn fixture(words: &[[u16; 2]], images: u16) -> Source {
    let mut bytes = vec![0; 100];
    bytes[0] = 0xc8;
    bytes[8..12].copy_from_slice(&1_u32.to_le_bytes());
    bytes[80..84].copy_from_slice(&100_u32.to_le_bytes());
    bytes[84..88].copy_from_slice(&(words.len() as u32 * 4).to_le_bytes());
    bytes[88..90].copy_from_slice(&images.to_le_bytes());
    for pair in words {
        for word in pair {
            bytes.extend(word.to_le_bytes());
        }
    }
    Source {
        bytes,
        short: 28,
        max_request: 0,
        reads: Rc::new(Cell::new(0)),
        fault: false,
        cancel_on_read: None,
    }
}
#[derive(Default)]
struct Visitor {
    events: Vec<(u64, NativeRecord)>,
    fail: bool,
    cancel: Option<Cancel>,
    suspend: bool,
}
impl NativeRecordVisitor for Visitor {
    async fn visit(&mut self, offset: u64, record: NativeRecord) -> crate::Result<()> {
        self.events.push((offset, record));
        if let Some(cancel) = &self.cancel {
            cancel.0.set(true);
        }
        if self.suspend {
            std::future::pending::<()>().await;
        }
        if self.fail {
            return Err(Error::InvalidInput {
                reason: "synthetic visitor failure",
            });
        }
        Ok(())
    }
}
fn parse(source: &mut Source, budget: TextBudget, visitor: &mut Visitor) -> Result<u32> {
    run(async {
        let limits = Limits::default();
        let cancel = Cancel::default();
        let mut reader = Hnc8Reader::open(source, &limits, &cancel, Default::default()).await?;
        reader.next_page().await?;
        reader.visit_native_records(budget, visitor).await
    })
}

#[test]
fn streams_raw_glyph_context_and_atomic_drawing_image_records() {
    // Invented coordinates/payloads. Marker-looking point/image words must not
    // turn into controls, glyphs or premature ends.
    let words = [
        [0x8001, 13],
        [0x8002, 41],
        [0x801d, 4],
        [0x8067, 9],
        [90, 0xcec4],
        [20, 0xa0c4],
        [0x8001, 71],
        [0x8002, 89],
        [15, 0xffff],
        [0x8006, 0xa381],
        [0x8004, 8],
        [2, 0x8001],
        [0xffff, 5],
        [0x8006, 0xa383],
        [31, 47],
        [53, 59],
        [0x8006, 0xa38b],
        [61, 67],
        [71, 73],
        [0xffff, 5],
        [0x800a, 0xd300],
        [0x8004, 0x8006],
        [1, 2],
        [3, 4],
        [5, 6],
        [7, 8],
        [9, 10],
        [0x8004, 39],
    ];
    for short in 1..=28 {
        let mut source = fixture(&words, 1);
        source.short = short;
        let mut visitor = Visitor::default();
        assert_eq!(
            parse(&mut source, TextBudget::default(), &mut visitor).unwrap(),
            14
        );
        assert!(source.max_request <= 24);
        assert_eq!(
            visitor.events[4],
            (
                116,
                NativeRecord::Glyph {
                    x: 90,
                    y: 13,
                    style: 41,
                    code: 0xcec4
                }
            )
        );
        assert_eq!(
            visitor.events[5].1,
            NativeRecord::Glyph {
                x: 20,
                y: 13,
                style: 41,
                code: 0xa0c4
            }
        );
        assert_eq!(
            visitor.events[8].1,
            NativeRecord::Glyph {
                x: 15,
                y: 71,
                style: 89,
                code: 0xffff
            }
        );
        assert_eq!(
            visitor.events[9],
            (
                136,
                NativeRecord::Drawing {
                    tag: 0x8006,
                    style: 0xa381,
                    points: [[0x8004, 8], [2, 0x8001]]
                }
            )
        );
        assert_eq!(visitor.events[10].0, 152);
        assert_eq!(visitor.events[11].0, 164);
        assert_eq!(
            visitor.events[12].1,
            NativeRecord::Image {
                words: [0xd300, 0x8004, 0x8006, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
            }
        );
        assert_eq!(visitor.events[13], (208, NativeRecord::End { value: 39 }));
    }
}

#[test]
fn unsupported_records_stop_without_consuming_their_payload_as_glyphs() {
    for pair in [
        [0x8006, 0],
        [0x8010, 1],
        [0xc053, 7],
        [0xc054, 8],
        [0x8072, 0],
        [0x8073, 0],
        [0x8074, 0],
        [0x801d, 1],
        [0x8067, 0],
        [0x800a, 0],
        [0xffff, 5],
    ] {
        let mut source = fixture(
            &[[0x8001, 5], [0x8002, 7], pair, [13, 0xcec4], [0x8004, 0]],
            0,
        );
        let mut visitor = Visitor::default();
        let error = parse(&mut source, TextBudget::default(), &mut visitor).unwrap_err();
        assert_eq!(error.offset, 108);
        assert_eq!(error.page, Some(1));
        assert!(matches!(error.kind, ErrorKind::Unsupported { .. }));
        assert_eq!(visitor.events.len(), 2);
    }
}

#[test]
fn context_end_and_image_counts_are_checked() {
    for words in [
        vec![[7, 0xcec4]],
        vec![[0x8001, 5], [7, 0xcec4]],
        vec![[0x8002, 5], [7, 0xcec4]],
        vec![],
        vec![[0x8001, 5]],
        vec![[0x8004, 0], [0x8004, 0]],
        vec![[0x8006, 0xa381], [1, 2], [3, 4], [0x8004, 0]],
    ] {
        let error = parse(
            &mut fixture(&words, 0),
            TextBudget::default(),
            &mut Visitor::default(),
        )
        .unwrap_err();
        assert!(matches!(error.kind, ErrorKind::Malformed { .. }));
    }
    let mut image = vec![[0x800a, 0xd300]];
    image.extend([[7, 11]; 6]);
    image.push([0x8004, 0]);
    for (words, count) in [(&image[..], 0), (&[[0x8004, 0]][..], 1)] {
        assert!(matches!(
            parse(
                &mut fixture(words, count),
                TextBudget::default(),
                &mut Visitor::default()
            )
            .unwrap_err()
            .kind,
            ErrorKind::Malformed { .. }
        ));
    }
}

#[test]
fn never_reads_past_indexed_span_for_any_truncated_record() {
    for words in [
        vec![[0x8004, 0]],
        vec![[0x8006, 0xa381], [1, 2], [3, 4], [0xffff, 5]],
        vec![[0x8006, 0xa383], [1, 2], [3, 4]],
        vec![
            [0x800a, 0xd300],
            [1, 2],
            [3, 4],
            [5, 6],
            [7, 8],
            [9, 10],
            [11, 12],
        ],
    ] {
        for length in 1..words.len() * 4 {
            let mut source = fixture(&words, u16::from(words[0][0] == 0x800a));
            source.bytes[84..88].copy_from_slice(&(length as u32).to_le_bytes());
            let error =
                parse(&mut source, TextBudget::default(), &mut Visitor::default()).unwrap_err();
            assert!(
                matches!(error.kind, ErrorKind::Truncated { .. }),
                "{length}: {error:?}"
            );
        }
    }
}

#[test]
fn budgets_and_protected_index_fail_before_emitting_records() {
    let budget = TextBudget::default();
    for limited in [
        TextBudget {
            max_span_bytes: 3,
            ..budget
        },
        TextBudget {
            max_decoded_bytes: 3,
            ..budget
        },
        TextBudget {
            max_working_bytes: 4095,
            ..budget
        },
        TextBudget {
            max_records: 0,
            ..budget
        },
    ] {
        let mut visitor = Visitor::default();
        assert!(matches!(
            parse(&mut fixture(&[[0x8004, 0]], 0), limited, &mut visitor)
                .unwrap_err()
                .kind,
            ErrorKind::LimitExceeded { .. }
        ));
        assert!(visitor.events.is_empty());
    }
    let mut source = fixture(&[[0x8004, 0]], 1);
    assert!(matches!(
        parse(
            &mut source,
            TextBudget {
                max_images: 0,
                ..budget
            },
            &mut Visitor::default()
        )
        .unwrap_err()
        .kind,
        ErrorKind::LimitExceeded { .. }
    ));
    let mut source = fixture(&[[0x8004, 0]], 0);
    source.bytes[80..84].copy_from_slice(&96_u32.to_le_bytes());
    assert!(matches!(
        parse(&mut source, budget, &mut Visitor::default())
            .unwrap_err()
            .kind,
        ErrorKind::Malformed { .. }
    ));
}

#[test]
fn reader_state_errors_failure_and_abandonment_poison_the_cursor() {
    let limits = Limits::default();
    let cancel = Cancel::default();
    let mut source = fixture(&[[0x8004, 0]], 0);
    let mut reader = run(Hnc8Reader::open(
        &mut source,
        &limits,
        &cancel,
        Default::default(),
    ))
    .unwrap();
    let mut visitor = Visitor::default();
    assert!(matches!(
        run(reader.visit_native_records(TextBudget::default(), &mut visitor))
            .unwrap_err()
            .kind,
        ErrorKind::NoCurrentPage
    ));
    run(reader.next_page()).unwrap();
    visitor.suspend = true;
    let reads = reader.source.reads.clone();
    let before = reads.get();
    {
        let mut future =
            std::pin::pin!(reader.visit_native_records(TextBudget::default(), &mut visitor));
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        let after = reads.get();
        assert_eq!(after, before + 1);
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        assert_eq!(reads.get(), after);
    }
    assert!(matches!(
        run(reader.visit_native_records(TextBudget::default(), &mut visitor))
            .unwrap_err()
            .kind,
        ErrorKind::Poisoned
    ));
    assert!(matches!(
        run(reader.next_page()).unwrap_err().kind,
        ErrorKind::Poisoned
    ));
}

#[test]
fn cancellation_source_and_visitor_failures_are_located() {
    for fault in 0..5 {
        let limits = Limits::default();
        let cancel = Cancel::default();
        let mut source = fixture(&[[0x8004, 0]], 0);
        let mut reader = run(Hnc8Reader::open(
            &mut source,
            &limits,
            &cancel,
            Default::default(),
        ))
        .unwrap();
        run(reader.next_page()).unwrap();
        let mut visitor = Visitor::default();
        match fault {
            0 => cancel.0.set(true),
            1 => reader.source.cancel_on_read = Some(cancel.clone()),
            2 => visitor.cancel = Some(cancel.clone()),
            3 => reader.source.fault = true,
            _ => visitor.fail = true,
        }
        let error =
            run(reader.visit_native_records(TextBudget::default(), &mut visitor)).unwrap_err();
        assert_eq!(error.offset, 100);
        if fault < 3 {
            assert!(matches!(error.kind, ErrorKind::Cancelled));
        } else {
            assert!(matches!(error.kind, ErrorKind::Source { .. }));
        }
        assert!(reader.poisoned);
    }
}

#[test]
fn current_variant_is_enforced_and_success_releases_poison() {
    for variant in [Variant::HnA, Variant::HnB, Variant::C8] {
        let limits = Limits::default();
        let cancel = Cancel::default();
        let mut source = fixture(
            &[
                [0x801d, 0],
                [0x8067, 5],
                [0x8067, 6],
                [0x8067, 8],
                [0x8004, 0],
            ],
            0,
        );
        let mut reader = run(Hnc8Reader::open(
            &mut source,
            &limits,
            &cancel,
            Default::default(),
        ))
        .unwrap();
        run(reader.next_page()).unwrap();
        reader.header.variant = variant;
        let result =
            run(reader.visit_native_records(TextBudget::default(), &mut Visitor::default()));
        if variant == Variant::C8 {
            assert_eq!(result.unwrap(), 5);
            assert!(!reader.poisoned);
            assert_eq!(run(reader.next_image()).unwrap(), None);
            assert_eq!(run(reader.next_page()).unwrap(), None);
        } else {
            assert!(matches!(
                result.unwrap_err().kind,
                ErrorKind::Unsupported { .. }
            ));
        }
    }
}
