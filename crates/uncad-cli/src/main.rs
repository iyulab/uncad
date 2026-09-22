//! Command-line front end for `uncad`: `<input> [-o <output>]`, plus the
//! `export` subcommand.
//!
//! Reading only -- with no `-o` it prints a summary, the `-o` targets are the
//! parsed model as JSON or a rendering of it as SVG/PNG, and `uncad export
//! <input> -o <dir>` writes the LLM/VLM package. There is no DWG/DXF output.
//!
//! One parser (`parse_args`) owns the flags of both commands, so an option is
//! either understood wherever it stands or refused by name.

use std::collections::HashMap;
use std::path::Path;
use std::process::ExitCode;
use uncad::{
    Background, CadDatabase, CropMode, ExportOptions, Fonts, PngSize, Profile, Rect, Space,
    ToJsonOptions, ToPngOptions, ToSvgOptions,
};

const USAGE: &str = "\
uncad - parse DWG/DXF drawings

Usage:
  uncad <input.dwg>                 print a summary (version, units, entity count per type)
  uncad <input> -o <output.json>    export the parsed model (entities + tables)
  uncad <input> -o <output.svg>     render to SVG
  uncad <input> -o <output.png>     render to PNG (rasterized from the SVG)
  uncad export <input> -o <dir>     write the LLM/VLM package (an overview, a tile
                                    pyramid per detached frame, a composited image
                                    per paper layout, and JSON records; see
                                    docs/VLM_EXPORT_DESIGN.md). Re-exporting into
                                    the same directory replaces the previous package

Both commands:
  -o, --output <path>         where the result goes: the output file above
                                (its extension picks the format) or, under
                                'export', the package directory
  -h, --help                  print this usage

JSON options:
  --pretty                    indented, multi-line JSON (default: one line)

SVG/PNG options:
  --space <model|paper|all>   which space to render (default: model)
                                model = the drawing itself
                                paper = sheet borders and title blocks
                                all   = everything, in one document
  --crop <mode>               what the image shows (default: auto)
                                auto   = the drawing's extents minus scale and
                                         far outliers (or the header extents
                                         when those cover more)
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
  --fonts <which>             bundled (default: the embedded Uncad Sans, the same
                                on every machine) or bundled+system (the host's
                                fonts for characters the bundled face lacks)

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
  --no-sheets                 skip sheets.json and the paper-layout images
  --svg                       also write drawing.svg
  --full                      also write entities.json (the whole model)
  --padding <units>           padding around the overview and each frame's
                                window, in drawing units (default: the
                                automatic 2 %); the sheets keep their own
                                zero padding
  (--crop, --no-trim, --include-hidden and --fonts apply too; --lattice does
   not -- the package's patch size is the profile's)

Examples:
  uncad drawing.dwg
  uncad export drawing.dwg -o drawing_pkg
  uncad drawing.dwg -o drawing.json --pretty
  uncad drawing.dwg -o drawing.svg
  uncad drawing.dwg -o drawing.svg --space paper
  uncad drawing.dwg -o drawing.png --fit 4000
  uncad drawing.dwg -o drawing.png --scale 2";

/// Which command line is being parsed: the flags each one accepts differ,
/// and a flag of the other command is refused with a message naming it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Command {
    /// `uncad <input> [-o <output>]`
    Plain,
    /// `uncad export <input> -o <dir>`
    Export,
}

/// Every option as the raw string it was given; the typed parsers
/// (`svg_options`, `png_options`, `export_options`) turn them into values,
/// so a bad value is reported by the command that uses it.
struct Args {
    input: Option<String>,
    output: Option<String>,
    // Both commands.
    crop: String,
    fonts: String,
    include_hidden: bool,
    padding: Option<String>,
    help: bool,
    // The plain command only.
    space: String,
    lattice: Option<String>,
    fit: Option<String>,
    ppu: Option<String>,
    scale: Option<String>,
    bg: String,
    stroke: Option<String>,
    max_edge: Option<String>,
    pretty: bool,
    // `uncad export` only.
    profile: Option<String>,
    max_levels: Option<String>,
    max_tiles: Option<String>,
    text_px: Option<String>,
    shard_kb: Option<String>,
    frame_gap: Option<String>,
    min_frame_entities: Option<String>,
    max_frames: Option<String>,
    svg: bool,
    full: bool,
    no_sheets: bool,
}

