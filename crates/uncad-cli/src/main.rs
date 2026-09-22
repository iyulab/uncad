//! Command-line front end for `uncad`: `<input> [-o <output>]`.
//!
//! Reading only -- with no `-o` it prints a summary, and the `-o` targets are
//! the parsed model as JSON or a rendering of it as SVG/PNG. There is no
//! DWG/DXF output.

use std::collections::HashMap;
use std::path::Path;
use std::process::ExitCode;
use uncad::{Background, CadDatabase, PngSize, Space, ToJsonOptions, ToPngOptions, ToSvgOptions};

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
  --fit <px>                  longest side of the image in pixels (default: 1568,
                                the largest a Claude standard-tier image keeps)
  --ppu <n>                   pixels per drawing unit, instead of --fit
  --scale <factor>            viewBox units times this factor, instead of --fit
                                (0.2.0's sizing; 1 = one pixel per drawing unit)
  --bg <white|transparent>    background (default: white, written as RGB)
  --stroke <px>               stroke width in pixels (default: 1.25)
  --max-edge <px>             refuse images wider or taller than this
                                (default: 8000)

Examples:
  uncad drawing.dwg
  uncad drawing.dwg -o drawing.json --pretty
  uncad drawing.dwg -o drawing.svg
  uncad drawing.dwg -o drawing.svg --space paper
  uncad drawing.dwg -o drawing.png --fit 4000
  uncad drawing.dwg -o drawing.png --scale 2";

struct Args {
    input: Option<String>,
    output: Option<String>,
    space: String,
    outlier_trim: bool,
    fit: Option<String>,
    ppu: Option<String>,
    scale: Option<String>,
    bg: String,
    stroke: Option<String>,
    max_edge: Option<String>,
    pretty: bool,
    help: bool,
}

fn parse_args(argv: &[String]) -> Args {
    let mut args = Args {
        input: None,
        output: None,
        space: "model".to_string(),
        outlier_trim: true,
        fit: None,
        ppu: None,
        scale: None,
        bg: "white".to_string(),
        stroke: None,
        max_edge: None,
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
            "--fit" => {
                i += 1;
                args.fit = argv.get(i).cloned();
            }
            "--ppu" => {
                i += 1;
                args.ppu = argv.get(i).cloned();
            }
            "--scale" => {
                i += 1;
                args.scale = argv.get(i).cloned();
            }
            "--bg" => {
                i += 1;
                if let Some(v) = argv.get(i) {
                    args.bg = v.clone();
                }
            }
            "--stroke" => {
                i += 1;
                args.stroke = argv.get(i).cloned();
            }
            "--max-edge" => {
                i += 1;
                args.max_edge = argv.get(i).cloned();
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
            let result = db.to_png(png_options(args)?).map_err(|e| e.to_string())?;
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

fn png_options(args: &Args) -> Result<ToPngOptions, String> {
    let defaults = ToPngOptions::default();
    // One sizing rule at a time: an explicit --scale or --ppu replaces the
    // default fit; --fit changes the fit's pixel count.
    let size = match (&args.scale, &args.ppu, &args.fit) {
        (Some(scale), _, _) => PngSize::Scale(parse_positive("--scale", scale)?),
        (None, Some(ppu), _) => PngSize::PxPerUnit(parse_positive("--ppu", ppu)?),
        (None, None, Some(fit)) => PngSize::FitLongEdge(parse_pixels("--fit", fit)?),
        (None, None, None) => defaults.size,
    };
    let background = match args.bg.as_str() {
        "white" => Background::White,
        "transparent" => Background::Transparent,
        other => {
            return Err(format!(
                "unsupported --bg value '{other}' (one of white, transparent)"
            ))
        }
    };
    let stroke_px = match &args.stroke {
        Some(stroke) => Some(parse_positive("--stroke", stroke)?),
        None => defaults.stroke_px,
    };
    let max_edge = match &args.max_edge {
        Some(max_edge) => parse_pixels("--max-edge", max_edge)?,
        None => defaults.max_edge,
    };
    Ok(ToPngOptions {
        svg: svg_options(args)?,
        size,
        background,
        stroke_px,
        max_edge,
    })
}

fn parse_positive(flag: &str, value: &str) -> Result<f64, String> {
    match value.parse::<f64>() {
        Ok(number) if number > 0.0 && number.is_finite() => Ok(number),
        _ => Err(format!(
            "{flag} must be a finite number greater than 0 (got '{value}')"
        )),
    }
}

fn parse_pixels(flag: &str, value: &str) -> Result<u32, String> {
    match value.parse::<u32>() {
        Ok(pixels) if pixels > 0 => Ok(pixels),
        _ => Err(format!(
            "{flag} must be a whole number of pixels greater than 0 (got '{value}')"
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
