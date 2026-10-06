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
    to_png, to_svg, Background, Crop, LeftOut, Paper, PngError, PngSize, Rect, Space, ToPngOptions,
    ToSvgOptions, DEFAULT_MAX_EDGE,
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
  uncad summarize <input> [--type <name>] [--layer <name>]
                  [--within <x0,y0,x1,y1>] [--space <model|paper>]
                  [--id <n>] [--limit <n>] [--detail]
                                    what the drawing contains, as JSON; any
                                      of the options also selects entities
                                      (every one given must hold), each
                                      with its box -- and with --detail its
                                      model record, whose fields are the
                                      paths `set` takes
  uncad hit-test <input> --x <n> --y <n> --tolerance <n> [--limit <n>]
                                    the entities at a point, as JSON
  uncad diff <before> <after> [--matching <auto|reference|geometry>]
             [--length-tolerance <n>] [--angle-tolerance <n>]
             [--min-similarity <n>] [--min-margin <n>] [--max-pairs <n>]
             [--omit <within,unstated>]
                                    the numeric difference, as JSON
  uncad set <input> --id <n> --path <field> --value <json> -o <new.json>
                                    set one field of one entity, write the
                                      result as model JSON (never over an
                                      existing file), answer with the diff
  uncad redline <before> <after> -o <out.svg|out.png> [--fit <px>]
                [--stroke <px>] [--paper <light|dark>]
                [--frame <drawing|changes>] [--proposal-color <#rrggbb>]
                [--matching ...] [--length-tolerance <n>]
                [--angle-tolerance <n>] [--min-similarity <n>]
                [--min-margin <n>] [--max-pairs <n>]
                [--omit <within,unstated>]
                                    draw the difference on top of <before>:
                                      the original unchanged, the changes in
                                      red (or --proposal-color) with a
                                      revision cloud (never over an
                                      existing file); answer with what
                                      was marked and what could not be
  uncad mcp                         serve summarize, hit-test, diff, set and
                                      redline as MCP tools over stdio

  Every command but export reads model JSON (.json) as a drawing too --
  what `-o <output.json>` and `set` write -- so edits chain, and the last
  state renders like any drawing.

JSON options (every command that answers with JSON, and -o <output.json>):
  --pretty                    indented, multi-line JSON (default: one line)

SVG/PNG options:
  --include-hidden            draw entities hidden by their layer or flag (off,
                                frozen, non-plotting, DEFPOINTS, invisible)
  --space <model|paper|all>   which space to render (default: model)
                                model = the drawing itself
                                paper = sheet borders and title blocks
                                all   = everything, in one document
  --no-trim                   frame and draw every entity, instead of setting
                                aside the few far larger or farther than the
                                rest of the drawing (named in a warning)
  --padding <units>           margin around the drawing, in drawing units
  --window <x0,y0,x1,y1>      frame exactly this rectangle of the drawing, in
                                drawing units (x0 < x1, y0 < y1), instead of
                                its extent; --padding is still added around
                                it (not with --no-trim)
  --paper <light|dark>        the page the drawing is drawn on (default: light)
                                light = no background, pure white drawn black
                                dark  = a black page, pure white kept white and
                                        pure black drawn white; a PNG is black
                                        wherever the drawing does not touch,
                                        whatever --background says

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
  uncad drawing.dwg -o drawing.png --fit 4000
  uncad drawing.dwg -o detail.png --fit 2000 --window 0,0,500,300
  uncad drawing.dwg -o drawing.png --fit 4000 --stroke 2 --paper dark";

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
    window: Option<String>,
    paper: String,
    pretty: bool,
    include_hidden: bool,
    help: bool,
    /// Every option given, as typed (`--fit`), in order -- what
    /// [`refuse_unused_options`] checks against the output.
    given: Vec<String>,
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
        window: None,
        paper: "light".to_string(),
        pretty: false,
        include_hidden: false,
        help: false,
        given: Vec::new(),
    };
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "-o" | "--output" | "--space" | "--scale" | "--fit" | "--max-edge" | "--stroke"
            | "--background" | "--padding" | "--window" | "--paper" => {
                let flag = argv[i].as_str();
                args.given.push(flag.to_string());
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
                    "--window" => args.window = Some(value),
                    "--paper" => args.paper = value,
                    _ => args.output = Some(value),
                }
            }
            "--no-trim" | "--pretty" | "--include-hidden" => {
                args.given.push(argv[i].clone());
                match argv[i].as_str() {
                    "--no-trim" => args.outlier_trim = false,
                    "--pretty" => args.pretty = true,
                    _ => args.include_hidden = true,
                }
            }
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

    // Asked-for help is the answer, on stdout; usage after a call without an
    // input is an error message, on stderr.
    if args.help {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if args.input.is_none() {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
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
    refuse_unused_options(args)?;
    // A drawing or model JSON: rendering and the summary need the model
    // alone, not the header only a drawing carries.
    let mut warnings = Vec::new();
    let db = verbs::read(input, &mut warnings)?;
    for warning in warnings {
        eprintln!("warning: {warning}");
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

    let reports = match extension.as_str() {
        "json" => {
            let json = db
                .to_json(ToJsonOptions {
                    pretty: args.pretty,
                })
                .map_err(|e| e.to_string())?;
            write_output(output, json.as_bytes())?;
            Reports::default()
        }
        "svg" => {
            let result = to_svg(&db, svg_options(args)?);
            write_output(output, result.svg.as_bytes())?;
            Reports {
                unsupported: result.unsupported_types,
                empty_blocks: result.empty_blocks,
                undefined_arcs: result.undefined_arcs,
                undefined_leaders: result.undefined_leaders,
                unsized_arrowheads: result.unsized_arrowheads,
                left_out: result.crop.left_out,
            }
        }
        "png" => {
            let result = to_png(&db, png_options(args)?).map_err(png_error)?;
            write_output(output, &result.png)?;
            Reports {
                unsupported: result.unsupported_types,
                empty_blocks: result.empty_blocks,
                undefined_arcs: result.undefined_arcs,
                undefined_leaders: result.undefined_leaders,
                unsized_arrowheads: result.unsized_arrowheads,
                left_out: result.crop.left_out,
            }
        }
        other => {
            return Err(format!(
                "unsupported output extension '.{other}' (only .json, .svg, .png)"
            ))
        }
    };
    let Reports {
        unsupported,
        empty_blocks,
        undefined_arcs,
        undefined_leaders,
        unsized_arrowheads,
        left_out,
    } = reports;

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
    if !undefined_arcs.is_empty() {
        let ids: Vec<String> = undefined_arcs
            .iter()
            .map(|id| id.value().to_string())
            .collect();
        eprintln!(
            "warning: arcs not drawn, their start and end angles being equal: {}",
            ids.join(", ")
        );
    }
    if !undefined_leaders.is_empty() {
        let ids: Vec<String> = undefined_leaders
            .iter()
            .map(|id| id.value().to_string())
            .collect();
        eprintln!(
            "warning: leaders not drawn, the file not defining their curve (a spline, or a path \
             it does not state): {}",
            ids.join(", ")
        );
    }
    if !unsized_arrowheads.is_empty() {
        let ids: Vec<String> = unsized_arrowheads
            .iter()
            .map(|id| id.value().to_string())
            .collect();
        eprintln!(
            "warning: arrowheads drawn at a default size, the file not stating theirs: {}",
            ids.join(", ")
        );
    }
    let set_aside = set_aside(&left_out);
    if !set_aside.is_empty() {
        eprintln!(
            "warning: left out of the image, far larger or farther than the rest of the \
             drawing (--no-trim draws them): {}",
            set_aside.join(", ")
        );
    }
    Ok(())
}

/// The entities a crop set aside and did not draw, as `TYPE id`. One outside
/// the view is still in the document, so it is not among them.
fn set_aside(left_out: &[LeftOut]) -> Vec<String> {
    left_out
        .iter()
        .filter(|l| l.reason.is_set_aside())
        .map(|l| format!("{} {}", l.type_name, l.id.value()))
        .collect()
}

/// The reader's non-fatal problems with `input`, worded once for every
/// command and either reader (LibreDWG for a DWG, the DXF reader for a DXF).
fn read_warning(input: &str, db: &CadDatabase) -> String {
    format!(
        "'{input}' was read with non-fatal problems ({}); what the reader could not decode \
         is missing from the result or marked in it",
        db.read_diagnostics.warnings.join(", ")
    )
}

/// One line of JSON laid out as `serde_json`'s pretty printer lays it out
/// (two spaces, `": "`), working on the text: the keys keep their order and
/// every number and string keeps its bytes -- only whitespace is added
/// outside strings.
fn indented(json: &str) -> String {
    let mut out = String::with_capacity(json.len() * 2);
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut chars = json.chars().peekable();
    let newline = |out: &mut String, depth: usize| {
        out.push('\n');
        out.push_str(&"  ".repeat(depth));
    };
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '{' | '[' => {
                out.push(c);
                // An empty object or array stays on its line.
                if matches!(chars.peek(), Some('}' | ']')) {
                    out.push(chars.next().expect("peeked"));
                } else {
                    depth += 1;
                    newline(&mut out, depth);
                }
            }
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                newline(&mut out, depth);
                out.push(c);
            }
            ',' => {
                out.push(c);
                newline(&mut out, depth);
            }
            ':' => out.push_str(": "),
            _ => out.push(c),
        }
    }
    out
}

