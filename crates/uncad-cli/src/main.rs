//! Command-line front end for `uncad`: `<input> [-o <output>]`.
//!
//! Reading only -- with no `-o` it prints a summary, and the `-o` targets are
//! the parsed model as JSON or a rendering of it as SVG/PNG. There is no
//! DWG/DXF output. The verbs (`summarize`, `hit-test`, `diff`) answer one
//! question each as JSON, and `set` makes one edit and writes it as model
//! JSON, from the table in `verbs.rs`, which `uncad mcp` also serves as MCP
//! tools. The verbs read model JSON as a drawing too.

mod mcp;
mod verbs;

use iron_render_cad::{
    to_png, to_svg, Background, Crop, PngError, PngSize, Space, ToPngOptions, ToSvgOptions,
    DEFAULT_MAX_EDGE,
};
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
  uncad export <input> -o <dir>     write the LLM/VLM package (images + JSON)
  uncad summarize <input>           what the drawing contains, as JSON
  uncad hit-test <input> --x <n> --y <n> --tolerance <n>
                                    the entities at a point, as JSON
  uncad diff <before> <after> [--matching <reference|geometry>]
             [--length-tolerance <n>] [--angle-tolerance <n>]
                                    the numeric difference, as JSON
  uncad set <input> --id <n> --path <field> --value <json> -o <new.json>
                                    set one field of one entity, write the
                                      result as model JSON (never over an
                                      existing file), answer with the diff
  uncad mcp                         serve the four verbs above as MCP tools
                                      over stdio

  The four verbs read model JSON (.json) as a drawing too -- what
  `-o <output.json>` and `set` write -- so edits chain.

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
  --padding <units>           margin around the drawing, in drawing units

PNG options:
  --scale <factor>            pixels per drawing unit (default: 1.0, e.g. 2.0
                                for twice the resolution)
  --fit <px>                  make the longer side this many pixels instead,
                                whatever the drawing's units (not with --scale)
  --max-edge <px>             refuse an image with a side longer than this
                                (default: 8192 -- the pixels are allocated
                                before drawing, so the bound keeps a drawing
                                from asking for gigabytes)
  --stroke <px>               every line this many pixels wide
  --background <white|transparent>
                              what the drawing does not cover (default: white)

Other options:
  -h, --help                  this text
  -V, --version               print the version

Export options (uncad export):
  --profile <name>            claude (default), claude-hires, openai-patch
  --max-levels <n>            deepest tile level (default 5; 0 = overview only)
  --max-tiles <n>             tile budget (default 400)
  --no-sheets                 skip the paper-layout sheet images
  --svg                       also write drawing.svg

Examples:
  uncad drawing.dwg
  uncad drawing.dwg -o drawing.json --pretty
  uncad drawing.dwg -o drawing.svg
  uncad drawing.dwg -o drawing.svg --space paper
  uncad drawing.dwg -o drawing.png --scale 2
  uncad drawing.dwg -o drawing.png --fit 4000";

