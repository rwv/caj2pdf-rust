// SPDX-License-Identifier: MIT

//! Native file handling: seekable inputs, bounded stdin spooling, same-file
//! checks, and staged path output that is renamed into place only on success.

use crate::CliError;
use crate::command::Endpoint;
#[cfg(unix)]
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, BufWriter, Read, Seek, Write};
#[cfg(unix)]
use std::os::fd::AsFd;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
#[cfg(windows)]
use std::os::windows::ffi::{OsStrExt, OsStringExt};
#[cfg(windows)]
use std::os::windows::io::AsHandle;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

const COPY_CHUNK: usize = 64 * 1024;
const OUTPUT_BUFFER: usize = 64 * 1024;
pub(crate) const TEMP_ATTEMPTS: u32 = 64;
pub(crate) static NEXT_TEMP: AtomicU32 = AtomicU32::new(0);

/// A device and inode pair identifying one file.
type Identity = (u64, u64);
#[cfg(unix)]
type InputIdentity = Identity;
#[cfg(windows)]
type InputIdentity = Option<Identity>;

#[cfg(unix)]
fn identity(metadata: &Metadata) -> Identity {
    (metadata.dev(), metadata.ino())
}

#[cfg(unix)]
fn file_metadata(file: &File) -> io::Result<Option<Metadata>> {
    file.metadata().map(Some)
}

#[cfg(windows)]
fn file_metadata(file: &File) -> io::Result<Option<Metadata>> {
    if winapi_util::file::typ(file)?.is_disk() {
        file.metadata().map(Some)
    } else {
        Ok(None)
    }
}

#[cfg(windows)]
fn identity(file: &File) -> io::Result<Identity> {
    let info = winapi_util::file::information(file)?;
    Ok((info.volume_serial_number(), info.file_index()))
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
    #[cfg(unix)]
    identity: Identity,
    #[cfg(windows)]
    identity: Option<Identity>,
    pub name: String,
}

#[cfg(unix)]
fn duplicate<F: AsFd>(handle: F) -> io::Result<File> {
    Ok(File::from(handle.as_fd().try_clone_to_owned()?))
}

#[cfg(windows)]
fn duplicate<F: AsHandle>(handle: F) -> io::Result<File> {
    Ok(File::from(handle.as_handle().try_clone_to_owned()?))
}

/// Open an input endpoint. A regular file is used in place; stdin, pipes,
/// and other forward-only files are spooled to an anonymous temporary file
/// of at most `limit` bytes.
pub fn open_input(endpoint: &Endpoint, limit: u64) -> Result<Input, CliError> {
    open_input_spooling_in(endpoint, limit, &std::env::temp_dir())
}

