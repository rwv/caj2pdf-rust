// SPDX-License-Identifier: MIT

//! The command line, parsed by clap. Arguments stay `OsString` values so
//! that non-UTF-8 paths reach the file system unchanged. clap handles
//! tokens, help and version; [`parse`] then checks the rules clap cannot
//! express and keeps the documented `caj2pdf: error:` messages.

use caj2pdf_core::hnc8::{NativeSymbolGlyph, SymbolFontIdentity};
use clap::error::{ContextKind, ContextValue, ErrorKind};
use clap::{Args, Parser, Subcommand};
use std::ffi::OsString;
use std::path::PathBuf;

/// A command-line input or output: `-` selects standard input or output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Endpoint {
    Std,
    Path(PathBuf),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConvertOptions {
    pub ttkn_response_file: Option<PathBuf>,
    pub no_bookmarks: bool,
    pub allow_damaged: bool,
    pub quiet: bool,
    pub fonts: [Option<PathBuf>; 8],
    /// Directory supplying roles by the fixed names in `FONT_FILES`.
    pub font_dir: Option<PathBuf>,
    pub decoration_char: Option<char>,
    /// Explicit symbols-font glyphs for raw HN-B mode-0 symbol codes.
    pub symbol_glyphs: Vec<NativeSymbolGlyph>,
    /// The symbols font the glyph map was measured on.
    pub symbol_font: Option<SymbolFontIdentity>,
    /// Never search the installed fonts for native C8/HN-B text.
    pub no_system_fonts: bool,
}

/// Fixed `--fonts DIR` file stems, in `ConvertOptions::fonts` role order.
/// Each is looked up with the extensions in [`FONT_EXTENSIONS`].
pub const FONT_FILES: [&str; 8] = [
    "cjk",
    "latin",
    "alternate-latin",
    "decoration",
    "symbols",
    "latin-state3",
    "latin-state28",
    "latin-state31",
];

/// Font file extensions tried for each `--fonts DIR` role, in order. A
/// collection (`.ttc`) supplies its first face.
pub const FONT_EXTENSIONS: [&str; 3] = ["ttf", "otf", "ttc"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    /// Help text rendered by clap for the command or a subcommand.
    Help(String),
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
        pages: bool,
    },
    AddBookmarks {
        outline: Endpoint,
        pdf: Endpoint,
        output: Endpoint,
        force: bool,
    },
}

const USAGE: &str = "caj2pdf INPUT [-o OUTPUT] [--force]
       caj2pdf inspect INPUT [--json] [--bookmarks] [--pages]
       caj2pdf add-bookmarks SOURCE_CAJ INPUT_PDF -o OUTPUT_PDF [--force]";

const ABOUT: &str = "Convert CAJ-family documents to PDF.

Conversion writes INPUT's sibling .pdf file unless -o is given. Use - for
standard input or output; standard input without -o writes to standard output.
Supported inputs: CAJ, KDH, PDF, experimental HN/C8 image pages,
and admitted native C8/HN-B text profiles with installed or given fonts.
HN/C8 uses built-in standard codec states; TEB remains unsupported.
C8/HN-B outlines are unverified: none is written and a warning is shown.";

const AFTER_HELP: &str = "\
Without font options, a native C8/HN-B text document uses the first
installed CJK and Latin fonts from a fixed list (see docs/cli.md) and names
them on standard error. Absent optional font roles and characters a role's
font lacks fall back to the CJK font for CJK-coded characters and to the
Latin font otherwise.

Exit status: 0 on success, 3 for a partial PDF with blank pages,
2 for invalid arguments, 1 for other failures.";

/// The whole command line. A subcommand is recognized only as the first
/// argument; anything else is a conversion.
#[derive(Debug, Parser)]
#[command(
    name = "caj2pdf",
    version,
    about = ABOUT,
    long_about = None,
    override_usage = USAGE,
    after_help = AFTER_HELP,
    args_conflicts_with_subcommands = true,
    subcommand_negates_reqs = true,
    disable_help_subcommand = true,
    propagate_version = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Sub>,
    #[command(flatten)]
    convert: ConvertArgs,
}

