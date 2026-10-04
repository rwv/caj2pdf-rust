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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConvertOptions {
    pub qm_states: Option<PathBuf>,
    pub mq_states: Option<PathBuf>,
    pub no_bookmarks: bool,
    pub quiet: bool,
    pub fonts: [Option<PathBuf>; 8],
    pub decoration_char: Option<char>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    Help(Topic),
    Version,
    Convert {
        input: Endpoint,
        output: Option<Endpoint>,
        force: bool,
        options: Box<ConvertOptions>,
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
Supported inputs: CAJ, KDH, PDF, experimental HN/C8 image pages,
and admitted native C8/HN-B text profiles with explicit fonts.
HN/C8 uses built-in standard codec states; TEB remains unsupported.
C8/HN-B outlines are unverified: none is written and a warning is shown.

Options:
  -o, --output OUTPUT  Write the PDF to OUTPUT (- for standard output)
  -f, --force          Replace an existing output file (never an input)
  -q, --quiet          Do not show progress on a terminal
  --no-bookmarks      Skip outline import (silences the C8/HN-B warning)
  --qm-states FILE    Experimental QM states for HN/C8 type-0 images
  --mq-states FILE    Experimental MQ states for arithmetic JBIG2 images
  --font-cjk FILE     Explicit native C8/HN-B CJK font (requires both Latin roles)
  --font-latin FILE   Explicit native C8/HN-B ordinary Latin font
  --font-alternate-latin FILE  Explicit native C8/HN-B alternate Latin font
  --font-latin-state3 FILE    Optional HN-B/C8 state-3 Latin font
  --font-latin-state28 FILE   Optional C8 state-28 Latin font
  --font-latin-state31 FILE   Optional C8 state-31 Latin font
  --font-symbols FILE         Optional HN-B mode-0 semantic symbol font
  --font-decoration FILE      Optional native C8/HN-B decoration font
  --decoration-char CHAR      Decoration alias (default: ►; not document text)
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

fn set_states(path: &mut Option<PathBuf>, value: OsString) -> Result<(), String> {
    if value.is_empty() || value == "-" {
        return Err("codec state files require a nonempty path, not standard input".into());
    }
    if path.replace(value.into()).is_some() {
        return Err("a codec state option was given more than once".into());
    }
    Ok(())
}

fn font_option(name: &str) -> Option<usize> {
    match name {
        "--font-cjk" => Some(0),
        "--font-latin" => Some(1),
        "--font-alternate-latin" => Some(2),
        "--font-decoration" => Some(3),
        "--font-symbols" => Some(4),
        "--font-latin-state3" => Some(5),
        "--font-latin-state28" => Some(6),
        "--font-latin-state31" => Some(7),
        _ => None,
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
    let mut options = ConvertOptions::default();
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
            "--no-bookmarks" if topic == Topic::Convert => options.no_bookmarks = true,
            "-q" | "--quiet" if topic == Topic::Convert => options.quiet = true,
            "--qm-states" | "--mq-states" if topic == Topic::Convert => {
                let value = args
                    .next()
                    .ok_or_else(|| format!("option '{text}' requires a value"))?;
                let path = if text == "--qm-states" {
                    &mut options.qm_states
                } else {
                    &mut options.mq_states
                };
                set_states(path, value)?;
            }
            _ if topic == Topic::Convert
                && font_option(text.split('=').next().unwrap()).is_some() =>
            {
                let (name, inline) = text
                    .split_once('=')
                    .map_or((text, None), |(name, value)| (name, Some(value)));
                let value = inline
                    .map(OsString::from)
                    .or_else(|| args.next())
                    .ok_or_else(|| format!("option '{name}' requires a path"))?;
                if value.is_empty() || value == "-" {
                    return Err("font resources require a nonempty path, not standard input".into());
                }
                if options.fonts[font_option(name).unwrap()]
                    .replace(value.into())
                    .is_some()
                {
                    return Err(format!("option '{name}' was given more than once"));
                }
            }
            "--decoration-char" if topic == Topic::Convert => {
                let value = args
                    .next()
                    .ok_or("--decoration-char requires a character")?;
                let value = value
                    .to_str()
                    .ok_or("decoration character must be Unicode")?;
                let mut chars = value.chars();
                let character = chars
                    .next()
                    .filter(|c| u32::from(*c) <= 0xffff)
                    .ok_or("decoration character must be one BMP Unicode scalar")?;
                if chars.next().is_some() || options.decoration_char.replace(character).is_some() {
                    return Err(
                        "--decoration-char requires one character and may appear only once".into(),
                    );
                }
            }
            "--json" if !writes => json = true,
            "--bookmarks" if !writes => bookmarks = true,
            _ if topic == Topic::Convert && text.starts_with("--qm-states=") => {
                set_states(&mut options.qm_states, text[12..].into())?;
            }
            _ if topic == Topic::Convert && text.starts_with("--mq-states=") => {
                set_states(&mut options.mq_states, text[12..].into())?;
            }
            _ => match text.strip_prefix("--output=") {
                Some(value) if writes => set_output(&mut output, value.into())?,
                _ => return Err(format!("unrecognized option '{text}'")),
            },
        }
    }

    if options.fonts.iter().any(Option::is_some) && options.fonts[..3].iter().any(Option::is_none) {
        return Err(
            "native C8/HN-B fonts require --font-cjk, --font-latin and --font-alternate-latin"
                .into(),
        );
    }
    if options.decoration_char.is_some() && options.fonts[3].is_none() {
        return Err("--decoration-char requires --font-decoration".into());
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
            options: Box::new(options),
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