/// [`open_input`] with forward-only inputs spooled in `spool_directory`.
pub fn open_input_spooling_in(
    endpoint: &Endpoint,
    limit: u64,
    spool_directory: &Path,
) -> Result<Input, CliError> {
    let name = describe(endpoint);
    let fail = |error: io::Error| CliError::runtime(format!("cannot read {name}: {error}"));
    let file = match endpoint {
        Endpoint::Std => duplicate(io::stdin()),
        Endpoint::Path(path) => File::open(path),
    }
    .map_err(fail)?;
    let metadata = file_metadata(&file).map_err(fail)?;
    if metadata.as_ref().is_some_and(Metadata::is_dir) {
        return Err(CliError::runtime(format!("{name} is a directory")));
    }
    #[cfg(unix)]
    let file_identity = identity(metadata.as_ref().expect("Unix file metadata"));
    #[cfg(windows)]
    let file_identity = if metadata.as_ref().is_some_and(Metadata::is_file) {
        Some(identity(&file).map_err(fail)?)
    } else {
        None
    };
    let file = if metadata.as_ref().is_some_and(Metadata::is_file) {
        file
    } else {
        spool(file, limit, spool_directory).map_err(|error| match error {
            SpoolError::Io(error) => fail(error),
            SpoolError::TooLarge => {
                CliError::runtime(format!("{name} exceeds the {limit}-byte input limit"))
            }
        })?
    };
    Ok(Input {
        file,
        identity: file_identity,
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
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        options.mode(mode);
        #[cfg(windows)]
        let _ = mode; // Access inherits the containing directory's Windows ACL.
        match options.open(&path) {
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            result => return result.map(|file| (path, file)),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no unused temporary file name",
    ))
}

/// Create private storage whose name is removed before it is used. The OS
/// releases the file when the handle closes, including after process exit.
fn anonymous_file(directory: &Path) -> io::Result<File> {
    let (path, file) = create_unique(directory, &OsString::from(".caj2pdf-spool"), 0o600)?;
    fs::remove_file(path)?;
    Ok(file)
}

/// Copy a forward-only reader into an unlinked temporary file in
/// `directory`. The name is removed before copying begins, so the storage
/// is released when the returned handle closes, including after a failure.
pub fn spool<R: Read>(mut reader: R, limit: u64, directory: &Path) -> Result<File, SpoolError> {
    let mut file = anonymous_file(directory)?;
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
    /// Inputs the target must not become between staging and commit.
    inputs: Vec<(InputIdentity, String)>,
}

impl Output {
    pub fn writer(&mut self) -> &mut BufWriter<File> {
        match self {
            Self::Stdout(writer) => writer,
            Self::Staged(staged) => staged.writer.as_mut().expect("uncommitted output"),
        }
    }

    /// Flush the output and, for a path, move it to the target.
    pub fn commit(self) -> Result<(), CliError> {
        match self {
            Self::Stdout(mut writer) => writer.flush().map_err(stdout_error),
            Self::Staged(staged) => staged.commit(),
        }
    }
}

impl Staged {
    /// Synchronize the staged file and give it the target name. The same-file
    /// and existence checks are repeated here because the target may have
    /// changed during conversion. Without `--force` the file is hard-linked,
    /// which fails atomically when the target exists; a file system without
    /// hard links falls back to a re-check and rename, leaving a short race.
    /// With `--force` a target swapped for an input between the re-check and
    /// the rename is still replaced. Drop removes the temporary name.
    fn commit(mut self) -> Result<(), CliError> {
        let target = self.target.display().to_string();
        let fail =
            |error: io::Error| CliError::runtime(format!("cannot write '{target}': {error}"));
        let file = self
            .writer
            .take()
            .expect("uncommitted output")
            .into_inner()
            .map_err(|error| fail(error.into_error()))?;
        file.sync_all().map_err(fail)?;
        drop(file);
        if let Ok(metadata) = fs::metadata(&self.target) {
            #[cfg(unix)]
            check_distinct(&metadata, &self.inputs, &format!("output '{target}'"))?;
            #[cfg(windows)]
            check_windows_path(
                &self.target,
                &metadata,
                &self.inputs,
                &format!("output '{target}'"),
            )?;
        }
        if !self.force {
            match fs::hard_link(&self.temp, &self.target) {
                Ok(()) => return Ok(()),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    return Err(exists_error(&self.target));
                }
                Err(_) if fs::symlink_metadata(&self.target).is_ok() => {
                    return Err(exists_error(&self.target));
                }
                Err(_) => {}
            }
        }
        fs::rename(&self.temp, &self.target).map_err(fail)?;
        self.temp = PathBuf::new();
        Ok(())
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        self.writer.take();
        if !self.temp.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.temp);
        }
    }
}

pub fn stdout_error(error: io::Error) -> CliError {
    CliError::runtime(format!("cannot write standard output: {error}"))
}

fn exists_error(path: &Path) -> CliError {
    CliError::runtime(format!(
        "output '{}' already exists; use --force to replace it",
        path.display()
    ))
}

/// Check that `metadata` does not belong to any input file.
#[cfg(unix)]
fn check_distinct(
    metadata: &Metadata,
    inputs: &[(Identity, String)],
    output: &str,
) -> Result<(), CliError> {
    match inputs.iter().find(|(id, _)| *id == identity(metadata)) {
        Some((_, name)) => Err(CliError::runtime(format!(
            "{output} is the same file as input {name}; refusing to overwrite an input"
        ))),
        None => Ok(()),
    }
}

