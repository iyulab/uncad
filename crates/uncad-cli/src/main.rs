//! Command-line front end for `uncad`: `<input> [-o <output>]`.
//!
//! Reading only -- with no `-o` it prints a summary, and the `-o` targets are
//! the parsed model as JSON or a rendering of it as SVG/PNG. There is no
//! DWG/DXF output.

use iron_render_cad::{to_png, to_svg, Crop, PngSize, Space, ToPngOptions, ToSvgOptions};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;
use uncad::{CadDatabase, ToJsonOptions};

const USAGE: &str = "\
uncad - parse DWG/DXF drawings

Usage:
  uncad <input.dwg>                 print a summary (entity count per type)
  uncad <input> -o <output.json>    export the parsed model (entities + tables)
  uncad <input> -o <output.svg>     render to SVG
  uncad <input> -o <output.png>     render to PNG (rasterized from the SVG)

JSON options:
  --pretty                    indented, multi-line JSON (default: one line)

SVG/PNG options:
  --include-hidden            draw entities hidden by their layer or flag (off,
                                frozen, non-plotting, DEFPOINTS, invisible)
  --space <model|paper|all>   which space to render (default: model)
                                model = the drawing itself
                                paper = sheet borders and title blocks
                                all   = everything, in one document
  --no-trim                   keep outlying coordinates in the viewBox instead
                                of trimming to the drawing's main cluster

PNG options:
  --scale <factor>            multiplies the SVG viewBox size (default: 1.0,
                                e.g. 2.0 for twice the resolution)

Other options:
  -h, --help                  this text
  -V, --version               print the version

Examples:
  uncad drawing.dwg
  uncad drawing.dwg -o drawing.json --pretty
  uncad drawing.dwg -o drawing.svg
  uncad drawing.dwg -o drawing.svg --space paper
  uncad drawing.dwg -o drawing.png --scale 2";