#[derive(Debug, Args)]
struct ConvertArgs {
    /// Read the case-sensitive TTKN response (32 hex ASCII bytes) from FILE
    #[arg(long, value_name = "FILE", allow_hyphen_values = true)]
    ttkn_response_file: Option<OsString>,
    /// Document to convert (- for standard input)
    #[arg(value_name = "INPUT", required = true)]
    input: Option<OsString>,
    /// Write the PDF to OUTPUT (- for standard output)
    #[arg(short, long, value_name = "OUTPUT", allow_hyphen_values = true)]
    output: Option<OsString>,
    /// Replace an existing output file (never an input)
    #[arg(short, long, overrides_with = "force")]
    force: bool,
    /// Do not show progress on a terminal or the chosen fonts
    #[arg(short, long, overrides_with = "quiet")]
    quiet: bool,
    /// Replace damaged CAJ pages with blanks; exit 3 if any
    #[arg(long, overrides_with = "allow_damaged")]
    allow_damaged: bool,
    /// Skip outline import (silences the C8/HN-B warning)
    #[arg(long, overrides_with = "no_bookmarks")]
    no_bookmarks: bool,
    /// Native C8/HN-B fonts named cjk, latin, ... (.ttf/.otf/.ttc) in DIR
    #[arg(long, value_name = "DIR", allow_hyphen_values = true)]
    fonts: Option<OsString>,
    /// Native C8/HN-B CJK font (required with --font-latin)
    #[arg(long, value_name = "FILE", allow_hyphen_values = true)]
    font_cjk: Option<OsString>,
    /// Native C8/HN-B ordinary Latin font
    #[arg(long, value_name = "FILE", allow_hyphen_values = true)]
    font_latin: Option<OsString>,
    /// Optional native C8/HN-B alternate Latin font
    #[arg(long, value_name = "FILE", allow_hyphen_values = true)]
    font_alternate_latin: Option<OsString>,
    /// Optional HN-B/C8 state-3 Latin font
    #[arg(long, value_name = "FILE", allow_hyphen_values = true)]
    font_latin_state3: Option<OsString>,
    /// Optional C8 state-28 Latin font
    #[arg(long, value_name = "FILE", allow_hyphen_values = true)]
    font_latin_state28: Option<OsString>,
    /// Optional C8 state-31 Latin font
    #[arg(long, value_name = "FILE", allow_hyphen_values = true)]
    font_latin_state31: Option<OsString>,
    /// Optional HN-B mode-0 semantic symbol font
    #[arg(long, value_name = "FILE", allow_hyphen_values = true)]
    font_symbols: Option<OsString>,
    /// Optional native C8/HN-B decoration font
    #[arg(long, value_name = "FILE", allow_hyphen_values = true)]
    font_decoration: Option<OsString>,
    /// Decoration alias (default: ►; not document text)
    #[arg(long, value_name = "CHAR", allow_hyphen_values = true)]
    decoration_char: Option<OsString>,
    /// Draw the symbols-font glyph CHAR (or U+XXXX) for HN-B mode-0 raw CODE (hex); repeatable
    #[arg(long, value_name = "CODE=CHAR", allow_hyphen_values = true)]
    symbol_glyph: Vec<OsString>,
    /// Require the symbols font NAME (PostScript) with head checkSumAdjustment CHECKSUM (8 hex digits)
    #[arg(long, value_name = "NAME:CHECKSUM", allow_hyphen_values = true)]
    symbol_font_identity: Option<OsString>,
    /// Do not search installed fonts for native C8/HN-B text
    #[arg(long, overrides_with = "no_system_fonts")]
    no_system_fonts: bool,
}

#[derive(Debug, Subcommand)]
enum Sub {
    /// Print document metadata
    #[command(
        override_usage = "caj2pdf inspect INPUT [--json] [--bookmarks] [--pages]",
        long_about = None
    )]
    Inspect {
        /// Document to inspect (- for standard input)
        #[arg(value_name = "INPUT")]
        input: OsString,
        /// Print one JSON object (schema_version 1) instead of text
        #[arg(long, overrides_with = "json")]
        json: bool,
        /// Include the bookmark hierarchy and page destinations
        #[arg(long, overrides_with = "bookmarks")]
        bookmarks: bool,
        /// Add a structure-only report; it never contains document text, titles or pixels
        #[arg(long, overrides_with = "pages")]
        pages: bool,
    },
    /// Copy INPUT_PDF to OUTPUT_PDF with the outline of SOURCE_CAJ added
    #[command(
        override_usage = "caj2pdf add-bookmarks SOURCE_CAJ INPUT_PDF -o OUTPUT_PDF [--force]",
        long_about = None,
        after_help = "INPUT_PDF is never modified. OUTPUT_PDF is required and must differ from both\n\
                      inputs; - writes to standard output. At most one input may be -."
    )]
    AddBookmarks {
        /// CAJ document whose outline is copied (- for standard input)
        #[arg(value_name = "SOURCE_CAJ")]
        source: OsString,
        /// PDF to copy (- for standard input)
        #[arg(value_name = "INPUT_PDF")]
        pdf: OsString,
        /// Write the PDF to OUTPUT_PDF
        #[arg(
            short,
            long,
            value_name = "OUTPUT_PDF",
            required = true,
            allow_hyphen_values = true
        )]
        output: Option<OsString>,
        /// Replace an existing output file (never an input)
        #[arg(short, long, overrides_with = "force")]
        force: bool,
    },
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