/// One parser for both commands, so a token is either understood or
/// refused: an option no arm matches is an error naming it (before or after
/// the input), a second positional is an error, and an option of the other
/// command is an error naming that command. Nothing falls through silently.
fn parse_args(argv: &[String], command: Command) -> Result<Args, String> {
    let mut args = Args {
        input: None,
        output: None,
        crop: "auto".to_string(),
        fonts: "bundled".to_string(),
        include_hidden: false,
        padding: None,
        help: false,
        space: "model".to_string(),
        lattice: None,
        fit: None,
        ppu: None,
        scale: None,
        bg: "white".to_string(),
        stroke: None,
        max_edge: None,
        pretty: false,
        profile: None,
        max_levels: None,
        max_tiles: None,
        text_px: None,
        shard_kb: None,
        frame_gap: None,
        min_frame_entities: None,
        max_frames: None,
        svg: false,
        full: false,
        no_sheets: false,
    };
    let mut i = 0;
    while i < argv.len() {
        let flag = argv[i].as_str();
        // The value of a valued option is the next token, whatever it looks
        // like (`--crop -5,-5,5,5` is a value, not an option).
        let mut value = || -> Result<String, String> {
            i += 1;
            argv.get(i)
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value (see --help)"))
        };
        match flag {
            "-h" | "--help" => args.help = true,
            "-o" | "--output" => args.output = Some(value()?),
            "--crop" => args.crop = value()?,
            "--no-trim" => args.crop = "raw".to_string(),
            "--fonts" => args.fonts = value()?,
            "--include-hidden" => args.include_hidden = true,
            // The plain command's rendering options.
            "--space" => {
                plain_only(flag, command)?;
                args.space = value()?;
            }
            // Both commands: the plain one pads its viewBox with it, the
            // package its overview and frame windows.
            "--padding" => args.padding = Some(value()?),
            "--lattice" => {
                plain_only(flag, command)?;
                args.lattice = Some(value()?);
            }
            "--fit" => {
                plain_only(flag, command)?;
                args.fit = Some(value()?);
            }
            "--ppu" => {
                plain_only(flag, command)?;
                args.ppu = Some(value()?);
            }
            "--scale" => {
                plain_only(flag, command)?;
                args.scale = Some(value()?);
            }
            "--bg" => {
                plain_only(flag, command)?;
                args.bg = value()?;
            }
            "--stroke" => {
                plain_only(flag, command)?;
                args.stroke = Some(value()?);
            }
            "--max-edge" => {
                plain_only(flag, command)?;
                args.max_edge = Some(value()?);
            }
            "--pretty" => {
                plain_only(flag, command)?;
                args.pretty = true;
            }
            // The package's options.
            "--profile" => {
                export_only(flag, command)?;
                args.profile = Some(value()?);
            }
            "--max-levels" => {
                export_only(flag, command)?;
                args.max_levels = Some(value()?);
            }
            "--max-tiles" => {
                export_only(flag, command)?;
                args.max_tiles = Some(value()?);
            }
            "--text-px" => {
                export_only(flag, command)?;
                args.text_px = Some(value()?);
            }
            "--shard-kb" => {
                export_only(flag, command)?;
                args.shard_kb = Some(value()?);
            }
            "--frame-gap" => {
                export_only(flag, command)?;
                args.frame_gap = Some(value()?);
            }
            "--min-frame-entities" => {
                export_only(flag, command)?;
                args.min_frame_entities = Some(value()?);
            }
            "--max-frames" => {
                export_only(flag, command)?;
                args.max_frames = Some(value()?);
            }
            "--svg" => {
                export_only(flag, command)?;
                args.svg = true;
            }
            "--full" => {
                export_only(flag, command)?;
                args.full = true;
            }
            "--no-sheets" => {
                export_only(flag, command)?;
                args.no_sheets = true;
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown option '{other}' (see --help)"));
            }
            other => match &args.input {
                None => args.input = Some(other.to_string()),
                Some(input) => {
                    return Err(format!(
                        "unexpected argument '{other}': the input is already '{input}' (see --help)"
                    ))
                }
            },
        }
        i += 1;
    }
    Ok(args)
}

/// Refuses a rendering/JSON option under `uncad export`, naming the command
/// it belongs to.
fn plain_only(flag: &str, command: Command) -> Result<(), String> {
    match command {
        Command::Plain => Ok(()),
        Command::Export => Err(format!(
            "{flag} is not an option of 'uncad export'; it belongs to 'uncad <input> -o <output>' (see --help)"
        )),
    }
}

