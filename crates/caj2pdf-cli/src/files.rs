// SPDX-License-Identifier: MIT

//! Linux file handling: seekable inputs, bounded stdin spooling, same-file
//! checks, and staged path output that is renamed into place only on success.

use crate::CliError;
use crate::args::Endpoint;
use std::ffi::OsString;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, BufWriter, IsTerminal, Read, Seek, Write};
use std::os::fd::AsFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

const COPY_CHUNK: usize = 64 * 1024;
const OUTPUT_BUFFER: usize = 64 * 1024;
pub(crate) const TEMP_ATTEMPTS: u32 = 64;
pub(crate) static NEXT_TEMP: AtomicU32 = AtomicU32::new(0);

/// A device and inode pair identifying one file.
pub type Identity = (u64, u64);

pub fn identity(metadata: &Metadata) -> Identity {
    (metadata.dev(), metadata.ino())
}

/// Human-readable name of an endpoint for diagnostics.
fn describe(endpoint: &Endpoint) -> String {
    match endpoint {
        Endpoint::Std => "standard input".to_owned(),
        Endpoint::Path(path) => format!("'{}'", path.display()),
    }
}

/// An opened, seekable input. Forward-only input has already been spooled.
pub struct Input {
    pub file: File,
    /// The identity of the file the user named; kept for same-file checks.
    pub identity: Identity,
    pub name: String,
}

fn duplicate<F: AsFd>(handle: F) -> io::Result<File> {
    Ok(File::from(handle.as_fd().try_clone_to_owned()?))
}

/// Open an input endpoint. A regular file is used in place; stdin, pipes,
/// and other forward-only files are spooled to an anonymous temporary file
/// of at most `limit` bytes.
pub fn open_input(endpoint: &Endpoint, limit: u64) -> Result<Input, CliError> {
    let name = describe(endpoint);
    let fail = |error: io::Error| CliError::runtime(format!("cannot read {name}: {error}"));
    let file = match endpoint {
        Endpoint::Std => duplicate(io::stdin()),
        Endpoint::Path(path) => File::open(path),
    }
    .map_err(fail)?;
    let metadata = file.metadata().map_err(fail)?;
    if metadata.is_dir() {
        return Err(CliError::runtime(format!("{name} is a directory")));
    }
    let file = if metadata.is_file() {
        file
    } else {
        spool(file, limit, &std::env::temp_dir()).map_err(|error| match error {
            SpoolError::Io(error) => fail(error),
            SpoolError::TooLarge => {
                CliError::runtime(format!("{name} exceeds the {limit}-byte input limit"))
            }
        })?
    };
    Ok(Input {
        file,
        identity: identity(&metadata),
        name,
    })
}

#[derive(Debug)]
pub enum SpoolError {
    Io(io::Error),
    TooLarge,
}

impl From<io::Error> for SpoolError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Create a new file in `directory` whose name starts with `stem`.
fn create_unique(directory: &Path, stem: &OsString, mode: u32) -> io::Result<(PathBuf, File)> {
    for _ in 0..TEMP_ATTEMPTS {
        let counter = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let mut name = stem.clone();
        name.push(format!(".{}-{counter}.tmp", std::process::id()));
        let path = directory.join(name);
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&path)
        {
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            result => return result.map(|file| (path, file)),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no unused temporary file name",
    ))
}

/// Copy a forward-only reader into an unlinked temporary file in
/// `directory`. The name is removed before copying begins, so the storage
/// is released when the returned handle closes, including after a failure.
pub fn spool<R: Read>(mut reader: R, limit: u64, directory: &Path) -> Result<File, SpoolError> {
    let (path, mut file) = create_unique(directory, &OsString::from(".caj2pdf-spool"), 0o600)?;
    fs::remove_file(&path)?;
    let mut buffer = vec![0; COPY_CHUNK];
    let mut total = 0u64;
    loop {
        let read = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        total += read as u64;
        if total > limit {
            return Err(SpoolError::TooLarge);
        }
        file.write_all(&buffer[..read])?;
    }
    file.rewind()?;
    Ok(file)
}

/// A PDF destination: buffered stdout or a staged sibling of the output path.
pub enum Output {
    Stdout(BufWriter<File>),
    Staged(Staged),
}