/// A font file or directory: a nonempty path, never standard input.
fn font_path(value: Option<OsString>, error: &str) -> Result<Option<PathBuf>, String> {
    match value {
        Some(value) if value.is_empty() || value == "-" => Err(error.to_owned()),
        value => Ok(value.map(PathBuf::from)),
    }
}

fn decoration_char(value: Option<OsString>) -> Result<Option<char>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value
        .into_string()
        .map_err(|_| "decoration character must be Unicode")?;
    let mut chars = value.chars();
    let character = chars
        .next()
        .filter(|c| u32::from(*c) <= 0xffff)
        .ok_or("decoration character must be one BMP Unicode scalar")?;
    if chars.next().is_some() {
        return Err("--decoration-char requires one character and may appear only once".into());
    }
    Ok(Some(character))
}

/// `CODE=CHAR`: four hexadecimal digits and one BMP character, given
/// literally or as `U+XXXX`. The core checks codes, duplicates and the font
/// for native documents, as for JavaScript.
fn symbol_glyph(value: OsString) -> Result<NativeSymbolGlyph, String> {
    const ERROR: &str =
        "--symbol-glyph requires CODE=CHAR: four hex digits and one BMP character or U+XXXX";
    let hex = |digits: &str| {
        u16::from_str_radix(digits, 16)
            .ok()
            .filter(|_| digits.len() == 4 && digits.bytes().all(|b| b.is_ascii_hexdigit()))
    };
    let value = value.into_string().map_err(|_| ERROR)?;
    let (code, glyph) = value.split_once('=').ok_or(ERROR)?;
    let mut chars = glyph.chars();
    let glyph = match glyph.strip_prefix("U+") {
        Some(digits) => hex(digits).and_then(|value| char::from_u32(value.into())),
        None => chars.next().filter(|_| chars.next().is_none()),
    };
    match (hex(code), glyph.filter(|c| u32::from(*c) <= 0xffff)) {
        (Some(code), Some(glyph)) => Ok(NativeSymbolGlyph { code, glyph }),
        _ => Err(ERROR.into()),
    }
}

/// `NAME:CHECKSUM`: a PostScript name and an eight-digit hexadecimal `head`
/// checkSumAdjustment. The name may itself contain `:`.
fn symbol_font_identity(value: OsString) -> Result<SymbolFontIdentity, String> {
    const ERROR: &str =
        "--symbol-font-identity requires NAME:CHECKSUM: a PostScript name and eight hex digits";
    let value = value.into_string().map_err(|_| ERROR)?;
    let (name, checksum) = value.rsplit_once(':').ok_or(ERROR)?;
    if !SymbolFontIdentity::is_valid_postscript_name(name)
        || checksum.len() != 8
        || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(ERROR.into());
    }
    Ok(SymbolFontIdentity {
        checksum_adjustment: u32::from_str_radix(checksum, 16).map_err(|_| ERROR)?,
        postscript_name: name.to_owned(),
    })
}

impl ConvertArgs {
    fn into_command(self) -> Result<Command, String> {
        const FONT: &str = "font resources require a nonempty path, not standard input";
        let fonts = [
            self.font_cjk,
            self.font_latin,
            self.font_alternate_latin,
            self.font_decoration,
            self.font_symbols,
            self.font_latin_state3,
            self.font_latin_state28,
            self.font_latin_state31,
        ];
        let mut options = ConvertOptions {
            ttkn_response_file: font_path(
                self.ttkn_response_file,
                "--ttkn-response-file requires a nonempty file path, not standard input",
            )?,
            no_bookmarks: self.no_bookmarks,
            allow_damaged: self.allow_damaged,
            quiet: self.quiet,
            font_dir: font_path(
                self.fonts,
                "--fonts requires a nonempty directory path, not standard input",
            )?,
            decoration_char: decoration_char(self.decoration_char)?,
            symbol_glyphs: self
                .symbol_glyph
                .into_iter()
                .map(symbol_glyph)
                .collect::<Result<_, _>>()?,
            symbol_font: self
                .symbol_font_identity
                .map(symbol_font_identity)
                .transpose()?,
            no_system_fonts: self.no_system_fonts,
            ..ConvertOptions::default()
        };
        for (role, path) in options.fonts.iter_mut().zip(fonts) {
            *role = font_path(path, FONT)?;
        }
        if options.font_dir.is_none()
            && options.fonts.iter().any(Option::is_some)
            && options.fonts[..2].iter().any(Option::is_none)
        {
            return Err(
                "native C8/HN-B fonts require --font-cjk and --font-latin, or --fonts DIR".into(),
            );
        }
        if options.decoration_char.is_some()
            && options.fonts[3].is_none()
            && options.font_dir.is_none()
        {
            return Err("--decoration-char requires --font-decoration or --fonts DIR".into());
        }
        if !options.symbol_glyphs.is_empty()
            && options.fonts[4].is_none()
            && options.font_dir.is_none()
        {
            return Err("--symbol-glyph requires --font-symbols or --fonts DIR".into());
        }
        if options.symbol_font.is_some() && options.symbol_glyphs.is_empty() {
            return Err("--symbol-font-identity requires --symbol-glyph".into());
        }
        Ok(Command::Convert {
            input: endpoint(self.input.expect("clap requires INPUT"))?,
            output: self.output.map(endpoint).transpose()?,
            force: self.force,
            options: Box::new(options),
        })
    }
}

