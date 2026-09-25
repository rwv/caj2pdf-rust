// SPDX-License-Identifier: MIT

//! The `caj2pdf` command. See `docs/cli.md` for its interface.
//!
//! Standard output carries only PDF bytes or the requested inspection report;
//! diagnostics go to standard error. The command does not print progress.

#![forbid(unsafe_code)]

use std::process::ExitCode;

#[cfg(unix)]
mod args;
#[cfg(unix)]
mod document;
#[cfg(unix)]
mod files;
#[cfg(unix)]
mod json;
#[cfg(unix)]
mod report;
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

#[cfg(unix)]
mod cli {
    use crate::CliError;
    use crate::args::{self, Command, Endpoint};
    use crate::files::{open_input, open_output, refuse_terminal};
    use crate::{document, report};
    use caj2pdf_core::Limits;
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
            .map_err(|error| CliError::runtime(format!("cannot write standard output: {error}")))
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
            } => {
                let output = match output {
                    Some(output) => output,
                    None => default_output(&input)?,
                };
                refuse_terminal(&output, io::stdout().is_terminal())?;
                let mut input = open_input(&input, limits.max_input_bytes)?;
                let mut output = open_output(&output, force, &[&input])?;
                document::convert(&mut input, output.writer(), &limits)?;
                output.commit()
            }
            Command::Inspect {
                input,
                json,
                bookmarks,
            } => {
                let mut input = open_input(&input, limits.max_input_bytes)?;
                let info = document::inspect(&mut input, &limits)?;
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
        run(args::parse(std::env::args_os().skip(1)).map_err(CliError::usage)?)
    }
}

#[cfg(unix)]
fn main() -> ExitCode {
    match cli::main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("caj2pdf: error: {}", error.message);
            if error.code == 2 {
                eprintln!("Try 'caj2pdf --help' for more information.");
            }
            ExitCode::from(error.code)
        }
    }
}

#[cfg(not(unix))]
fn main() -> ExitCode {
    eprintln!("caj2pdf: error: this command-line interface supports Unix-like systems only");
    ExitCode::FAILURE
}
