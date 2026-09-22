//! Command-line front end for `uncad`: `<input> [-o <output>]`.
//!
//! Reading only -- with no `-o` it prints a summary, and the `-o` targets are
//! the parsed model as JSON or a rendering of it as SVG/PNG. There is no
//! DWG/DXF output.

use std::collections::HashMap;
use std::path::Path;
use std::process::ExitCode;
use uncad::{
    Background, CadDatabase, CropMode, ExportOptions, PngSize, Profile, Rect, Space, ToJsonOptions,
    ToPngOptions, ToSvgOptions,
};

const USAGE: &str = "\
uncad - parse DWG/DXF drawings

Usage:
  uncad <input.dwg>                 print a summary (version, units, entity count per type)
  uncad <input> -o <output.json>    export the parsed model (entities + tables)
  uncad <input> -o <output.svg>     render to SVG
  uncad <input> -o <output.png>     render to PNG (rasterized from the SVG)
  uncad export <input> -o <dir>     write the LLM/VLM package (overview, tiles,
                                    JSON records; see docs/VLM_EXPORT_DESIGN.md)

JSON options:
  --pretty                    indented, multi-line JSON (default: one line)

SVG/PNG options:
  --space <model|paper|all>   which space to render (default: model)
                                model = the drawing itself
                                paper = sheet borders and title blocks
                                all   = everything, in one document
  --crop <mode>               what the image shows (default: auto)
                                auto   = the drawing's extents minus scale
                                         outliers (or the header extents when
                                         those cover more)
                                raw    = every visible entity, outliers included
                                header = $EXTMIN/$EXTMAX as stored
                                x0,y0,x1,y1 = this world rectangle
  --padding <units>           padding on each side in drawing units (default:
                                2 % of the longer side, at least 24 px in a PNG)
  --no-trim                   same as --crop raw (0.2.0's name)
  --include-hidden            draw hidden entities (layers off, frozen or
                                non-plotting, DEFPOINTS, invisible) at 50 %

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
  --lattice <px>              round the image size up to a multiple of this,
                                the model's patch size (default: 28; 0 = off)

Export options (uncad export):
  --profile <name>            claude (default), claude-hires, openai-patch
  --max-levels <n>            deepest zoom level (default: 5)
  --max-tiles <n>             most tiles written (default: 400)
  --text-px <px>              target pixel height of the dominant text (default: 14)
  --shard-kb <kb>             split record files above this size (default: 96)
  --frame-gap <fraction>      entities closer than this fraction of the crop's
                                diagonal are one group; a detached group gets its
                                own frame (default: 0.05)
  --min-frame-entities <n>    a detached group needs this many entities, or one
                                text, to become a frame (default: 20)
  --max-frames <n>            frames written at most (default: 8)
  --svg                       also write drawing.svg
  --full                      also write entities.json (the whole model)
  (--crop, --include-hidden apply too)

Examples:
  uncad drawing.dwg
  uncad export drawing.dwg -o drawing_pkg
  uncad drawing.dwg -o drawing.json --pretty
  uncad drawing.dwg -o drawing.svg
  uncad drawing.dwg -o drawing.svg --space paper
  uncad drawing.dwg -o drawing.png --fit 4000
  uncad drawing.dwg -o drawing.png --scale 2";

struct Args {
    input: Option<String>,
    output: Option<String>,
    space: String,
    crop: String,
    padding: Option<String>,
    lattice: Option<String>,
    include_hidden: bool,
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
        crop: "auto".to_string(),
        padding: None,
        lattice: None,
        include_hidden: false,
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
            "--no-trim" => args.crop = "raw".to_string(),
            "--crop" => {
                i += 1;
                if let Some(v) = argv.get(i) {
                    args.crop = v.clone();
                }
            }
            "--padding" => {
                i += 1;
                args.padding = argv.get(i).cloned();
            }
            "--lattice" => {
                i += 1;
                args.lattice = argv.get(i).cloned();
            }
            "--include-hidden" => args.include_hidden = true,
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
    if argv.first().map(String::as_str) == Some("export") {
        return match run_export(&argv[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("error: {message}");
                ExitCode::FAILURE
            }
        };
    }
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

    let (unsupported, hidden, crop) = match extension.as_str() {
        "json" => {
            let json = db
                .to_json(ToJsonOptions {
                    pretty: args.pretty,
                })
                .map_err(|e| e.to_string())?;
            write_output(output, json.as_bytes())?;
            (Vec::new(), 0, None)
        }
        "svg" => {
            let result = db.to_svg(svg_options(args)?);
            write_output(output, result.svg.as_bytes())?;
            (result.unsupported_types, result.hidden, Some(result.crop))
        }
        "png" => {
            let result = db.to_png(png_options(args)?).map_err(|e| e.to_string())?;
            write_output(output, &result.png)?;
            (result.unsupported_types, result.hidden, Some(result.crop))
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
    if hidden > 0 && !args.include_hidden {
        eprintln!(
            "note: {hidden} hidden entities left out (layers off, frozen or non-plotting, \
             DEFPOINTS, invisible); --include-hidden draws them at 50 %"
        );
    }
    if let Some(crop) = crop {
        if !crop.excluded.is_empty() {
            let mut reasons: Vec<&str> = crop.excluded.iter().map(|e| e.reason.as_str()).collect();
            reasons.sort_unstable();
            reasons.dedup();
            let handles: Vec<&str> = crop
                .excluded
                .iter()
                .take(5)
                .map(|e| e.handle.as_str())
                .collect();
            eprintln!(
                "note: crop {} leaves {} entities outside ({}; handles {}{}); --crop raw keeps everything",
                crop.source.as_str(),
                crop.excluded.len(),
                reasons.join(", "),
                handles.join(", "),
                if crop.excluded.len() > 5 { ", ..." } else { "" }
            );
        }
    }
    Ok(())
}

fn run_export(argv: &[String]) -> Result<(), String> {
    let args = parse_args(argv);
    if args.help {
        eprintln!("{USAGE}");
        return Ok(());
    }
    let input = args
        .input
        .as_deref()
        .ok_or_else(|| "export needs an input file: uncad export <input> -o <dir>".to_string())?;
    let output = args
        .output
        .as_deref()
        .ok_or_else(|| "export needs an output directory: -o <dir>".to_string())?;
    let mut options = ExportOptions {
        crop: parse_crop(&args.crop)?,
        include_hidden: args.include_hidden,
        source_name: Path::new(input)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned()),
        ..Default::default()
    };
    let mut i = 0;
    while i < argv.len() {
        let next = || {
            argv.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{} needs a value", argv[i]))
        };
        match argv[i].as_str() {
            "--profile" => {
                let name = next()?;
                options.profile = Profile::by_name(&name).ok_or_else(|| {
                    format!("unsupported --profile '{name}' (claude, claude-hires, openai-patch)")
                })?;
                i += 1;
            }
            "--max-levels" => {
                options.max_levels = parse_count("--max-levels", &next()?)?;
                i += 1;
            }
            "--max-tiles" => {
                options.max_tiles = parse_count("--max-tiles", &next()?)? as usize;
                i += 1;
            }
            "--text-px" => {
                options.target_text_px = parse_positive("--text-px", &next()?)?;
                i += 1;
            }
            "--shard-kb" => {
                options.shard_kb = parse_pixels("--shard-kb", &next()?)? as usize;
                i += 1;
            }
            "--frame-gap" => {
                options.frame_gap = parse_non_negative("--frame-gap", &next()?)?;
                i += 1;
            }
            "--min-frame-entities" => {
                options.min_frame_entities =
                    parse_count("--min-frame-entities", &next()?)? as usize;
                i += 1;
            }
            "--max-frames" => {
                options.max_frames = parse_count("--max-frames", &next()?)? as usize;
                i += 1;
            }
            "--svg" => options.svg = true,
            "--full" => options.full = true,
            _ => {}
        }
        i += 1;
    }
    let db = parse_input(input)?;
    let report = uncad::export::export_package(&db, Path::new(output), &options)
        .map_err(|e| e.to_string())?;
    println!(
        "wrote: {} ({} files; overview {}x{} px; {} tiles in {} frames; {} texts, {} dimensions, {} geometry, {} regions, {} block instances)",
        report.dir.display(),
        report.files.len(),
        report.overview.px[0],
        report.overview.px[1],
        report.counts.tiles,
        report.frames.len(),
        report.counts.texts,
        report.counts.dimensions,
        report.counts.geometry,
        report.counts.regions,
        report.counts.blocks
    );
    for warning in &report.warnings {
        eprintln!("warning: {warning}");
    }
    if !report.crop.excluded.is_empty() {
        eprintln!(
            "note: {} entities outside the crop, listed in report.json",
            report.crop.excluded.len()
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
    let padding = match &args.padding {
        Some(value) => Some(parse_non_negative("--padding", value)?),
        None => None,
    };
    Ok(ToSvgOptions {
        space: parse_space(&args.space)?,
        crop: parse_crop(&args.crop)?,
        padding,
        include_hidden: args.include_hidden,
        ..Default::default()
    })
}

fn parse_crop(value: &str) -> Result<CropMode, String> {
    match value {
        "auto" => return Ok(CropMode::Auto),
        "raw" => return Ok(CropMode::Raw),
        "header" => return Ok(CropMode::Header),
        _ => {}
    }
    let numbers: Vec<f64> = value
        .split(',')
        .map(|part| part.trim().parse::<f64>())
        .collect::<Result<_, _>>()
        .map_err(|_| {
            format!("unsupported --crop value '{value}' (auto, raw, header or x0,y0,x1,y1)")
        })?;
    match numbers[..] {
        [x0, y0, x1, y1] if x1 > x0 && y1 > y0 && numbers.iter().all(|n| n.is_finite()) => {
            Ok(CropMode::Fixed(Rect::new(x0, y0, x1, y1)))
        }
        _ => Err(format!(
            "--crop x0,y0,x1,y1 needs four finite numbers with x1 > x0 and y1 > y0 (got '{value}')"
        )),
    }
}

fn parse_count(flag: &str, value: &str) -> Result<u32, String> {
    value
        .parse::<u32>()
        .map_err(|_| format!("{flag} must be a whole number of 0 or more (got '{value}')"))
}

fn parse_non_negative(flag: &str, value: &str) -> Result<f64, String> {
    match value.parse::<f64>() {
        Ok(number) if number >= 0.0 && number.is_finite() => Ok(number),
        _ => Err(format!(
            "{flag} must be a finite number of 0 or more (got '{value}')"
        )),
    }
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
    let lattice = match &args.lattice {
        Some(lattice) => lattice
            .parse::<u32>()
            .map_err(|_| format!("--lattice must be a whole number of pixels (got '{lattice}')"))?,
        None => defaults.lattice,
    };
    Ok(ToPngOptions {
        svg: svg_options(args)?,
        size,
        background,
        stroke_px,
        max_edge,
        lattice,
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
