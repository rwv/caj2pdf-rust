// SPDX-License-Identifier: MIT

//! Native file handling: seekable inputs, bounded stdin spooling, same-file
//! checks, and staged path output that is renamed into place only on success.
//!
//! Temporary files come from `tempfile` and file identity from `same-file`.

use crate::CliError;
use crate::command::Endpoint;
use same_file::Handle;
use std::fs::{self, File, Metadata};
use std::io::{self, BufWriter, Read, Seek, Write};
#[cfg(unix)]
use std::os::fd::AsFd;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(windows)]
use std::os::windows::io::AsHandle;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use tempfile::TempPath;

const COPY_CHUNK: usize = 64 * 1024;
const OUTPUT_BUFFER: usize = 64 * 1024;
/// Name prefix of a staged output, a hidden sibling of the target.
pub(crate) const STAGED_PREFIX: &str = ".caj2pdf-";
/// Name suffix of a staged output.
pub(crate) const STAGED_SUFFIX: &str = ".tmp";

/// Metadata of an open file, or `None` when it is not a disk file. Windows
/// pipes, consoles, and character devices have no meaningful metadata, and
/// asking for it can fail.
#[cfg(unix)]
fn disk_metadata(file: &File) -> io::Result<Option<Metadata>> {
    file.metadata().map(Some)
}

#[cfg(windows)]
fn disk_metadata(file: &File) -> io::Result<Option<Metadata>> {
    if winapi_util::file::typ(file)?.is_disk() {
        file.metadata().map(Some)
    } else {
        Ok(None)
    }
}

