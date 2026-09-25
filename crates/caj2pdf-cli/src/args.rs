// SPDX-License-Identifier: MIT

//! Hand-written argument parser. Arguments stay `OsString` values so that
//! non-UTF-8 Linux paths reach the file system unchanged.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

/// A command-line input or output: `-` selects standard input or output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Endpoint {
    Std,
    Path(PathBuf),
}

/// Which help text to print.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Topic {
    Convert,
    Inspect,
    AddBookmarks,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    Help(Topic),
    Version,
    Convert {
        input: Endpoint,
        output: Option<Endpoint>,
        force: bool,
    },
    Inspect {
        input: Endpoint,
        json: bool,
        bookmarks: bool,
    },
    AddBookmarks {
        outline: Endpoint,
        pdf: Endpoint,
        output: Endpoint,
        force: bool,
    },
}

pub const MAIN_HELP: &str = "\
Convert CAJ-family documents to PDF.

Usage:
  caj2pdf INPUT [-o OUTPUT] [--force]
  caj2pdf inspect INPUT [--json] [--bookmarks]
  caj2pdf add-bookmarks SOURCE_CAJ INPUT_PDF -o OUTPUT_PDF [--force]

Conversion writes INPUT's sibling .pdf file unless -o is given. Use - for
standard input or output; standard input without -o writes to standard output.
Supported inputs: CAJ, KDH, and PDF. HN and C8 are recognized but cannot be
converted yet; TEB is recognized and unsupported.

Options:
  -o, --output OUTPUT  Write the PDF to OUTPUT (- for standard output)
  -f, --force          Replace an existing output file (never an input)
  -h, --help           Print help (also: caj2pdf COMMAND --help)
  -V, --version        Print version

Exit status: 0 on success, 2 for invalid arguments, 1 for other failures.
";

pub const INSPECT_HELP: &str = "\
Print document metadata.

Usage:
  caj2pdf inspect INPUT [--json] [--bookmarks]

Options:
  --json       Print one JSON object (schema_version 1) instead of text
  --bookmarks  Include the bookmark hierarchy and page destinations
  -h, --help   Print help
";

pub const ADD_BOOKMARKS_HELP: &str = "\
Copy INPUT_PDF to OUTPUT_PDF with the outline of SOURCE_CAJ added.

Usage:
  caj2pdf add-bookmarks SOURCE_CAJ INPUT_PDF -o OUTPUT_PDF [--force]

INPUT_PDF is never modified. OUTPUT_PDF is required and must differ from both
inputs; - writes to standard output. At most one input may be -.

Options:
  -o, --output OUTPUT_PDF  Write the PDF to OUTPUT_PDF
  -f, --force              Replace an existing output file (never an input)
  -h, --help               Print help
";

impl Topic {
    pub fn help(self) -> &'static str {
        match self {
            Self::Convert => MAIN_HELP,
            Self::Inspect => INSPECT_HELP,
            Self::AddBookmarks => ADD_BOOKMARKS_HELP,
        }
    }
}

fn endpoint(value: OsString) -> Result<Endpoint, String> {
    if value.is_empty() {
        Err("empty path argument".to_owned())
    } else if value == "-" {
        Ok(Endpoint::Std)
    } else {
        Ok(Endpoint::Path(PathBuf::from(value)))
    }
}

fn is_option(arg: &OsStr) -> bool {
    let bytes = arg.as_encoded_bytes();
    bytes.len() > 1 && bytes[0] == b'-'
}

fn set_output(output: &mut Option<OsString>, value: OsString) -> Result<(), String> {
    if output.replace(value).is_some() {
        Err("the output option was given more than once".to_owned())
    } else {
        Ok(())
    }
}

/// Parse the arguments after the program name. An error is a usage message.
pub fn parse<I: IntoIterator<Item = OsString>>(args: I) -> Result<Command, String> {
    let mut args = args.into_iter().peekable();
    let topic = match args.peek().and_then(|arg| arg.to_str()) {
        Some("inspect") => Topic::Inspect,
        Some("add-bookmarks") => Topic::AddBookmarks,
        _ => Topic::Convert,
    };
    if topic != Topic::Convert {
        args.next();
    }
    let writes = topic != Topic::Inspect;
    let (mut output, mut force, mut json, mut bookmarks) = (None, false, false, false);
    let mut positionals = Vec::new();
    let mut only_positionals = false;
    while let Some(arg) = args.next() {
        if only_positionals || !is_option(&arg) {
            positionals.push(arg);
            continue;
        }
        let text = arg
            .to_str()
            .ok_or_else(|| format!("unrecognized option '{}'", arg.to_string_lossy()))?;
        match text {
            "--" => only_positionals = true,
            "-h" | "--help" => return Ok(Command::Help(topic)),
            "-V" | "--version" => return Ok(Command::Version),
            "-o" | "--output" if writes => {
                let value = args
                    .next()
                    .ok_or_else(|| format!("option '{text}' requires a value"))?;
                set_output(&mut output, value)?;
            }
            "-f" | "--force" if writes => force = true,
            "--json" if !writes => json = true,
            "--bookmarks" if !writes => bookmarks = true,
            _ => match text.strip_prefix("--output=") {
                Some(value) if writes => set_output(&mut output, value.into())?,
                _ => return Err(format!("unrecognized option '{text}'")),
            },
        }
    }

    let expected = if topic == Topic::AddBookmarks { 2 } else { 1 };
    if let Some(extra) = positionals.get(expected) {
        return Err(format!("unexpected argument '{}'", extra.to_string_lossy()));
    }
    if positionals.len() < expected {
        return Err(match topic {
            Topic::AddBookmarks => "add-bookmarks requires SOURCE_CAJ and INPUT_PDF",
            _ => "missing INPUT",
        }
        .to_owned());
    }
    let mut positionals = positionals.into_iter().map(endpoint);
    let input = positionals.next().expect("one positional argument")?;
    let output = output.map(endpoint).transpose()?;
    Ok(match topic {
        Topic::Convert => Command::Convert {
            input,
            output,
            force,
        },
        Topic::Inspect => Command::Inspect {
            input,
            json,
            bookmarks,
        },
        Topic::AddBookmarks => {
            let pdf = positionals.next().expect("two positional arguments")?;
            if input == Endpoint::Std && pdf == Endpoint::Std {
                return Err("only one input can be read from standard input".to_owned());
            }
            Command::AddBookmarks {
                outline: input,
                pdf,
                output: output.ok_or("add-bookmarks requires -o OUTPUT_PDF")?,
                force,
            }
        }
    })
}