#[cfg(windows)]
fn check_windows_file(
    file: &File,
    inputs: &[(InputIdentity, String)],
    output: &str,
) -> Result<(), CliError> {
    let id =
        identity(file).map_err(|e| CliError::runtime(format!("cannot identify {output}: {e}")))?;
    if let Some((_, name)) = inputs.iter().find(|(input, _)| *input == Some(id)) {
        return Err(CliError::runtime(format!(
            "{output} is the same file as input {name}; refusing to overwrite an input"
        )));
    }
    Ok(())
}

#[cfg(windows)]
fn check_windows_path(
    path: &Path,
    metadata: &Metadata,
    inputs: &[(InputIdentity, String)],
    output: &str,
) -> Result<(), CliError> {
    if metadata.is_file() {
        let file = File::open(path).map_err(|e| {
            CliError::runtime(format!("cannot open {output} for identity check: {e}"))
        })?;
        check_windows_file(&file, inputs, output)?;
    }
    Ok(())
}

/// Refuse PDF output to a terminal. It is checked before any input is opened,
/// so standard input is not spooled first. `stdout_is_terminal` is injected so
/// the refusal can be tested without a pseudo-terminal.
pub fn refuse_terminal(endpoint: &Endpoint, stdout_is_terminal: bool) -> Result<(), CliError> {
    if *endpoint == Endpoint::Std && stdout_is_terminal {
        Err(CliError::runtime(
            "refusing to write binary PDF data to a terminal; redirect standard output or use -o FILE",
        ))
    } else {
        Ok(())
    }
}

/// Longest output file name kept in a temporary name, leaving room for the
/// prefix and suffix within the usual 255-byte name limit.
const TEMP_STEM_BYTES: usize = 200;

/// Validate and open an output endpoint.
pub fn open_output(
    endpoint: &Endpoint,
    force: bool,
    inputs: &[&Input],
) -> Result<Output, CliError> {
    let inputs: Vec<_> = inputs
        .iter()
        .map(|input| (input.identity, input.name.clone()))
        .collect();
    let path = match endpoint {
        Endpoint::Std => {
            // Rust reopens a closed descriptor 1 as /dev/null at startup, so
            // duplicating it fails only when descriptors are exhausted.
            let (metadata, stdout) = duplicate(io::stdout())
                .and_then(|file| Ok((file_metadata(&file)?, file)))
                .map_err(stdout_error)?;
            if let Some(metadata) = metadata.filter(Metadata::is_file) {
                #[cfg(unix)]
                check_distinct(&metadata, &inputs, "standard output")?;
                #[cfg(windows)]
                {
                    let _ = metadata;
                    check_windows_file(&stdout, &inputs, "standard output")?;
                }
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
        #[cfg(unix)]
        check_distinct(&metadata, &inputs, &shown)?;
        #[cfg(windows)]
        check_windows_path(path, &metadata, &inputs, &shown)?;
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
    #[cfg(unix)]
    let name = file_name.as_bytes();
    let mut stem = OsString::from(".");
    #[cfg(unix)]
    stem.push(OsStr::from_bytes(&name[..name.len().min(TEMP_STEM_BYTES)]));
    #[cfg(windows)]
    stem.push(OsString::from_wide(
        &file_name
            .encode_wide()
            .take(TEMP_STEM_BYTES)
            .collect::<Vec<_>>(),
    ));
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
        inputs,
    }))
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[test]
    fn private_spool_is_bounded_and_deleted_when_closed() {
        let directory =
            std::env::temp_dir().join(format!("caj2pdf-win-spool-{}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let mut file = spool(&b"abc"[..], 3, &directory).unwrap();
        let mut data = Vec::new();
        file.read_to_end(&mut data).unwrap();
        assert_eq!(data, b"abc");
        drop(file);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
        assert!(matches!(
            spool(&b"abcd"[..], 3, &directory),
            Err(SpoolError::TooLarge)
        ));
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
        fs::remove_dir(directory).unwrap();
    }
}
