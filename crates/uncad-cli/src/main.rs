//! Command-line front end for `uncad`: `<input> [-o <output>]`.
//!
//! Reading only -- with no `-o` it prints a summary, and the `-o` targets are
//! the parsed model as JSON or a rendering of it as SVG/PNG. There is no
//! DWG/DXF output.

use std::collections::HashMap;
use std::path::Path;
use std::process::ExitCode;
use uncad::{CadDatabase, Space, ToJsonOptions, ToPngOptions, ToSvgOptions};

const USAGE: &str = "\
uncad - parse DWG/DXF drawings

Usage:
  uncad <input.dwg>                 print a summary (version, units, entity count per type)
  uncad <input> -o <output.json>    export the parsed model (entities + tables)
  uncad <input> -o <output.svg>     render to SVG
  uncad <input> -o <output.png>     render to PNG (rasterized from the SVG)

JSON options:
  --pretty                    indented, multi-line JSON (default: one line)

SVG/PNG options:
  --space <model|paper|all>   which space to render (default: model)
                                model = the drawing itself
                                paper = sheet borders and title blocks
                                all   = everything, in one document
  --no-trim                   keep outlying coordinates in the viewBox instead
                                of trimming to the drawing's main cluster

PNG options:
  --scale <factor>            multiplies the SVG viewBox size (default: 1.0,
                                e.g. 2.0 for twice the resolution)

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
    help: bool,
}

fn parse_args(argv: &[String]) -> Args {
    let mut args = Args {
        input: None,
        output: None,
        space: "model".to_string(),
        outlier_trim: true,
        scale: "1".to_string(),
        pretty: false,
        help: false,
    };
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "-o" | "--output" => {
                i += 1;
                args.output = argv.get(i).cloned();
            }
            "--space" => {
                i += 1;
                if let Some(v) = argv.get(i) {
                    args.space = v.clone();
                }
            }
            "--no-trim" => args.outlier_trim = false,
            "--scale" => {
                i += 1;
                if let Some(v) = argv.get(i) {
                    args.scale = v.clone();
                }
            }
            "--pretty" => args.pretty = true,
            "-h" | "--help" => args.help = true,
            other if args.input.is_none() => args.input = Some(other.to_string()),
            _ => {}
        }
        i += 1;
    }
    args
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = parse_args(&argv);

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

    let Some(output) = args.output.as_deref() else {
        print_summary(input, &db);
        return Ok(());
    };

    let extension = Path::new(output)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let unsupported = match extension.as_str() {
        "json" => {
            let json = db
                .to_json(ToJsonOptions {
                    pretty: args.pretty,
                })
                .map_err(|e| e.to_string())?;
            write_output(output, json.as_bytes())?;
            Vec::new()
        }
        "svg" => {
            let result = db.to_svg(svg_options(args)?);
            write_output(output, result.svg.as_bytes())?;
            result.unsupported_types
        }
        "png" => {
            let result = db
                .to_png(ToPngOptions {
                    svg: svg_options(args)?,
                    scale: parse_scale(&args.scale)?,
                })
                .map_err(|e| e.to_string())?;
            write_output(output, &result.png)?;
            result.unsupported_types
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
    Ok(())
}

/// `uncad::parse()` reports a nonexistent path or a wrong extension as a bare
/// LibreDWG error code, which is accurate but not something a user can act on
/// without cross-referencing dwg.h. The obvious cases are checked here first so
/// the message says what is actually wrong.
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
        outlier_trim: args.outlier_trim,
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
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for e in &db.entities {
        *counts.entry(e.type_name()).or_insert(0) += 1;
    }
    // Most common first, then alphabetically, so the same drawing always
    // prints the same order (the counts come out of a HashMap).
    let mut entries: Vec<_> = counts.into_iter().collect();
    entries.sort_by_key(|&(type_name, count)| (std::cmp::Reverse(count), type_name));

    println!("file: {input}");
    println!(
        "version: {} (codepage {})",
        db.header.version, db.header.codepage_name
    );
    println!(
        "units: {} (INSUNITS {})",
        db.header.units.name, db.header.insunits
    );
    println!("entities: {}", db.entities.len());
    for (type_name, count) in entries {
        println!("  {type_name}: {count}");
    }
}
