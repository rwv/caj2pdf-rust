// SPDX-License-Identifier: MIT

//! The `caj2pdf` command. See `docs/cli.md` for its interface.
//!
//! Standard output carries only PDF bytes or the requested inspection report;
//! diagnostics, including skipped-bookmark warnings, go to standard error.
//! Conversion shows input progress there only when standard error is a
//! terminal and `--quiet` is not given.

#![forbid(unsafe_code)]

use std::process::ExitCode;

#[cfg(any(unix, windows))]
mod args;
#[cfg(any(unix, windows))]
mod document;
#[cfg(any(unix, windows))]
mod files;
#[cfg(any(unix, windows))]
mod hnc8;
#[cfg(any(unix, windows))]
mod json;
#[cfg(any(unix, windows))]
mod progress;
#[cfg(any(unix, windows))]
mod report;
#[cfg(any(unix, windows))]
mod signals;
#[cfg(all(test, unix))]
mod tests;

/// A failure with its exit status: 2 for usage errors, 1 otherwise.
#[derive(Debug, Eq, PartialEq)]
pub struct CliError {
    pub code: u8,
    pub message: String,
}

impl CliError {
    pub fn usage(message: impl Into<String>) -> Self {
        Self {
            code: 2,
            message: message.into(),
        }
    }

    pub fn runtime(message: impl Into<String>) -> Self {
        Self {
            code: 1,
            message: message.into(),
        }
    }
}

#[cfg(any(unix, windows))]
mod cli {
    use crate::CliError;
    use crate::args::{self, Command, Endpoint};
    use crate::files::{open_input, open_output, refuse_terminal, stdout_error};
    use crate::{document, report};
    use caj2pdf_core::{Limits, hnc8::OutlineReport};
    use std::io::{self, IsTerminal, Write};

    /// The output used when `-o` is absent: stdout for stdin, otherwise the
    /// input's sibling `.pdf`, which must not be the input path itself.
    pub fn default_output(input: &Endpoint) -> Result<Endpoint, CliError> {
        match input {
            Endpoint::Std => Ok(Endpoint::Std),
            Endpoint::Path(path) => {
                let output = path.with_extension("pdf");
                if output == *path {
                    Err(CliError::usage(format!(
                        "'{}' already has a .pdf name; give a distinct output with -o OUTPUT (or -o - for standard output)",
                        path.display()
                    )))
                } else {
                    Ok(Endpoint::Path(output))
                }
            }
        }
    }

    fn print(
        write: impl FnOnce(&mut io::StdoutLock<'_>) -> io::Result<()>,
    ) -> Result<(), CliError> {
        let mut stdout = io::stdout().lock();
        write(&mut stdout)
            .and_then(|()| stdout.flush())
            .map_err(stdout_error)
    }

    /// Report skipped HN-A bookmarks; a failed diagnostic write is ignored
    /// like the error diagnostic, so it cannot change the exit status.
    fn warn(outline: &OutlineReport) {
        let _ = report::write_warnings(&mut io::stderr().lock(), outline);
    }

    pub fn run(command: Command) -> Result<(), CliError> {
        let limits = Limits::default();
        match command {
            Command::Help(topic) => print(|out| out.write_all(topic.help().as_bytes())),
            Command::Version => print(|out| writeln!(out, "caj2pdf {}", env!("CARGO_PKG_VERSION"))),
            Command::Convert {
                input,
                output,
                force,
                options,
            } => {
                let output = match output {
                    Some(output) => output,
                    None => default_output(&input)?,
                };
                refuse_terminal(&output, io::stdout().is_terminal())?;
                let mut input = open_input(&input, limits.max_input_bytes)?;
                let mut resources = crate::hnc8::Resources::load(&options, &limits)?;
                let mut protected = vec![&input];
                protected.extend(resources.inputs.iter());
                let mut output = open_output(&output, force, &protected)?;
                let mut terminal = (!options.quiet && io::stderr().is_terminal()).then(io::stderr);
                let outline = document::convert(
                    &mut input,
                    output.writer(),
                    &limits,
                    &mut resources,
                    !options.no_bookmarks,
                    terminal.as_mut().map(|err| err as &mut dyn io::Write),
                )?;
                output.commit()?;
                warn(&outline);
                Ok(())
            }
            Command::Inspect {
                input,
                json,
                bookmarks,
            } => {
                let mut input = open_input(&input, limits.max_input_bytes)?;
                let info = document::inspect(&mut input, &limits)?;
                warn(&info.outline);
                print(|out| {
                    if json {
                        report::write_json(out, &info, bookmarks)
                    } else {
                        report::write_text(out, &info, bookmarks)
                    }
                })
            }
            Command::AddBookmarks {
                outline,
                pdf,
                output,
                force,
            } => {
                refuse_terminal(&output, io::stdout().is_terminal())?;
                let mut outline = open_input(&outline, limits.max_input_bytes)?;
                let mut pdf = open_input(&pdf, limits.max_input_bytes)?;
                let mut output = open_output(&output, force, &[&outline, &pdf])?;
                document::add_bookmarks(&mut outline, &mut pdf, output.writer(), &limits)?;
                output.commit()
            }
        }
    }

    pub fn main() -> Result<(), CliError> {
        let command = args::parse(std::env::args_os().skip(1)).map_err(CliError::usage)?;
        crate::signals::install().map_err(|error| CliError::runtime(error.to_string()))?;
        run(command)
    }
}

#[cfg(any(unix, windows))]
fn main() -> ExitCode {
    use std::io::Write;

    match cli::main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // A failure to write the diagnostic must not replace the status.
            let hint = if error.code == 2 {
                "Try 'caj2pdf --help' for more information.\n"
            } else {
                ""
            };
            let _ = write!(
                std::io::stderr(),
                "caj2pdf: error: {}\n{hint}",
                error.message
            );
            ExitCode::from(error.code)
        }
    }
}

#[cfg(not(any(unix, windows)))]
fn main() -> ExitCode {
    eprintln!("caj2pdf: error: this target has no native command-line adapter; use the WASM API");
    ExitCode::FAILURE
}
