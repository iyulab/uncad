//! The verb table: every question this tool answers about a drawing, and
//! the one edit it makes, written down once.
//!
//! The command line (`uncad <verb> ...`) and the MCP server (`uncad mcp`)
//! both answer from here, through [`Verb::call`], so a verb's answer is the
//! same bytes whichever way it was asked. An answer is the result structure
//! of the library that computes it, serialized as it is -- this table adds
//! no schema of its own, and holds no state between calls: every call reads
//! its files again.
//!
//! A drawing argument is a DWG or DXF file, or the model JSON this tool
//! writes (`uncad <drawing> -o <state.json>`, or `set`'s output) -- which is
//! how an edit's result is the next call's input. The one verb that writes,
//! `set`, writes model JSON only, never over an existing file, and never
//! changes its input.
//!
//! Argument names are snake_case, as the libraries spell them; the command
//! line writes them as `--kebab-case` flags.

use serde_json::{Map, Value};
use uncad::CadDatabase;

/// What a verb answers: the result structure as JSON text (one line), and
/// what went wrong while reading the input that the result itself does not
/// say. The command line prints the first on stdout and the second on
/// stderr; the MCP server returns them as the first and the following
/// content blocks.
pub struct Answer {
    pub json: String,
    pub warnings: Vec<String>,
}

/// The type of an argument value.
pub enum Kind {
    /// A file, as a path.
    Path,
    /// An integer, 0 or more.
    Integer,
    /// A non-empty string.
    Text,
    /// Any JSON value. The command line takes it as JSON text.
    Json,
    /// A finite number.
    Number,
    /// A finite number, 0 or more.
    NonNegative,
    /// One of the listed words.
    Choice(&'static [&'static str]),
}

pub struct Param {
    pub name: &'static str,
    pub kind: Kind,
    pub required: bool,
    /// Given on the command line by position instead of as a flag, in the
    /// order the table lists them.
    pub positional: bool,
    pub description: &'static str,
}

pub struct Verb {
    pub name: &'static str,
    pub description: &'static str,
    pub params: &'static [Param],
    /// Whether the verb writes a file. None changes its input.
    pub writes: bool,
    run: fn(&Map<String, Value>) -> Result<Answer, String>,
}

pub const VERBS: &[Verb] = &[
    Verb {
        name: "summarize",
        description: "What a drawing contains: the entity count per type, every layer and block \
            definition, the attribute values on block references, loose texts that read as \
            label and value, and the lowest confidence of anything summarized. A value that \
            several places give differently is listed with every value, never one of them.",
        params: &[Param {
            name: "input",
            kind: Kind::Path,
            required: true,
            positional: true,
            description: DRAWING,
        }],
        writes: false,
        run: summarize,
    },
    Verb {
        name: "hit_test",
        description: "The entities at a point of a drawing: every entity whose geometry passes \
            within the tolerance (nearest first, none preferred over another), closed shapes \
            that enclose the point, what block references draw there with the chain of \
            references it was reached through, and what could not be searched and why. \
            Coordinates are in the drawing's own units.",
        params: &[
            Param {
                name: "input",
                kind: Kind::Path,
                required: true,
                positional: true,
                description: DRAWING,
            },
            Param {
                name: "x",
                kind: Kind::Number,
                required: true,
                positional: false,
                description: "The point's x coordinate, in drawing units.",
            },
            Param {
                name: "y",
                kind: Kind::Number,
                required: true,
                positional: false,
                description: "The point's y coordinate, in drawing units.",
            },
            Param {
                name: "tolerance",
                kind: Kind::NonNegative,
                required: true,
                positional: false,
                description: "How far from the point an entity's geometry may pass and still \
                    be a hit, in drawing units. There is no default: the right value depends \
                    on the drawing's scale.",
            },
        ],
        writes: false,
        run: hit_test,
    },
    Verb {
        name: "diff",
        description: "The exact numeric difference between two drawing states: which entities \
            were added, removed or modified, and for a modified one every field that differs, \
            by how much, and whether that is within or beyond the tolerance, which is written \
            into the answer. `reference` matching pairs entities by their reference IDs (two \
            saves of the same drawing); `geometry` pairs them by type and shape, only where \
            the pairing is certain, and reports the rest as unknown with the candidates.",
        params: &[
            Param {
                name: "before",
                kind: Kind::Path,
                required: true,
                positional: true,
                description: "The earlier drawing (.dwg, .dxf, or model JSON .json).",
            },
            Param {
                name: "after",
                kind: Kind::Path,
                required: true,
                positional: true,
                description: "The later drawing (.dwg, .dxf, or model JSON .json).",
            },
            Param {
                name: "matching",
                kind: Kind::Choice(&["reference", "geometry"]),
                required: false,
                positional: false,
                description: "How entities of the two drawings are paired (default: reference).",
            },
            Param {
                name: "length_tolerance",
                kind: Kind::NonNegative,
                required: false,
                positional: false,
                description: "Tolerance for every numeric field except angles, in drawing \
                    units (default: 1e-6).",
            },
            Param {
                name: "angle_tolerance",
                kind: Kind::NonNegative,
                required: false,
                positional: false,
                description: "Tolerance for angles, in radians (default: 1e-9).",
            },
        ],
        writes: false,
        run: diff,
    },
    Verb {
        name: "set",
        description: "Sets one field of one entity to a value and writes the edited drawing as \
            a new model JSON file, which every verb reads as a drawing -- so edits chain, each \
            call starting from the last one's output. The answer is the numeric difference from \
            the input to what was written (as `diff` answers it), which shows that the one field \
            changed and nothing else. The input is never changed and an existing file is never \
            written over. An edit the drawing does not allow is refused with the reason, and \
            nothing is written.",
        params: &[
            Param {
                name: "input",
                kind: Kind::Path,
                required: true,
                positional: true,
                description: DRAWING,
            },
            Param {
                name: "id",
                kind: Kind::Integer,
                required: true,
                positional: false,
                description: "The entity's reference ID, as summarize and hit_test give it.",
            },
            Param {
                name: "path",
                kind: Kind::Text,
                required: true,
                positional: false,
                description: "The field, as the model JSON names it: `radius`, `center.x`, \
                    `vertices[2].point.y`, `common.layer`.",
            },
            Param {
                name: "value",
                kind: Kind::Json,
                required: true,
                positional: false,
                description: "The new value, in the model JSON's form for that field: a number, \
                    a string in quotes, an object.",
            },
            Param {
                name: "output",
                kind: Kind::Path,
                required: true,
                positional: false,
                description: "Where to write the edited drawing: a .json path that does not \
                    exist yet.",
            },
        ],
        writes: true,
        run: set,
    },
];