impl Sub {
    fn into_command(self) -> Result<Command, String> {
        Ok(match self {
            Self::Inspect {
                input,
                json,
                bookmarks,
                pages,
            } => Command::Inspect {
                input: endpoint(input)?,
                json,
                bookmarks,
                pages,
            },
            Self::AddBookmarks {
                source,
                pdf,
                output,
                force,
            } => {
                let (outline, pdf) = (endpoint(source)?, endpoint(pdf)?);
                if outline == Endpoint::Std && pdf == Endpoint::Std {
                    return Err("only one input can be read from standard input".to_owned());
                }
                Command::AddBookmarks {
                    outline,
                    pdf,
                    output: endpoint(output.expect("clap requires OUTPUT_PDF"))?,
                    force,
                }
            }
        })
    }
}

fn context(error: &clap::Error, kind: ContextKind) -> Vec<&str> {
    match error.get(kind) {
        Some(ContextValue::String(value)) => vec![value],
        Some(ContextValue::Strings(values)) => values.iter().map(String::as_str).collect(),
        _ => Vec::new(),
    }
}

/// The usage message for a clap error, worded as the documented
/// diagnostics. clap names an option as `--output <OUTPUT>`.
fn usage_message(error: &clap::Error) -> String {
    let args = context(error, ContextKind::InvalidArg);
    let arg = args.first().copied().unwrap_or_default();
    let option = arg.split(' ').next().unwrap_or_default();
    match error.kind() {
        ErrorKind::UnknownArgument if arg.len() > 1 && arg.starts_with('-') => {
            format!("unrecognized option '{arg}'")
        }
        ErrorKind::UnknownArgument => format!("unexpected argument '{arg}'"),
        // An extra positional argument after INPUT is reported as a
        // subcommand that cannot follow it.
        ErrorKind::ArgumentConflict if error.get(ContextKind::InvalidSubcommand).is_some() => {
            let extra = context(error, ContextKind::InvalidSubcommand);
            format!("unexpected argument '{}'", extra[0])
        }
        ErrorKind::MissingRequiredArgument => if args
            .iter()
            .any(|arg| *arg == "<SOURCE_CAJ>" || *arg == "<INPUT_PDF>")
        {
            "add-bookmarks requires SOURCE_CAJ and INPUT_PDF"
        } else if args.iter().any(|arg| arg.starts_with("--output")) {
            "add-bookmarks requires -o OUTPUT_PDF"
        } else {
            "missing INPUT"
        }
        .to_owned(),
        ErrorKind::ArgumentConflict if context(error, ContextKind::PriorArg) == [arg] => {
            match option {
                "--output" => "the output option was given more than once".to_owned(),
                "--decoration-char" => {
                    "--decoration-char requires one character and may appear only once".to_owned()
                }
                _ => format!("option '{option}' was given more than once"),
            }
        }
        ErrorKind::InvalidValue if context(error, ContextKind::InvalidValue) == [""] => {
            match option {
                "--fonts" => "option '--fonts' requires a directory".to_owned(),
                "--decoration-char" => "--decoration-char requires a character".to_owned(),
                _ if option.starts_with("--font-") => format!("option '{option}' requires a path"),
                _ => format!("option '{option}' requires a value"),
            }
        }
        // clap's own wording for anything else, without its prefix and usage.
        _ => {
            let rendered = error.render().to_string();
            let line = rendered.lines().next().unwrap_or_default();
            line.strip_prefix("error: ").unwrap_or(line).to_owned()
        }
    }
}

/// Parse the arguments after the program name. An error is a usage message.
pub fn parse<I: IntoIterator<Item = OsString>>(args: I) -> Result<Command, String> {
    let args = std::iter::once(OsString::from("caj2pdf")).chain(args);
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => {
            return match error.kind() {
                ErrorKind::DisplayHelp => Ok(Command::Help(error.render().to_string())),
                ErrorKind::DisplayVersion => Ok(Command::Version),
                _ => Err(usage_message(&error)),
            };
        }
    };
    match cli.command {
        Some(sub) => sub.into_command(),
        None => cli.convert.into_command(),
    }
}
