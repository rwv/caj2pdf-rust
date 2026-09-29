// SPDX-License-Identifier: MIT

use super::*;
use caj2pdf_core::jbig2::text_composer::RandomAccessScratch;

pub(super) struct Scratch {
    shared: Rc<RefCell<Shared>>,
    store: u32,
    size: u64,
    max_bytes: u64,
}

impl Scratch {
    pub(super) fn new(shared: Rc<RefCell<Shared>>, store: u32, max_bytes: u64) -> Self {
        Self {
            shared,
            store,
            size: 0,
            max_bytes,
        }
    }

    fn check_range(&self, offset: u64, length: usize) -> Result<()> {
        if offset > self.size || length as u64 > self.size - offset {
            return Err(Error::InvalidInput {
                reason: "scratch request escapes declared size",
            });
        }
        Ok(())
    }
}

impl RandomAccessScratch for Scratch {
    fn size(&self) -> Result<u64> {
        Ok(self.size)
    }

    async fn set_len(&mut self, bytes: u64) -> Result<()> {
        if bytes > self.max_bytes {
            return Err(Error::LimitExceeded {
                resource: "WASM scratch bytes",
                limit: self.max_bytes,
                attempted: bytes,
            });
        }
        poll_fn(|_| {
            let mut shared = self.shared.borrow_mut();
            if shared.cancelled {
                return Poll::Ready(Err(Error::Cancelled));
            }
            if let Some(Response::Resize) = shared.response {
                shared.response = None;
                self.size = bytes;
                return Poll::Ready(Ok(()));
            }
            shared.request = Some(Request::ScratchResize {
                store: self.store,
                bytes,
            });
            Poll::Pending
        })
        .await
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        self.check_range(offset, destination.len())?;
        if destination.is_empty() {
            return Ok(0);
        }
        poll_fn(|_| {
            let mut shared = self.shared.borrow_mut();
            if shared.cancelled {
                return Poll::Ready(Err(Error::Cancelled));
            }
            if let Some(Response::Read(length)) = shared.response {
                shared.response = None;
                destination[..length].copy_from_slice(&shared.staging[..length]);
                return Poll::Ready(Ok(length));
            }
            let length = destination.len().min(shared.staging.len());
            shared.request = Some(Request::ScratchRead {
                store: self.store,
                offset,
                length,
            });
            Poll::Pending
        })
        .await
    }

    async fn write_at(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        self.check_range(offset, bytes.len())?;
        if bytes.is_empty() {
            return Ok(0);
        }
        poll_fn(|_| {
            let mut shared = self.shared.borrow_mut();
            if shared.cancelled {
                return Poll::Ready(Err(Error::Cancelled));
            }
            if let Some(Response::Write(length)) = shared.response {
                shared.response = None;
                return Poll::Ready(Ok(length));
            }
            if shared.request.is_none() {
                let length = bytes.len().min(shared.staging.len());
                shared.staging[..length].copy_from_slice(&bytes[..length]);
                shared.request = Some(Request::ScratchWrite {
                    store: self.store,
                    offset,
                    length,
                });
            }
            Poll::Pending
        })
        .await
    }

    async fn flush(&mut self) -> Result<()> {
        poll_fn(|_| {
            let mut shared = self.shared.borrow_mut();
            if shared.cancelled {
                return Poll::Ready(Err(Error::Cancelled));
            }
            if let Some(Response::Flush) = shared.response {
                shared.response = None;
                return Poll::Ready(Ok(()));
            }
            shared.request = Some(Request::ScratchFlush { store: self.store });
            Poll::Pending
        })
        .await
    }
}