const DRAWING: &str = "The drawing (.dwg, .dxf, or model JSON .json).";

/// The verb named `name`, in either spelling (`hit_test` or `hit-test`).
pub fn find(name: &str) -> Option<&'static Verb> {
    let name = name.replace('-', "_");
    VERBS.iter().find(|v| v.name == name)
}

/// `name` as the command line writes it.
pub fn cli_spelling(name: &str) -> String {
    name.replace('_', "-")
}

impl Verb {
    /// Checks `args` against this verb's parameters, then answers. The one
    /// path both front ends take.
    pub fn call(&self, args: &Map<String, Value>) -> Result<Answer, String> {
        self.check(args)?;
        (self.run)(args)
    }

    fn check(&self, args: &Map<String, Value>) -> Result<(), String> {
        for key in args.keys() {
            if !self.params.iter().any(|p| p.name == key) {
                return Err(format!("{} takes no argument '{key}'", self.name));
            }
        }
        for param in self.params {
            let Some(value) = args.get(param.name) else {
                if param.required {
                    return Err(format!("{} needs '{}'", self.name, param.name));
                }
                continue;
            };
            let fits = match &param.kind {
                Kind::Path | Kind::Text => value.as_str().is_some_and(|s| !s.is_empty()),
                Kind::Integer => value.as_u64().is_some(),
                Kind::Json => true,
                Kind::Number => value.as_f64().is_some_and(f64::is_finite),
                Kind::NonNegative => value.as_f64().is_some_and(|v| v.is_finite() && v >= 0.0),
                Kind::Choice(words) => value.as_str().is_some_and(|s| words.contains(&s)),
            };
            if !fits {
                return Err(format!(
                    "'{}' must be {} (got {value})",
                    param.name,
                    param.kind.expected()
                ));
            }
        }
        Ok(())
    }