/// A temporary sibling file removed on drop unless committed.
pub struct Staged {
    writer: Option<BufWriter<File>>,
    temp: PathBuf,
    target: PathBuf,
    force: bool,
}

impl Output {
    pub fn writer(&mut self) -> &mut BufWriter<File> {
        match self {
            Self::Stdout(writer) => writer,
            Self::Staged(staged) => staged.writer.as_mut().expect("uncommitted output"),
        }
    }

    /// Flush the output and, for a path, rename it over the target.
    pub fn commit(self) -> Result<(), CliError> {
        match self {
            Self::Stdout(mut writer) => writer.flush().map_err(stdout_error),
            Self::Staged(mut staged) => {
                let target = staged.target.display().to_string();
                let fail = |error: io::Error| {
                    CliError::runtime(format!("cannot write '{target}': {error}"))
                };
                let file = staged
                    .writer
                    .take()
                    .expect("uncommitted output")
                    .into_inner()
                    .map_err(|error| fail(error.into_error()))?;
                file.sync_all().map_err(fail)?;
                drop(file);
                if !staged.force && fs::symlink_metadata(&staged.target).is_ok() {
                    return Err(exists_error(&staged.target));
                }
                fs::rename(&staged.temp, &staged.target).map_err(fail)?;
                staged.temp = PathBuf::new();
                Ok(())
            }
        }
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        if !self.temp.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.temp);
        }
    }
}

fn stdout_error(error: io::Error) -> CliError {
    CliError::runtime(format!("cannot write standard output: {error}"))
}

fn exists_error(path: &Path) -> CliError {
    CliError::runtime(format!(
        "output '{}' already exists; use --force to replace it",
        path.display()
    ))
}

/// Check that `metadata` does not belong to any input file.
fn check_distinct(metadata: &Metadata, inputs: &[&Input], output: &str) -> Result<(), CliError> {
    match inputs
        .iter()
        .find(|input| input.identity == identity(metadata))
    {
        Some(input) => Err(CliError::runtime(format!(
            "{output} is the same file as input {}; refusing to overwrite an input",
            input.name
        ))),
        None => Ok(()),
    }
}

/// Validate and open an output endpoint. `stdout_is_terminal` is injected so
/// the terminal refusal can be tested without a pseudo-terminal.
pub fn open_output(
    endpoint: &Endpoint,
    force: bool,
    inputs: &[&Input],
    stdout_is_terminal: bool,
) -> Result<Output, CliError> {
    let path = match endpoint {
        Endpoint::Std => {
            if stdout_is_terminal {
                return Err(CliError::runtime(
                    "refusing to write binary PDF data to a terminal; redirect standard output or use -o FILE",
                ));
            }
            // Rust reopens a closed descriptor 1 as /dev/null at startup, so
            // duplicating it fails only when descriptors are exhausted.
            let (metadata, stdout) = duplicate(io::stdout())
                .and_then(|file| Ok((file.metadata()?, file)))
                .map_err(stdout_error)?;
            if metadata.is_file() {
                check_distinct(&metadata, inputs, "standard output")?;
            }
            return Ok(Output::Stdout(BufWriter::with_capacity(
                OUTPUT_BUFFER,
                stdout,
            )));
        }
        Endpoint::Path(path) => path,
    };
    let shown = format!("output '{}'", path.display());
    if let Ok(metadata) = fs::metadata(path) {
        check_distinct(&metadata, inputs, &shown)?;
        if metadata.is_dir() {
            return Err(CliError::runtime(format!("{shown} is a directory")));
        }
    }
    if !force && fs::symlink_metadata(path).is_ok() {
        return Err(exists_error(path));
    }
    let file_name = path
        .file_name()
        .ok_or_else(|| CliError::runtime(format!("{shown} does not name a file")))?;
    let directory = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let mut stem = OsString::from(".");
    stem.push(file_name);
    let (temp, file) = create_unique(directory, &stem, 0o666).map_err(|error| {
        CliError::runtime(format!(
            "cannot create a temporary file in '{}': {error}",
            directory.display()
        ))
    })?;
    Ok(Output::Staged(Staged {
        writer: Some(BufWriter::with_capacity(OUTPUT_BUFFER, file)),
        temp,
        target: path.clone(),
        force,
    }))
}

/// Whether the process's standard output is a terminal.
pub fn stdout_is_terminal() -> bool {
    io::stdout().is_terminal()
}
