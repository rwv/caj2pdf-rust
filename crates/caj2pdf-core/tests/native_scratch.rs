// SPDX-License-Identifier: MIT

use caj2pdf_core::{
    Error, MAX_IO_CHUNK, jbig2::text_composer::RandomAccessScratch, native::FileScratch,
};
use std::{
    fs::{self, File, OpenOptions},
    future::Future,
    path::PathBuf,
    pin::pin,
    sync::atomic::{AtomicU64, Ordering},
    task::{Context, Poll, Waker},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Temporary(PathBuf);

impl Temporary {
    fn new() -> (Self, File) {
        let path = std::env::temp_dir().join(format!(
            "caj2pdf-native-scratch-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        (Self(path), file)
    }
}

impl Drop for Temporary {
    fn drop(&mut self) {
        fs::remove_file(&self.0).unwrap();
    }
}

fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("native scratch unexpectedly suspended"),
    }
}

#[test]
fn positioned_rows_resize_reset_and_return_owned_file() {
    let (_temporary, file) = Temporary::new();
    let mut scratch = FileScratch::new(file, 32).unwrap();
    ready(async {
        assert_eq!(scratch.size().unwrap(), 0);
        scratch.set_len(16).await.unwrap();
        assert_eq!(scratch.write_at(8, &[9, 8, 7, 6]).await.unwrap(), 4);
        assert_eq!(scratch.write_at(0, &[1, 2, 3, 4]).await.unwrap(), 4);
        scratch.flush().await.unwrap();
        let mut bytes = [0xff; 16];
        assert_eq!(scratch.read_at(0, &mut bytes).await.unwrap(), 16);
        assert_eq!(bytes, [1, 2, 3, 4, 0, 0, 0, 0, 9, 8, 7, 6, 0, 0, 0, 0]);
        assert_eq!(scratch.read_at(16, &mut []).await.unwrap(), 0);
        assert_eq!(scratch.write_at(16, &[]).await.unwrap(), 0);
        scratch.set_len(0).await.unwrap();
        scratch.set_len(32).await.unwrap();
        let mut reused = [0xff; 32];
        assert_eq!(scratch.read_at(0, &mut reused).await.unwrap(), 32);
        assert_eq!(reused, [0; 32], "reset cannot retain the previous image");
        scratch.set_len(0).await.unwrap();
    });
    let file = scratch.into_inner();
    assert_eq!(file.metadata().unwrap().len(), 0);
    file.set_len(4).unwrap();
}

#[test]
fn bounds_fail_before_modifying_the_file() {
    let (_temporary, file) = Temporary::new();
    file.set_len(8).unwrap();
    assert!(matches!(
        FileScratch::new(file.try_clone().unwrap(), 7),
        Err(Error::LimitExceeded {
            limit: 7,
            attempted: 8,
            ..
        })
    ));
    let mut scratch = FileScratch::new(file, 8).unwrap();
    ready(async {
        assert!(matches!(
            scratch.set_len(9).await,
            Err(Error::LimitExceeded { .. })
        ));
        assert_eq!(scratch.size().unwrap(), 8);
        for offset in [7, 9, u64::MAX] {
            assert!(matches!(
                scratch.read_at(offset, &mut [0; 2]).await,
                Err(Error::InvalidInput { .. })
            ));
            assert!(matches!(
                scratch.write_at(offset, &[42; 2]).await,
                Err(Error::InvalidInput { .. })
            ));
        }
        assert!(scratch.read_at(9, &mut []).await.is_err());
        assert!(scratch.write_at(9, &[]).await.is_err());
        let mut oversized = vec![0; MAX_IO_CHUNK + 1];
        assert!(matches!(
            scratch.read_at(0, &mut oversized).await,
            Err(Error::LimitExceeded {
                resource: "I/O request bytes",
                ..
            })
        ));
        assert!(matches!(
            scratch.write_at(0, &oversized).await,
            Err(Error::LimitExceeded {
                resource: "I/O request bytes",
                ..
            })
        ));
        let mut unchanged = [0xff; 8];
        scratch.read_at(0, &mut unchanged).await.unwrap();
        assert_eq!(unchanged, [0; 8]);
    });
}

#[test]
fn file_access_errors_are_preserved() {
    let (temporary, file) = Temporary::new();
    file.set_len(4).unwrap();
    drop(file);
    let mut readonly = FileScratch::new(File::open(&temporary.0).unwrap(), 4).unwrap();
    let mut writeonly = FileScratch::new(
        OpenOptions::new().write(true).open(&temporary.0).unwrap(),
        4,
    )
    .unwrap();
    ready(async {
        assert!(matches!(readonly.set_len(0).await, Err(Error::Io(_))));
        assert!(matches!(
            readonly.write_at(0, &[1]).await,
            Err(Error::Io(_))
        ));
        assert!(matches!(
            writeonly.read_at(0, &mut [0]).await,
            Err(Error::Io(_))
        ));
        assert_eq!(readonly.size().unwrap(), 4);
    });
}

#[cfg(unix)]
#[test]
fn non_regular_storage_is_refused() {
    assert!(matches!(
        FileScratch::new(File::open(std::env::temp_dir()).unwrap(), 0),
        Err(Error::InvalidInput { .. })
    ));
}