struct Args {
    input: Option<String>,
    output: Option<String>,
    space: String,
    outlier_trim: bool,
    scale: String,
    pretty: bool,
    include_hidden: bool,
    help: bool,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut args = Args {
        input: None,
        output: None,
        space: "model".to_string(),
        outlier_trim: true,
        scale: "1".to_string(),
        pretty: false,
        include_hidden: false,
        help: false,
    };
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "-o" | "--output" | "--space" | "--scale" => {
                let flag = argv[i].as_str();
                i += 1;
                let value = argv
                    .get(i)
                    .cloned()
                    .ok_or_else(|| format!("{flag} needs a value"))?;
                match flag {
                    "--space" => args.space = value,
                    "--scale" => args.scale = value,
                    _ => args.output = Some(value),
                }
            }
            "--no-trim" => args.outlier_trim = false,
            "--pretty" => args.pretty = true,
            "--include-hidden" => args.include_hidden = true,
            "-h" | "--help" => args.help = true,
            // Answered by `main` before any parsing.
            "-V" | "--version" => {}
            flag if flag.starts_with('-') && flag.len() > 1 => {
                return Err(format!("unknown option '{flag}' (see --help)"));
            }
            positional => {
                if let Some(first) = &args.input {
                    return Err(format!(
                        "unexpected argument '{positional}': the input is already '{first}'"
                    ));
                }
                args.input = Some(positional.to_string());
            }
        }
        i += 1;
    }
    Ok(args)
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.iter().any(|a| a == "-V" || a == "--version") {
        println!("uncad {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    let args = match parse_args(&argv) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };

    if args.help || args.input.is_none() {
        eprintln!("{USAGE}");
        return if args.help {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }

    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Every failure path funnels back here as a message `main` prints with one
/// `error:` prefix.
fn run(args: &Args) -> Result<(), String> {
    let input = args.input.as_deref().expect("checked by the caller");
    let db = parse_input(input)?;
    if !db.read_diagnostics.is_clean() {
        eprintln!(
            "warning: LibreDWG read '{input}' with non-fatal problems ({}); \
             objects it could not decode are missing from the result",
            db.read_diagnostics.warnings.join(", ")
        );
    }

    let Some(output) = args.output.as_deref() else {
        print_summary(input, &db);
        return Ok(());
    };

    let extension = Path::new(output)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let (unsupported, empty_blocks) = match extension.as_str() {
        "json" => {
            let json = db
                .to_json(ToJsonOptions {
                    pretty: args.pretty,
                })
                .map_err(|e| e.to_string())?;
            write_output(output, json.as_bytes())?;
            (Vec::new(), Vec::new())
        }
        "svg" => {
            let result = to_svg(&db, svg_options(args)?);
            write_output(output, result.svg.as_bytes())?;
            (result.unsupported_types, result.empty_blocks)
        }
        "png" => {
            let result = to_png(
                &db,
                ToPngOptions {
                    svg: svg_options(args)?,
                    size: PngSize::Scale(parse_scale(&args.scale)?),
                    ..ToPngOptions::default()
                },
            )
            .map_err(|e| e.to_string())?;
            write_output(output, &result.png)?;
            (result.unsupported_types, result.empty_blocks)
        }
        other => {
            return Err(format!(
                "unsupported output extension '.{other}' (only .json, .svg, .png)"
            ))
        }
    };

    println!("wrote: {output}");
    if !unsupported.is_empty() {
        eprintln!(
            "warning: left out of the image, unsupported entity types: {}",
            unsupported.join(", ")
        );
    }
    if !empty_blocks.is_empty() {
        eprintln!(
            "warning: block references that drew nothing: {}",
            empty_blocks.join(", ")
        );
    }
    Ok(())
}

/// `uncad::parse()` reports a path it cannot read as `ParseError::Io`, and a
/// file that is not a drawing as a bare LibreDWG error code, which is accurate
/// but not something a user can act on without cross-referencing dwg.h. The
/// obvious cases are checked here first so the message says what is actually
/// wrong.
fn parse_input(input: &str) -> Result<CadDatabase, String> {
    match std::fs::metadata(input) {
        Ok(meta) if meta.is_dir() => {
            return Err(format!("input path is a directory, not a file: '{input}'"))
        }
        Err(e) => return Err(format!("cannot open input file '{input}': {e}")),
        Ok(_) => {}
    }
    uncad::parse(input)
        .map_err(|e| format!("could not parse '{input}' ({e}) -- is it a valid DWG/DXF file?"))
}

fn write_output(path: &str, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("cannot write '{path}': {e}"))
}

fn svg_options(args: &Args) -> Result<ToSvgOptions, String> {
    Ok(ToSvgOptions {
        space: parse_space(&args.space)?,
        crop: if args.outlier_trim {
            Crop::Cluster
        } else {
            Crop::Everything
        },
        include_hidden: args.include_hidden,
        ..Default::default()
    })
}

fn parse_space(value: &str) -> Result<Space, String> {
    match value {
        "model" => Ok(Space::Model),
        "paper" => Ok(Space::Paper),
        "all" => Ok(Space::All),
        other => Err(format!(
            "unsupported --space value '{other}' (one of model, paper, all)"
        )),
    }
}

fn parse_scale(value: &str) -> Result<f32, String> {
    match value.parse::<f32>() {
        Ok(scale) if scale > 0.0 && scale.is_finite() => Ok(scale),
        _ => Err(format!(
            "--scale must be a finite number greater than 0 (got '{value}')"
        )),
    }
}

fn print_summary(input: &str, db: &CadDatabase) {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for e in &db.entities {
        *counts.entry(e.type_name()).or_insert(0) += 1;
    }
    // Most common first, then alphabetically.
    let mut entries: Vec<_> = counts.into_iter().collect();
    entries.sort_by_key(|&(type_name, count)| (std::cmp::Reverse(count), type_name));

    println!("file: {input}");
    println!("entities: {}", db.entities.len());
    for (type_name, count) in entries {
        println!("  {type_name}: {count}");
    }
}