    /// The JSON Schema of this verb's arguments.
    pub fn input_schema(&self) -> Map<String, Value> {
        let mut properties = Map::new();
        for param in self.params {
            let mut schema = Map::new();
            match &param.kind {
                Kind::Path | Kind::Text => {
                    schema.insert("type".into(), "string".into());
                }
                Kind::Integer => {
                    schema.insert("type".into(), "integer".into());
                    schema.insert("minimum".into(), 0.into());
                }
                // Any JSON value: the schema leaves the type open.
                Kind::Json => {}
                Kind::Number => {
                    schema.insert("type".into(), "number".into());
                }
                Kind::NonNegative => {
                    schema.insert("type".into(), "number".into());
                    schema.insert("minimum".into(), 0.into());
                }
                Kind::Choice(words) => {
                    schema.insert("type".into(), "string".into());
                    schema.insert("enum".into(), (*words).into());
                }
            }
            schema.insert("description".into(), param.description.into());
            properties.insert(param.name.into(), schema.into());
        }
        let required: Vec<&str> = self
            .params
            .iter()
            .filter(|p| p.required)
            .map(|p| p.name)
            .collect();
        let mut schema = Map::new();
        schema.insert("type".into(), "object".into());
        schema.insert("properties".into(), properties.into());
        schema.insert("required".into(), required.into());
        schema.insert("additionalProperties".into(), false.into());
        schema
    }

    /// The command-line arguments after the verb, as the argument object
    /// [`Verb::call`] takes. Values are checked there, not here: this only
    /// maps words to names and numbers to numbers.
    pub fn parse_cli(&self, argv: &[String]) -> Result<Map<String, Value>, String> {
        let verb = cli_spelling(self.name);
        let mut args = Map::new();
        let mut positionals = self.params.iter().filter(|p| p.positional);
        let mut i = 0;
        while i < argv.len() {
            // `-o` is short for `--output`, as it is for the plain command.
            let word = match argv[i].as_str() {
                "-o" => "--output",
                other => other,
            };
            if let Some(flag) = word.strip_prefix("--").filter(|f| !f.is_empty()) {
                let param = self
                    .params
                    .iter()
                    .find(|p| !p.positional && cli_spelling(p.name) == flag)
                    .ok_or_else(|| format!("unknown option '{word}' for uncad {verb}"))?;
                i += 1;
                let raw = argv.get(i).ok_or_else(|| format!("{word} needs a value"))?;
                let value = match param.kind {
                    Kind::Number | Kind::NonNegative => raw
                        .parse::<f64>()
                        .ok()
                        .and_then(|v| serde_json::Number::from_f64(v).map(Value::Number))
                        .ok_or_else(|| format!("{word} must be a finite number (got '{raw}')"))?,
                    Kind::Integer => raw.parse::<u64>().map(Value::from).map_err(|_| {
                        format!("{word} must be an integer, 0 or more (got '{raw}')")
                    })?,
                    Kind::Json => serde_json::from_str(raw).map_err(|e| {
                        format!("{word} must be JSON -- a string in quotes (got '{raw}': {e})")
                    })?,
                    Kind::Path | Kind::Text | Kind::Choice(_) => Value::String(raw.clone()),
                };
                args.insert(param.name.into(), value);
            } else if word.starts_with('-') && word.len() > 1 {
                return Err(format!("unknown option '{word}' for uncad {verb}"));
            } else {
                let param = positionals
                    .next()
                    .ok_or_else(|| format!("unexpected argument '{word}' for uncad {verb}"))?;
                args.insert(param.name.into(), Value::String(word.to_string()));
            }
            i += 1;
        }
        Ok(args)
    }

    /// One usage line, as `--help` shows it.
    pub fn usage(&self) -> String {
        let mut line = format!("uncad {}", cli_spelling(self.name));
        for param in self.params {
            let word = if param.positional {
                format!("<{}>", param.name)
            } else {
                format!(
                    "--{} <{}>",
                    cli_spelling(param.name),
                    param.kind.placeholder()
                )
            };
            if param.required {
                line.push_str(&format!(" {word}"));
            } else {
                line.push_str(&format!(" [{word}]"));
            }
        }
        line
    }
}

impl Kind {
    fn expected(&self) -> String {
        match self {
            Kind::Path => "a file path".into(),
            Kind::Integer => "an integer, 0 or more".into(),
            Kind::Text => "a non-empty string".into(),
            Kind::Json => "a JSON value".into(),
            Kind::Number => "a finite number".into(),
            Kind::NonNegative => "a finite number, 0 or more".into(),
            Kind::Choice(words) => format!("one of {}", words.join(", ")),
        }
    }

    fn placeholder(&self) -> String {
        match self {
            Kind::Path => "path".into(),
            Kind::Integer | Kind::Number | Kind::NonNegative => "n".into(),
            Kind::Text => "text".into(),
            Kind::Json => "json".into(),
            Kind::Choice(words) => words.join("|"),
        }
    }
}

fn path(args: &Map<String, Value>, name: &str) -> String {
    args[name]
        .as_str()
        .expect("checked by Verb::call")
        .to_string()
}

fn number(args: &Map<String, Value>, name: &str) -> Option<f64> {
    args.get(name)
        .map(|v| v.as_f64().expect("checked by Verb::call"))
}