/// `uncad <verb> ...`: the answer on stdout as one line of JSON, warnings on
/// stderr -- the same answer `uncad mcp` gives for the same arguments.
fn run_verb(verb: &verbs::Verb, argv: &[String]) -> ExitCode {
    if argv.iter().any(|a| a == "-h" || a == "--help") {
        print!("{}", verb.help());
        return ExitCode::SUCCESS;
    }
    // `--pretty` is how the answer is printed, not an argument of the verb:
    // the MCP tool, which has no terminal to print to, never takes it.
    let pretty = argv.iter().any(|a| a == "--pretty");
    let argv: Vec<String> = argv.iter().filter(|a| *a != "--pretty").cloned().collect();
    match verb.parse_cli(&argv).and_then(|args| verb.call(&args)) {
        Ok(answer) => {
            if pretty {
                println!("{}", indented(&answer.json));
            } else {
                println!("{}", answer.json);
            }
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
/// DWG that is not a drawing as a bare LibreDWG error code, which is accurate
/// but not something a user can act on without cross-referencing dwg.h. The
/// obvious cases are checked here first so the message says what is actually
/// wrong. Both commands read their input through here, so they word a bad
/// input the same way.
fn parse_input(input: &str) -> Result<(CadDatabase, uncad::Header), String> {
    if verbs::is_model_json(input) {
        return Err(format!(
            "'{input}' is model JSON; export needs the drawing itself (.dwg or .dxf), whose \
             header it reads -- every other command reads model JSON"
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
    let crop = match (&args.window, args.outlier_trim) {
        (Some(_), false) => {
            return Err(
                "--window and --no-trim both choose what the picture frames; give one".into(),
            )
        }
        (Some(window), true) => Crop::Window(parse_window(window)?),
        (None, true) => Crop::default(),
        (None, false) => Crop::Everything,
    };
    let mut options = ToSvgOptions {
        space: parse_space(&args.space)?,
        crop,
        include_hidden: args.include_hidden,
        paper: parse_paper(&args.paper)?,
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

/// Options that shape the picture, SVG or PNG.
const IMAGE_OPTIONS: [&str; 6] = [
    "--space",
    "--no-trim",
    "--window",
    "--padding",
    "--paper",
    "--include-hidden",
];
/// Options in pixels, which only a PNG has.
const PNG_OPTIONS: [&str; 5] = ["--scale", "--fit", "--max-edge", "--stroke", "--background"];

/// An option the requested output has no use for is an error, named, before
/// anything is read: a width that silently does nothing reads as a width
/// that was applied.
fn refuse_unused_options(args: &Args) -> Result<(), String> {
    let extension = args.output.as_deref().map(|o| {
        Path::new(o)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase()
    });
    let (usable, what): (Vec<&str>, &str) = match extension.as_deref() {
        None => (Vec::new(), "the summary (no -o)"),
        Some("json") => (vec!["--pretty"], "model JSON"),
        Some("svg") => (
            IMAGE_OPTIONS.to_vec(),
            "an SVG, which has no pixels -- write a .png for sizes, strokes and backgrounds",
        ),
        Some("png") => (
            IMAGE_OPTIONS.iter().chain(&PNG_OPTIONS).copied().collect(),
            "a PNG",
        ),
        // Refused with its own message when the output is written.
        Some(_) => return Ok(()),
    };
    let mut unused: Vec<&str> = Vec::new();
    for flag in args.given.iter().map(String::as_str) {
        if !matches!(flag, "-o" | "--output") && !usable.contains(&flag) && !unused.contains(&flag)
        {
            unused.push(flag);
        }
    }
    if unused.is_empty() {
        return Ok(());
    }
    Err(format!(
        "{} {} nothing for {what}; leave {} out",
        unused.join(", "),
        if unused.len() == 1 { "does" } else { "do" },
        if unused.len() == 1 { "it" } else { "them" }
    ))
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

/// `x0,y0,x1,y1`: four finite numbers, a rectangle with an area.
fn parse_window(value: &str) -> Result<Rect, String> {
    let numbers: Option<Vec<f64>> = value
        .split(',')
        .map(|n| n.trim().parse::<f64>().ok().filter(|v| v.is_finite()))
        .collect();
    match numbers.as_deref() {
        Some(&[x0, y0, x1, y1]) if x0 < x1 && y0 < y1 => Ok(Rect::new(x0, y0, x1, y1)),
        Some(&[_, _, _, _]) => Err(format!(
            "--window must be a rectangle with x0 < x1 and y0 < y1 (got '{value}')"
        )),
        _ => Err(format!(
            "--window must be four finite numbers x0,y0,x1,y1 (got '{value}')"
        )),
    }
}

/// `light` or `dark`, as `--paper` takes it.
fn parse_paper(value: &str) -> Result<Paper, String> {
    match value {
        "light" => Ok(Paper::Light),
        "dark" => Ok(Paper::Dark),
        other => Err(format!(
            "unsupported --paper value '{other}' (one of light, dark)"
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
                println!("{USAGE}");
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
    if !db.read_diagnostics.is_clean() {
        eprintln!("warning: {}", read_warning(input, &db));
    }
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

/// What a picture left out or drew from a default, to say on standard error.
#[derive(Default)]
struct Reports {
    unsupported: Vec<String>,
    empty_blocks: Vec<String>,
    undefined_arcs: Vec<uncad::model::EntityId>,
    undefined_leaders: Vec<uncad::model::EntityId>,
    unsized_arrowheads: Vec<uncad::model::EntityId>,
    left_out: Vec<iron_render_cad::LeftOut>,
}
