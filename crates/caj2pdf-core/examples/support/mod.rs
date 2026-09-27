// SPDX-License-Identifier: MIT

//! Native-only helpers for optional SHA-pinned JBIG2 diagnostics.

use caj2pdf_core::{
    Limits,
    jbig2::mq::{MQ_STATE_COUNT, MqState, MqTable},
};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::{
    env,
    error::Error as StdError,
    fs::{self, File, OpenOptions},
    future::Future,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    pin::pin,
    task::{Context, Poll, Waker},
    time::{SystemTime, UNIX_EPOCH},
};

const TABLE_SHA: &str = "bdf6eeeca3bc5d5a8dc1a13acc7698ec356c886b27f6526f3e09fc2c8520ac57";

pub fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("native file unexpectedly yielded"),
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn digest_file(path: &Path, range: Option<(u64, u64)>) -> Result<String, Box<dyn StdError>> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    let (start, count) = range.unwrap_or((0, length));
    if start.checked_add(count).is_none_or(|end| end > length) {
        return Err("digest range escapes source".into());
    }
    file.seek(SeekFrom::Start(start))?;
    let mut remaining = count;
    let mut buffer = [0u8; 64 * 1024];
    let mut hash = Sha256::new();
    while remaining > 0 {
        let requested = remaining.min(buffer.len() as u64) as usize;
        let got = file.read(&mut buffer[..requested])?;
        if got == 0 {
            return Err("source shortened while hashing".into());
        }
        hash.update(&buffer[..got]);
        remaining -= got as u64;
    }
    Ok(hex(&hash.finalize()))
}

pub fn table(path: &Path, limits: &Limits) -> Result<MqTable, Box<dyn StdError>> {
    let canonical = path.canonicalize()?;
    if !canonical.starts_with("/tmp") {
        return Err("private table fixture must stay in /tmp".into());
    }
    let mut bytes = Vec::new();
    File::open(&canonical)?
        .take(16 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 16 * 1024 || hex(&Sha256::digest(&bytes)) != TABLE_SHA {
        return Err("private table fixture size or SHA differs".into());
    }
    let text = std::str::from_utf8(&bytes)?;
    let mut lines = text.lines();
    if lines.next() != Some("T88-2000-H2") || lines.next() != Some("47") {
        return Err("private table framing differs".into());
    }
    let mut states = Vec::new();
    states.try_reserve_exact(MQ_STATE_COUNT)?;
    for _ in 0..MQ_STATE_COUNT {
        let row = lines.next().ok_or("missing table row")?;
        let parts: Vec<_> = row.split_whitespace().collect();
        if parts.len() != 4 {
            return Err("table row width differs".into());
        }
        let switch: u8 = parts[3].parse()?;
        if switch > 1 {
            return Err("table switch differs".into());
        }
        states.push(MqState {
            qe: parts[0].parse()?,
            next_mps: parts[1].parse()?,
            next_lps: parts[2].parse()?,
            switch_mps: switch == 1,
        });
    }
    MqTable::new(states, limits).map_err(Into::into)
}

pub struct TempStore(PathBuf);

impl TempStore {
    pub fn create(prefix: &str) -> Result<(Self, File), Box<dyn StdError>> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = env::temp_dir().join(format!("{prefix}-{}-{nonce}.bin", std::process::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options.open(&path)?;
        Ok((Self(path), file))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempStore {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub fn peak_rss_kib() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find(|line| line.starts_with("VmHWM:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

pub fn allocated_disk_bytes(metadata: &fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        metadata.blocks().saturating_mul(512)
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        0
    }
}