struct Args {
    input: Option<String>,
    output: Option<String>,
    space: String,
    outlier_trim: bool,
    scale: Option<String>,
    fit: Option<String>,
    max_edge: Option<String>,
    stroke: Option<String>,
    background: String,
    padding: Option<String>,
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
        scale: None,
        fit: None,
        max_edge: None,
        stroke: None,
        background: "white".to_string(),
        padding: None,
        pretty: false,
        include_hidden: false,
        help: false,
    };
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "-o" | "--output" | "--space" | "--scale" | "--fit" | "--max-edge" | "--stroke"
            | "--background" | "--padding" => {
                let flag = argv[i].as_str();
                i += 1;
                let value = argv
                    .get(i)
                    .cloned()
                    .ok_or_else(|| format!("{flag} needs a value"))?;
                match flag {
                    "--space" => args.space = value,
                    "--scale" => args.scale = Some(value),
                    "--fit" => args.fit = Some(value),
                    "--max-edge" => args.max_edge = Some(value),
                    "--stroke" => args.stroke = Some(value),
                    "--background" => args.background = value,
                    "--padding" => args.padding = Some(value),
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
    // Both commands answer `--version`, before either parses anything.
    if argv.iter().any(|a| a == "-V" || a == "--version") {
        println!("uncad {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if argv.first().map(String::as_str) == Some("mcp") {
        if argv.len() > 1 {
            eprintln!("error: uncad mcp takes no arguments");
            return ExitCode::FAILURE;
        }
        return match mcp::serve() {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("error: {message}");
                ExitCode::FAILURE
            }
        };
    }
    if let Some(verb) = argv.first().and_then(|name| verbs::find(name)) {
        return run_verb(verb, &argv[1..]);
    }
    if argv.first().map(String::as_str) == Some("export") {
        return match run_export(&argv[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("error: {message}");
                ExitCode::FAILURE
            }
        };
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
    let (db, _) = parse_input(input)?;
    if !db.read_diagnostics.is_clean() {
        eprintln!("warning: {}", read_warning(input, &db));
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
            let result = to_png(&db, png_options(args)?).map_err(png_error)?;
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

/// The reader's non-fatal problems with `input`, worded once for every
/// command.
fn read_warning(input: &str, db: &CadDatabase) -> String {
    format!(
        "LibreDWG read '{input}' with non-fatal problems ({}); objects it could not decode \
         are missing from the result",
        db.read_diagnostics.warnings.join(", ")
    )
}

/// `uncad <verb> ...`: the answer on stdout as one line of JSON, warnings on
/// stderr -- the same answer `uncad mcp` gives for the same arguments.
fn run_verb(verb: &verbs::Verb, argv: &[String]) -> ExitCode {
    if argv.iter().any(|a| a == "-h" || a == "--help") {
        eprintln!("{}\n\n{}", verb.usage(), verb.description);
        return ExitCode::SUCCESS;
    }
    match verb.parse_cli(argv).and_then(|args| verb.call(&args)) {
        Ok(answer) => {
            println!("{}", answer.json);
            for warning in &answer.warnings {
                eprintln!("warning: {warning}");
            }
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

/// `uncad::parse()` reports a path it cannot read as `ParseError::Io`, and a
/// file that is not a drawing as a bare LibreDWG error code, which is accurate
/// but not something a user can act on without cross-referencing dwg.h. The
/// obvious cases are checked here first so the message says what is actually
/// wrong. Both commands read their input through here, so they word a bad
/// input the same way.
fn parse_input(input: &str) -> Result<(CadDatabase, uncad::Header), String> {
    if verbs::is_model_json(input) {
        return Err(format!(
            "'{input}' is model JSON; this command needs the drawing itself (.dwg or .dxf) -- \
             the verbs summarize, hit-test, diff and set read model JSON"
        ));
    }
    match std::fs::metadata(input) {
        Ok(meta) if meta.is_dir() => {
            return Err(format!("input path is a directory, not a file: '{input}'"))
        }
        Err(e) => return Err(format!("cannot open input file '{input}': {e}")),
        Ok(_) => {}
    }
    uncad::parse_with_header(input)
        .map_err(|e| format!("could not parse '{input}' ({e}) -- is it a valid DWG/DXF file?"))
}

fn write_output(path: &str, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("cannot write '{path}': {e}"))
}

fn svg_options(args: &Args) -> Result<ToSvgOptions, String> {
    let mut options = ToSvgOptions {
        space: parse_space(&args.space)?,
        crop: if args.outlier_trim {
            Crop::Cluster
        } else {
            Crop::Everything
        },
        include_hidden: args.include_hidden,
        ..Default::default()
    };
    if let Some(padding) = &args.padding {
        options.padding = match padding.parse::<f64>() {
            Ok(p) if p >= 0.0 && p.is_finite() => p,
            _ => {
                return Err(format!(
                    "--padding must be a finite number, 0 or more (got '{padding}')"
                ))
            }
        };
    }
    Ok(options)
}

fn png_options(args: &Args) -> Result<ToPngOptions, String> {
    let size = match (&args.scale, &args.fit) {
        (Some(_), Some(_)) => return Err("--scale and --fit both set the size; give one".into()),
        (_, Some(fit)) => PngSize::FitLongEdge(parse_pixels(fit, "--fit")?),
        (Some(scale), None) => PngSize::Scale(parse_scale(scale)?),
        (None, None) => PngSize::Scale(1.0),
    };
    let max_edge = match &args.max_edge {
        Some(v) => parse_pixels(v, "--max-edge")?,
        None => DEFAULT_MAX_EDGE,
    };
    let stroke_px = match &args.stroke {
        Some(v) => match v.parse::<f64>() {
            Ok(px) if px > 0.0 && px.is_finite() => Some(px),
            _ => {
                return Err(format!(
                    "--stroke must be a finite number greater than 0 (got '{v}')"
                ))
            }
        },
        None => None,
    };
    let background = match args.background.as_str() {
        "white" => Background::White,
        "transparent" => Background::Transparent,
        other => {
            return Err(format!(
                "unsupported --background value '{other}' (one of white, transparent)"
            ))
        }
    };
    Ok(ToPngOptions {
        svg: svg_options(args)?,
        size,
        stroke_px,
        max_edge,
        background,
        ..ToPngOptions::default()
    })
}

/// The renderer's own message names its option (`max_edge`); here it says
/// what to type.
fn png_error(e: PngError) -> String {
    match e {
        PngError::TooLarge {
            width,
            height,
            max_edge,
        } => format!(
            "the image would be {width}x{height} px, more than the {max_edge} px limit on a \
             side: ask for a smaller one with --fit <px> or --scale <factor>, or raise the \
             limit with --max-edge <px>"
        ),
        other => other.to_string(),
    }
}

fn parse_pixels(value: &str, flag: &str) -> Result<u32, String> {
    match value.parse::<u32>() {
        Ok(px) if px > 0 => Ok(px),
        _ => Err(format!(
            "{flag} must be a whole number of pixels, 1 or more (got '{value}')"
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

/// `uncad export <input> -o <dir> [options]`: the LLM/VLM package through
/// `iron-pack-cad`. An option this subcommand does not know is an error, not
/// something silently ignored.
fn run_export(argv: &[String]) -> Result<(), String> {
    let mut input: Option<&str> = None;
    let mut output: Option<&str> = None;
    let mut options = iron_pack_cad::ExportOptions::default();
    let mut i = 0;
    let value = |i: usize, flag: &str| -> Result<&str, String> {
        argv.get(i + 1)
            .map(String::as_str)
            .ok_or_else(|| format!("{flag} needs a value"))
    };
    let count = |v: &str, flag: &str| -> Result<usize, String> {
        v.parse::<usize>()
            .map_err(|_| format!("{flag} takes a whole number, not '{v}'"))
    };
    while i < argv.len() {
        match argv[i].as_str() {
            "-o" | "--output" => {
                output = Some(value(i, "-o")?);
                i += 1;
            }
            "--profile" => {
                let name = value(i, "--profile")?;
                options.profile = iron_pack_cad::Profile::by_name(name).ok_or_else(|| {
                    let names: Vec<&str> =
                        iron_pack_cad::Profile::ALL.iter().map(|p| p.name).collect();
                    format!("unknown profile '{name}' (one of: {})", names.join(", "))
                })?;
                i += 1;
            }
            "--max-levels" => {
                options.max_levels = count(value(i, "--max-levels")?, "--max-levels")? as u32;
                i += 1;
            }
            "--max-tiles" => {
                options.max_tiles = count(value(i, "--max-tiles")?, "--max-tiles")?;
                i += 1;
            }
            "--no-sheets" => options.sheets = false,
            "--svg" => options.svg = true,
            "-h" | "--help" => {
                eprintln!("{USAGE}");
                return Ok(());
            }
            flag if flag.starts_with('-') => {
                return Err(format!("unknown option '{flag}' for uncad export"));
            }
            positional => {
                if let Some(first) = input {
                    return Err(format!(
                        "unexpected argument '{positional}': the input is already '{first}'"
                    ));
                }
                input = Some(positional);
            }
        }
        i += 1;
    }
    let input = input.ok_or("uncad export needs an input drawing")?;
    let output = output.ok_or("uncad export needs -o <dir>")?;
    let (db, header) = parse_input(input)?;
    let input = Path::new(input);
    let header = pack_header(&header)?;
    if options.source_name.is_none() {
        options.source_name = input.file_name().map(|n| n.to_string_lossy().into_owned());
    }
    let report = iron_pack_cad::export_package(&db, Some(&header), Path::new(output), &options)
        .map_err(|e| e.to_string())?;
    for warning in &report.warnings {
        eprintln!("warning: {warning}");
    }
    println!(
        "wrote: {} ({} files; overview {}x{} px; {} frames; {} sheets)",
        report.dir.display(),
        report.files.len(),
        report.overview.px[0],
        report.overview.px[1],
        report.frames.len(),
        report.sheets.len()
    );
    Ok(())
}

/// The header as `iron-pack-cad` takes it. Both crates name a header
/// variable after its DXF `$VARIABLE` in lower case, so one JSON round trip
/// carries every variable the package reads; a field only this reader has
/// (the codepage number, `$MEASUREMENT`, ...) lands in the package's
/// `Header::other` and is written back unchanged.
fn pack_header(header: &uncad::Header) -> Result<iron_pack_cad::Header, String> {
    serde_json::to_value(header)
        .and_then(serde_json::from_value)
        .map_err(|e| format!("the drawing's header does not carry over: {e}"))
}