/// Whether `path` names model JSON rather than a drawing file.
pub fn is_model_json(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("json"))
}

/// Reads a drawing -- a DWG or DXF file, or model JSON -- with the reader's
/// non-fatal problems as warnings. Model JSON carries the problems of the
/// read that produced it, and they are reported the same way.
fn read(input: &str, warnings: &mut Vec<String>) -> Result<CadDatabase, String> {
    let db = if is_model_json(input) {
        let text = std::fs::read_to_string(input)
            .map_err(|e| format!("cannot open input file '{input}': {e}"))?;
        serde_json::from_str::<CadDatabase>(&text).map_err(|e| {
            format!(
                "cannot read '{input}' as model JSON ({e}) -- write it again from the drawing \
                 with `uncad <drawing> -o <file.json>`"
            )
        })?
    } else {
        crate::parse_input(input)?.0
    };
    if !db.read_diagnostics.is_clean() {
        warnings.push(if is_model_json(input) {
            format!(
                "'{input}' records non-fatal problems ({}) from the read of the drawing it was \
                 written from; objects that read could not decode are missing from it",
                db.read_diagnostics.warnings.join(", ")
            )
        } else {
            crate::read_warning(input, &db)
        });
    }
    Ok(db)
}

fn answer(result: &impl serde::Serialize, warnings: Vec<String>) -> Result<Answer, String> {
    let json = serde_json::to_string(result).map_err(|e| e.to_string())?;
    Ok(Answer { json, warnings })
}

fn summarize(args: &Map<String, Value>) -> Result<Answer, String> {
    let mut warnings = Vec::new();
    let db = read(&path(args, "input"), &mut warnings)?;
    answer(&iron_scout_cad::summarize(&db), warnings)
}

fn hit_test(args: &Map<String, Value>) -> Result<Answer, String> {
    let mut warnings = Vec::new();
    let db = read(&path(args, "input"), &mut warnings)?;
    let point = uncad::model::Point2D {
        x: number(args, "x").expect("required"),
        y: number(args, "y").expect("required"),
    };
    let tolerance = number(args, "tolerance").expect("required");
    answer(&iron_scout_cad::hit_test(&db, point, tolerance), warnings)
}

fn diff(args: &Map<String, Value>) -> Result<Answer, String> {
    let mut warnings = Vec::new();
    let before = read(&path(args, "before"), &mut warnings)?;
    let after = read(&path(args, "after"), &mut warnings)?;
    let mut options = iron_diff_cad::DiffOptions::default();
    if let Some(matching) = args.get("matching").and_then(Value::as_str) {
        options.matching = match matching {
            "geometry" => iron_diff_cad::Matching::Geometry,
            _ => iron_diff_cad::Matching::Reference,
        };
    }
    if let Some(length) = number(args, "length_tolerance") {
        options.tolerance.length = length;
    }
    if let Some(angle) = number(args, "angle_tolerance") {
        options.tolerance.angle = angle;
    }
    answer(&iron_diff_cad::diff(&before, &after, options), warnings)
}

fn set(args: &Map<String, Value>) -> Result<Answer, String> {
    let input = path(args, "input");
    let output = path(args, "output");
    if !is_model_json(&output) {
        return Err(format!(
            "set writes model JSON only: '{output}' does not end in .json"
        ));
    }
    // Checked before anything is read, so a refused call costs nothing; the
    // write below refuses an existing file again, atomically.
    if std::path::Path::new(&output).exists() {
        return Err(format!(
            "'{output}' already exists -- set never writes over a file, its input included; \
             give a new path"
        ));
    }
    let mut warnings = Vec::new();
    let before = read(&input, &mut warnings)?;
    let id = uncad::model::EntityId::new(args["id"].as_u64().expect("checked by Verb::call"));
    let field = args["path"].as_str().expect("checked by Verb::call");
    let after =
        iron_hand_cad::set(&before, id, field, args["value"].clone()).map_err(|refusal| {
            let reason = serde_json::to_value(&refusal)
                .ok()
                .and_then(|v| v["reason"].as_str().map(str::to_string))
                .unwrap_or_default();
            format!("refused ({reason}): {refusal}")
        })?;
    let json = after
        .to_json(uncad::ToJsonOptions::default())
        .map_err(|e| e.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)
        .map_err(|e| format!("cannot write '{output}': {e}"))?;
    std::io::Write::write_all(&mut file, json.as_bytes())
        .map_err(|e| format!("cannot write '{output}': {e}"))?;
    answer(
        &iron_diff_cad::diff(&before, &after, iron_diff_cad::DiffOptions::default()),
        warnings,
    )
}
