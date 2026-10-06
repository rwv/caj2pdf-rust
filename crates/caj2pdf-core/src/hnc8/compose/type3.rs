// SPDX-License-Identifier: MIT

//! Temporary live views for the type-3 decoder. All handles are used
//! serially by one conversion future; no task can access a store concurrently.

use super::*;
use crate::hnc8::type3_image::{
    Type3RefinedStore, Type3Store, Type3Workspaces, emit_type3_xobject, prepare_type3_image,
};
use crate::pdf::ImageObject;
use std::cell::{Cell, RefCell};

struct Meter {
    bytes: Cell<u64>,
    work: Cell<u64>,
    budget: ComposeBudget,
}

impl Meter {
    fn charge(&self, count: usize) -> crate::Result<()> {
        let attempted = self.work.get().saturating_add(count as u64);
        if attempted > self.budget.max_row_store_io_bytes {
            return Err(Error::LimitExceeded {
                resource: "type-3 store I/O bytes",
                limit: self.budget.max_row_store_io_bytes,
                attempted,
            });
        }
        self.work.set(attempted);
        Ok(())
    }
    fn resize(&self, old: u64, new: u64) -> crate::Result<u64> {
        let attempted = self.bytes.get().saturating_sub(old).saturating_add(new);
        if attempted > self.budget.max_row_store_bytes {
            return Err(Error::LimitExceeded {
                resource: "type-3 store bytes",
                limit: self.budget.max_row_store_bytes,
                attempted,
            });
        }
        Ok(attempted)
    }
    fn resized(&self, bytes: u64) {
        self.bytes.set(bytes);
    }
}

struct Store<'a, T> {
    inner: RefCell<&'a mut T>,
    length: Cell<u64>,
    meter: &'a Meter,
}
impl<'a, T> Store<'a, T> {
    fn new(inner: &'a mut T, meter: &'a Meter) -> Self {
        Self {
            inner: RefCell::new(inner),
            length: Cell::new(0),
            meter,
        }
    }
}