/// The identity of an open regular file. Anything else has none and is never
/// compared: it is spooled as input and cannot hold an input's bytes.
fn identity(file: &File, metadata: Option<&Metadata>) -> io::Result<Option<Handle>> {
    if metadata.is_some_and(Metadata::is_file) {
        Handle::from_file(file.try_clone()?).map(Some)
    } else {
        Ok(None)
    }
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
    /// The regular file the user named, kept for same-file checks.
    identity: Option<Rc<Handle>>,
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
    let metadata = disk_metadata(&file).map_err(fail)?;
    if metadata.as_ref().is_some_and(Metadata::is_dir) {
        return Err(CliError::runtime(format!("{name} is a directory")));
    }
    let identity = identity(&file, metadata.as_ref()).map_err(fail)?;
    let file = if identity.is_some() {
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
        identity: identity.map(Rc::new),
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

/// Copy a forward-only reader into an anonymous temporary file in
/// `directory`. The file has no name while it is copied (`O_TMPFILE`, or a
/// name removed at once; delete-on-close on Windows), so its storage is
/// released when the returned handle closes, including after a failure.
pub fn spool<R: Read>(mut reader: R, limit: u64, directory: &Path) -> Result<File, SpoolError> {
    let mut file = tempfile::tempfile_in(directory)?;
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

/// A hidden temporary sibling of the target, removed on drop unless
/// committed.
pub struct Staged {
    // Declared before `temp`, so the file is closed before its name is removed.
    writer: BufWriter<File>,
    temp: TempPath,
    target: PathBuf,
    force: bool,
    /// Inputs the target must not become between staging and commit.
    inputs: Vec<(Rc<Handle>, String)>,
}

impl Output {
    pub fn writer(&mut self) -> &mut BufWriter<File> {
        match self {
            Self::Stdout(writer) => writer,
            Self::Staged(staged) => &mut staged.writer,
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
    /// check is repeated here because the target may have changed during
    /// conversion. Without `--force` the name is given with an exclusive
    /// rename or a hard link, either of which fails atomically when the
    /// target exists; a file system supporting neither falls back to a
    /// re-check and rename, leaving a short race. With `--force` a target
    /// swapped for an input between the re-check and the rename is still
    /// replaced. On failure the temporary name is removed.
    fn commit(self) -> Result<(), CliError> {
        let Self {
            writer,
            temp,
            target,
            force,
            inputs,
        } = self;
        let shown = target.display();
        let fail = |error: io::Error| CliError::runtime(format!("cannot write '{shown}': {error}"));
        let file = writer
            .into_inner()
            .map_err(|error| fail(error.into_error()))?;
        file.sync_all().map_err(fail)?;
        drop(file);
        check_path(&target, &inputs, &format!("output '{shown}'"))?;
        let persisted = if force {
            temp.persist(&target)
        } else {
            match temp.persist_noclobber(&target) {
                Err(error)
                    if error.error.kind() == io::ErrorKind::AlreadyExists
                        || fs::symlink_metadata(&target).is_ok() =>
                {
                    return Err(exists_error(&target));
                }
                Err(error) => error.path.persist(&target),
                persisted => persisted,
            }
        };
        persisted.map_err(|error| fail(error.error))
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

/// Check that `output` is not any of `inputs`.
fn check_distinct(
    output: &Handle,
    inputs: &[(Rc<Handle>, String)],
    shown: &str,
) -> Result<(), CliError> {
    match inputs.iter().find(|(input, _)| **input == *output) {
        Some((_, name)) => Err(same_file_error(shown, name)),
        None => Ok(()),
    }
}

fn same_file_error(shown: &str, input: &str) -> CliError {
    CliError::runtime(format!(
        "{shown} is the same file as input {input}; refusing to overwrite an input"
    ))
}

/// Check that `path`, after following symbolic links, is not a regular file
/// that is one of `inputs`, and return its metadata when it exists. Unix
/// identifies the file from its metadata, so an unreadable target can still
/// be replaced with `--force`; Windows has to open it.
fn check_path(
    path: &Path,
    inputs: &[(Rc<Handle>, String)],
    shown: &str,
) -> Result<Option<Metadata>, CliError> {
    let Ok(metadata) = fs::metadata(path) else {
        return Ok(None);
    };
    if metadata.is_file() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if let Some((_, name)) = inputs
                .iter()
                .find(|(input, _)| (input.dev(), input.ino()) == (metadata.dev(), metadata.ino()))
            {
                return Err(same_file_error(shown, name));
            }
        }
        #[cfg(windows)]
        {
            let handle = Handle::from_path(path).map_err(|error| {
                CliError::runtime(format!("cannot open {shown} for identity check: {error}"))
            })?;
            check_distinct(&handle, inputs, shown)?;
        }
    }
    Ok(Some(metadata))
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

/// Validate and open an output endpoint.
pub fn open_output(
    endpoint: &Endpoint,
    force: bool,
    inputs: &[&Input],
) -> Result<Output, CliError> {
    let inputs: Vec<_> = inputs
        .iter()
        .filter_map(|input| Some((Rc::clone(input.identity.as_ref()?), input.name.clone())))
        .collect();
    let path = match endpoint {
        Endpoint::Std => {
            // Rust reopens a closed descriptor 1 as /dev/null at startup, so
            // duplicating it fails only when descriptors are exhausted.
            let stdout = duplicate(io::stdout()).map_err(stdout_error)?;
            let metadata = disk_metadata(&stdout).map_err(stdout_error)?;
            if let Some(handle) = identity(&stdout, metadata.as_ref()).map_err(stdout_error)? {
                check_distinct(&handle, &inputs, "standard output")?;
            }
            return Ok(Output::Stdout(BufWriter::with_capacity(
                OUTPUT_BUFFER,
                stdout,
            )));
        }
        Endpoint::Path(path) => path,
    };
    let shown = format!("output '{}'", path.display());
    if check_path(path, &inputs, &shown)?.is_some_and(|metadata| metadata.is_dir()) {
        return Err(CliError::runtime(format!("{shown} is a directory")));
    }
    if !force && fs::symlink_metadata(path).is_ok() {
        return Err(exists_error(path));
    }
    if path.file_name().is_none() {
        return Err(CliError::runtime(format!("{shown} does not name a file")));
    }
    let directory = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let mut builder = tempfile::Builder::new();
    builder.prefix(STAGED_PREFIX).suffix(STAGED_SUFFIX);
    // A finished PDF gets ordinary permissions, not tempfile's private 0600.
    // On Windows the file inherits the directory's ACL.
    #[cfg(unix)]
    builder.permissions(fs::Permissions::from_mode(0o666));
    let (file, temp) = builder
        .tempfile_in(directory)
        .map_err(|error| {
            CliError::runtime(format!(
                "cannot create a temporary file in '{}': {error}",
                directory.display()
            ))
        })?
        .into_parts();
    Ok(Output::Staged(Staged {
        writer: BufWriter::with_capacity(OUTPUT_BUFFER, file),
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