/// Refuses a package option under the plain command, naming `uncad export`.
fn export_only(flag: &str, command: Command) -> Result<(), String> {
    match command {
        Command::Export => Ok(()),
        Command::Plain => Err(format!(
            "{flag} is an option of 'uncad export' (uncad export <input> -o <dir> [options]; see --help)"
        )),
    }
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
    let args = match parse_args(&argv, Command::Plain) {
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

    let Some(output) = args.output.as_deref() else {
        print_summary(input, &db);
        return Ok(());
    };

    let extension = Path::new(output)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let (unsupported, hidden, crop, limits) = match extension.as_str() {
        "json" => {
            let json = db
                .to_json(ToJsonOptions {
                    pretty: args.pretty,
                })
                .map_err(|e| e.to_string())?;
            write_output(output, json.as_bytes())?;
            (Vec::new(), 0, None, Default::default())
        }
        "svg" => {
            let result = db.to_svg(svg_options(args)?);
            write_output(output, result.svg.as_bytes())?;
            (
                result.unsupported_types,
                result.hidden,
                Some(result.crop),
                result.limits,
            )
        }
        "png" => {
            let result = db.to_png(png_options(args)?).map_err(|e| e.to_string())?;
            write_output(output, &result.png)?;
            (
                result.unsupported_types,
                result.hidden,
                Some(result.crop),
                result.limits,
            )
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
    if let Some(summary) = limits.summary() {
        eprintln!("warning: the drawing hit the renderer's robustness limits: {summary}");
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
    let args = parse_args(argv, Command::Export)?;
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
    let options = export_options(&args, input)?;
    let db = parse_input(input)?;
    let report = uncad::export::export_package(&db, Path::new(output), &options)
        .map_err(|e| e.to_string())?;
    println!(
        "wrote: {} ({} files; overview {}x{} px; {} tiles in {} frames; {} sheets; {} texts, {} dimensions, {} geometry, {} regions, {} block instances)",
        report.dir.display(),
        report.files.len(),
        report.overview.px[0],
        report.overview.px[1],
        report.counts.tiles,
        report.frames.len(),
        report.sheets.len(),
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

fn export_options(args: &Args, input: &str) -> Result<ExportOptions, String> {
    let defaults = ExportOptions::default();
    let profile = match &args.profile {
        Some(name) => Profile::by_name(name).ok_or_else(|| {
            format!("unsupported --profile '{name}' (claude, claude-hires, openai-patch)")
        })?,
        None => defaults.profile,
    };
    let count = |flag: &str, value: &Option<String>, default: usize| -> Result<usize, String> {
        match value {
            Some(value) => parse_count(flag, value).map(|n| n as usize),
            None => Ok(default),
        }
    };
    Ok(ExportOptions {
        profile,
        max_levels: match &args.max_levels {
            Some(value) => parse_count("--max-levels", value)?,
            None => defaults.max_levels,
        },
        max_tiles: count("--max-tiles", &args.max_tiles, defaults.max_tiles)?,
        target_text_px: match &args.text_px {
            Some(value) => parse_positive("--text-px", value)?,
            None => defaults.target_text_px,
        },
        crop: parse_crop(&args.crop)?,
        padding: match &args.padding {
            Some(value) => Some(parse_non_negative("--padding", value)?),
            None => None,
        },
        include_hidden: args.include_hidden,
        shard_kb: match &args.shard_kb {
            Some(value) => parse_whole("--shard-kb", "kilobytes", value)? as usize,
            None => defaults.shard_kb,
        },
        svg: args.svg,
        full: args.full,
        source_name: Path::new(input)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned()),
        frame_gap: match &args.frame_gap {
            Some(value) => parse_non_negative("--frame-gap", value)?,
            None => defaults.frame_gap,
        },
        min_frame_entities: count(
            "--min-frame-entities",
            &args.min_frame_entities,
            defaults.min_frame_entities,
        )?,
        max_frames: count("--max-frames", &args.max_frames, defaults.max_frames)?,
        fonts: parse_fonts(&args.fonts)?,
        sheets: !args.no_sheets,
    })
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

fn parse_fonts(value: &str) -> Result<Fonts, String> {
    match value {
        "bundled" => Ok(Fonts::Bundled),
        "bundled+system" | "system" => Ok(Fonts::BundledAndSystem),
        other => Err(format!(
            "unsupported --fonts value '{other}' (bundled or bundled+system)"
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
        fonts: parse_fonts(&args.fonts)?,
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
    parse_whole(flag, "pixels", value)
}

/// A whole number of `unit` (pixels, kilobytes) greater than 0.
fn parse_whole(flag: &str, unit: &str, value: &str) -> Result<u32, String> {
    match value.parse::<u32>() {
        Ok(number) if number > 0 => Ok(number),
        _ => Err(format!(
            "{flag} must be a whole number of {unit} greater than 0 (got '{value}')"
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