// Each operation is awaited before another handle is used. Holding this
// exclusive borrow across adapter I/O enforces that serial access contract.
#[allow(clippy::await_holding_refcell_ref)]
impl<T: RandomAccessScratch> RandomAccessScratch for &Store<'_, T> {
    fn size(&self) -> crate::Result<u64> {
        Ok(self.length.get())
    }
    async fn set_len(&mut self, bytes: u64) -> crate::Result<()> {
        let total = self.meter.resize(self.length.get(), bytes)?;
        self.inner.borrow_mut().set_len(bytes).await?;
        self.length.set(bytes);
        self.meter.resized(total);
        Ok(())
    }
    async fn read_at(&mut self, offset: u64, bytes: &mut [u8]) -> crate::Result<usize> {
        self.meter.charge(bytes.len())?;
        let read = self.inner.borrow_mut().read_at(offset, bytes).await?;
        if read > bytes.len() {
            return Err(Error::InvalidInput {
                reason: "type-3 store overreported read",
            });
        }
        Ok(read)
    }
    async fn write_at(&mut self, offset: u64, bytes: &[u8]) -> crate::Result<usize> {
        self.meter.charge(bytes.len())?;
        let end = offset
            .checked_add(bytes.len() as u64)
            .ok_or(Error::InvalidInput {
                reason: "type-3 store write extent overflow",
            })?;
        if end > self.length.get() {
            return Err(Error::InvalidInput {
                reason: "type-3 write escapes declared store extent",
            });
        }
        let written = self.inner.borrow_mut().write_at(offset, bytes).await?;
        if written > bytes.len() {
            return Err(Error::InvalidInput {
                reason: "type-3 store overreported write",
            });
        }
        Ok(written)
    }
    async fn flush(&mut self) -> crate::Result<()> {
        self.inner.borrow_mut().flush().await
    }
}
impl<T: RandomAccessScratch> RangedSource for &Store<'_, T> {
    fn size(&self) -> u64 {
        self.length.get()
    }
    async fn read_at(&mut self, offset: u64, bytes: &mut [u8]) -> crate::Result<usize> {
        RandomAccessScratch::read_at(self, offset, bytes).await
    }
}
impl<T: RandomAccessScratch> SequentialSink for &Store<'_, T> {
    async fn write(&mut self, bytes: &[u8]) -> crate::Result<usize> {
        let start = self.length.get();
        let end = start
            .checked_add(bytes.len() as u64)
            .ok_or(Error::InvalidInput {
                reason: "type-3 append extent overflow",
            })?;
        RandomAccessScratch::set_len(self, end).await?;
        let written = RandomAccessScratch::write_at(self, start, bytes).await?;
        if written < bytes.len() {
            RandomAccessScratch::set_len(self, start + written as u64).await?;
        }
        Ok(written)
    }
    async fn flush(&mut self) -> crate::Result<()> {
        RandomAccessScratch::flush(self).await
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn emit<S, W, T, C>(
    source: &mut S,
    document: &mut PdfDocument<'_, W, C>,
    at: At,
    checked: CheckedType3,
    workspaces: &mut ComposeWorkspaces<'_, T>,
    options: ComposeOptions,
    limits: &Limits,
    cancellation: &C,
) -> Result<(ImageObject, Option<TextHeaderAnomaly>), ComposeError>
where
    S: RangedSource,
    W: SequentialSink,
    T: RandomAccessScratch,
    C: Cancellation,
{
    let stores = workspaces.type3.as_mut().expect("type-3 stores checked");
    let meter = Meter {
        bytes: Cell::new(0),
        work: Cell::new(0),
        budget: options.budget,
    };
    let result = async {
        for store in [
            &mut *stores.first,
            &mut *stores.second,
            &mut *stores.refined,
            &mut *workspaces.rows,
        ] {
            store
                .set_len(0)
                .await
                .map_err(at.io(ComposeStage::Scratch))?;
        }
        let page = checked.page();
        let first = Store::new(stores.first, &meter);
        let second = Store::new(stores.second, &meter);
        let refined = Store::new(stores.refined, &meter);
        let text = Store::new(workspaces.rows, &meter);
        let mut decoding = Type3Workspaces {
            first: Type3Store {
                reader: &mut &first,
                compose_reader: &mut &first,
                writer: &mut &first,
            },
            second: Type3Store {
                reader: &mut &second,
                compose_reader: &mut &second,
                writer: &mut &second,
            },
            refined: Type3RefinedStore {
                reader: &mut &refined,
                writer: &mut &refined,
            },
            text: &mut &text,
        };
        let prepared = prepare_type3_image(
            source,
            stores.table,
            &mut decoding,
            checked,
            at,
            options.type3,
            limits,
            cancellation,
        )
        .await?;
        let (object, report) = emit_type3_xobject(
            source,
            document,
            stores.table,
            prepared,
            page.width,
            at,
            options.type3,
            limits,
            cancellation,
        )
        .await?;
        Ok((object, report.text_header_anomaly))
    }
    .await;
    // Try every cleanup even if an earlier one fails. The caller still owns
    // the stores and must dispose them when a pending future is dropped.
    let mut cleanup = None;
    for store in [
        &mut *stores.first,
        &mut *stores.second,
        &mut *stores.refined,
        &mut *workspaces.rows,
    ] {
        if let Err(error) = store.set_len(0).await {
            cleanup.get_or_insert(error);
        }
    }
    match (result, cleanup) {
        (Ok(emitted), None) => Ok(emitted),
        (Err(primary), None) => Err(primary),
        (Ok(_), Some(cleanup)) => Err(at.io(ComposeStage::Cleanup)(cleanup)),
        (Err(primary), Some(cleanup)) => Err(at.error((
            ComposeStage::Cleanup,
            ComposeErrorKind::Cleanup {
                primary: Box::new(primary),
                cleanup,
            },
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ready;

    #[derive(Default)]
    struct Memory {
        bytes: Vec<u8>,
        overread: bool,
        overwrite: bool,
    }
    impl RandomAccessScratch for Memory {
        fn size(&self) -> crate::Result<u64> {
            Ok(self.bytes.len() as u64)
        }
        async fn set_len(&mut self, bytes: u64) -> crate::Result<()> {
            self.bytes.resize(bytes as usize, 0);
            Ok(())
        }
        async fn read_at(&mut self, offset: u64, bytes: &mut [u8]) -> crate::Result<usize> {
            if self.overread {
                return Ok(bytes.len() + 1);
            }
            let count = bytes.len().min(self.bytes.len() - offset as usize).min(1);
            bytes[..count].copy_from_slice(&self.bytes[offset as usize..offset as usize + count]);
            Ok(count)
        }
        async fn write_at(&mut self, offset: u64, bytes: &[u8]) -> crate::Result<usize> {
            if self.overwrite {
                return Ok(bytes.len() + 1);
            }
            let count = bytes.len().min(1);
            self.bytes[offset as usize..offset as usize + count].copy_from_slice(&bytes[..count]);
            Ok(count)
        }
        async fn flush(&mut self) -> crate::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn live_symbol_views_keep_short_appends_visible_and_bound_aggregate_storage() {
        let meter = Meter {
            bytes: Cell::new(0),
            work: Cell::new(0),
            budget: ComposeBudget {
                max_row_store_bytes: 5,
                ..Default::default()
            },
        };
        let mut memory = Memory::default();
        let mut other = Memory::default();
        let store = Store::new(&mut memory, &meter);
        let second = Store::new(&mut other, &meter);
        let mut writer = &store;
        let mut reader = &store;
        assert_eq!(
            ready(SequentialSink::write(&mut writer, &[0xab, 0xcd])).unwrap(),
            1
        );
        assert_eq!(RangedSource::size(&reader), 1);
        assert_eq!(store.inner.borrow().size().unwrap(), 1);
        assert_eq!(RandomAccessScratch::size(&reader).unwrap(), 1);
        assert_eq!(
            ready(SequentialSink::write(&mut writer, &[0xcd])).unwrap(),
            1
        );
        let mut byte = [0];
        assert_eq!(
            ready(RangedSource::read_at(&mut reader, 1, &mut byte)).unwrap(),
            1
        );
        assert_eq!(byte, [0xcd]);
        ready(SequentialSink::flush(&mut writer)).unwrap();
        ready(RandomAccessScratch::set_len(&mut &second, 3)).unwrap();
        assert!(ready(RandomAccessScratch::set_len(&mut writer, 3)).is_err());
        ready(RandomAccessScratch::set_len(&mut writer, 0)).unwrap();
        assert_eq!(RangedSource::size(&reader), 0);
        store.inner.borrow_mut().overread = true;
        assert!(ready(RangedSource::read_at(&mut reader, 0, &mut byte)).is_err());
        store.inner.borrow_mut().overwrite = true;
        assert!(ready(SequentialSink::write(&mut writer, &[1])).is_err());
        assert!(ready(RandomAccessScratch::write_at(&mut writer, u64::MAX, &[1])).is_err());
        assert!(ready(RandomAccessScratch::write_at(&mut writer, 1, &[1])).is_err());
        store.length.set(u64::MAX);
        assert!(ready(SequentialSink::write(&mut writer, &[1])).is_err());
    }
}
