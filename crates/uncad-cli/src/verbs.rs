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
//! how an edit's result is the next call's input. Two verbs write a file:
//! `set` the edited model JSON, `redline` a picture of the edit. Neither
//! writes over an existing file, and neither changes its input.
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
    /// A finite number greater than 0.
    Positive,
    /// One of the listed words.
    Choice(&'static [&'static str]),
    /// Some of the listed words, each once. The command line takes them
    /// comma-separated.
    Choices(&'static [&'static str]),
    /// A color as `#rrggbb`: six hexadecimal digits after a `#`.
    Color,
    /// A box as four finite numbers `[x0, y0, x1, y1]`, `x0 <= x1` and
    /// `y0 <= y1`. The command line takes them comma-separated.
    Window,
    /// On or off. The command line takes the bare flag for on.
    Flag,
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
        description: "What a drawing contains: the unit its header states (null when it states \
            none), the entity count per type, every layer and block definition, the \
            attribute values on block references, loose texts that read as label and value, \
            and the lowest confidence of anything summarized. A value that several places \
            give differently is listed with every value, never one of them. \
            Given a type, a layer, a box, a space or an ID, it also selects entities -- to find \
            them by what they are rather than where -- and with `detail` gives each one's \
            model record, the fields `set` addresses.",
        params: &[
            Param {
                name: "input",
                kind: Kind::Path,
                required: true,
                positional: true,
                description: DRAWING,
            },
            Param {
                name: "type",
                kind: Kind::Text,
                required: false,
                positional: false,
                description: "Select the entities of this type (CIRCLE, LINE ...), matched \
                    without regard to case. Any selection argument adds `selection` to the \
                    answer: the entities every given filter keeps, each with its layer, space \
                    and box, and `total` counting them all.",
            },
            Param {
                name: "layer",
                kind: Kind::Text,
                required: false,
                positional: false,
                description: "Select the entities on this layer, matched without regard to \
                    case.",
            },
            Param {
                name: "within",
                kind: Kind::Window,
                required: false,
                positional: false,
                description: "Select the entities that reach into this box, [x0, y0, x1, y1] \
                    in drawing units (a crossing selection). An entity whose extent is not \
                    measured cannot be judged: it is left out and its type named in \
                    `not_measured`.",
            },
            Param {
                name: "space",
                kind: Kind::Choice(&["model", "paper"]),
                required: false,
                positional: false,
                description: "Select the entities of model space, or of the paper-space \
                    sheets.",
            },
            Param {
                name: "id",
                kind: Kind::Integer,
                required: false,
                positional: false,
                description: "Select the entity with this reference ID -- with `detail`, to \
                    read it.",
            },
            Param {
                name: "limit",
                kind: Kind::Integer,
                required: false,
                positional: false,
                description: "Select at most this many entities, the first by reference ID \
                    (default 100); `total` still counts all of them.",
            },
            Param {
                name: "detail",
                kind: Kind::Flag,
                required: false,
                positional: false,
                description: "Give each selected entity's model record too: its fields as \
                    the model JSON names them, which are the `path`s `set` takes.",
            },
        ],
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
            Param {
                name: "limit",
                kind: Kind::Integer,
                required: false,
                positional: false,
                description: "Keep at most this many hits, the nearest; `hits_total` then \
                    says how many there were. A wide search in a dense drawing otherwise \
                    answers with most of it.",
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
            MATCHING,
            LENGTH_TOLERANCE,
            ANGLE_TOLERANCE,
            OMIT,
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
            OMIT,
        ],
        writes: true,
        run: set,
    },
    Verb {
        name: "redline",
        description: "Draws the difference between two drawing states on top of the first: the \
            first drawing exactly as it renders alone, and over it, in red (or the color given), what each \
            changed entity became, what was removed (dashed), and a revision cloud around every change. \
            A change whose counterpart is uncertain gets a dashed cloud and no geometry. Writes \
            an SVG or PNG file -- never over an existing one -- and answers with what it marked \
            and what it could not (inside a block definition, or nothing drawn), each with the \
            change's index in the change set `diff` gives for the same arguments.",
        params: &[
            Param {
                name: "before",
                kind: Kind::Path,
                required: true,
                positional: true,
                description: "The drawing as it is (.dwg, .dxf, or model JSON .json): drawn \
                    unchanged.",
            },
            Param {
                name: "after",
                kind: Kind::Path,
                required: true,
                positional: true,
                description: "The drawing as proposed (.dwg, .dxf, or model JSON .json) -- \
                    what `set` writes.",
            },
            Param {
                name: "output",
                kind: Kind::Path,
                required: true,
                positional: false,
                description: "Where to write the picture: a .svg or .png path that does not \
                    exist yet.",
            },
            Param {
                name: "fit",
                kind: Kind::Integer,
                required: false,
                positional: false,
                description: "For a PNG: make the longer side this many pixels. Without it a \
                    drawing unit is a pixel, which a large drawing exceeds.",
            },
            Param {
                name: "stroke",
                kind: Kind::Positive,
                required: false,
                positional: false,
                description: "For a PNG: draw every line this many pixels wide. Without it the \
                    lines are about 1/6000 of the picture's diagonal, under a pixel at most \
                    sizes. An SVG has no pixels, so it does not take this.",
            },
            Param {
                name: "paper",
                kind: Kind::Choice(&["light", "dark"]),
                required: false,
                positional: false,
                description: "The page: light (default; pure white drawn black) or dark (a black \
                    page; pure white kept white, pure black drawn white). Every other color, \
                    the changes' included, is drawn as it is.",
            },
            Param {
                name: "frame",
                kind: Kind::Choice(&["drawing", "changes"]),
                required: false,
                positional: false,
                description: "What the picture shows: the whole drawing (default), or the \
                    changes with some of the drawing around them -- in a large drawing a small \
                    change is otherwise a few pixels of the picture.",
            },
            Param {
                name: "proposal_color",
                kind: Kind::Color,
                required: false,
                positional: false,
                description: "The color the changes are drawn in (default #e4002b, a red). \
                    Give another when the drawing itself uses colors close to it -- the answer \
                    lists those as proposal_color_conflicts.",
            },
            MATCHING,
            LENGTH_TOLERANCE,
            ANGLE_TOLERANCE,
            OMIT,
        ],
        writes: true,
        run: redline,
    },
];

const DRAWING: &str = "The drawing (.dwg, .dxf, or model JSON .json).";

/// How the two drawings of a comparison are paired.
const MATCHING: Param = Param {
    name: "matching",
    kind: Kind::Choice(&["reference", "geometry"]),
    required: false,
    positional: false,
    description: "How entities of the two drawings are paired (default: reference).",
};

const LENGTH_TOLERANCE: Param = Param {
    name: "length_tolerance",
    kind: Kind::NonNegative,
    required: false,
    positional: false,
    description: "Tolerance for every numeric field except angles, in drawing units (default: \
        1e-6).",
};

const ANGLE_TOLERANCE: Param = Param {
    name: "angle_tolerance",
    kind: Kind::NonNegative,
    required: false,
    positional: false,
    description: "Tolerance for angles, in radians (default: 1e-9).",
};

/// The projection a change-set answer may ask for.
const OMIT: Param = Param {
    name: "omit",
    kind: Kind::Choices(&["within", "unstated"]),
    required: false,
    positional: false,
    description: "Field changes to leave out of the answer: `within` (numeric fields that moved \
        within tolerance), `unstated` (fields one drawing does not state -- a newer format \
        states what an older one had no place for). A modified entity left with no field is \
        left out too. The answer counts what was left out in `omitted`. Default: nothing is \
        left out.",
};

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
                Kind::Positive => value.as_f64().is_some_and(|v| v.is_finite() && v > 0.0),
                Kind::Choice(words) => value.as_str().is_some_and(|s| words.contains(&s)),
                Kind::Color => value.as_str().and_then(color).is_some(),
                Kind::Window => window(value).is_some(),
                Kind::Flag => value.is_boolean(),
                Kind::Choices(words) => value.as_array().is_some_and(|items| {
                    items
                        .iter()
                        .all(|i| i.as_str().is_some_and(|s| words.contains(&s)))
                        && items
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<std::collections::BTreeSet<_>>()
                            .len()
                            == items.len()
                }),
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
                Kind::Positive => {
                    schema.insert("type".into(), "number".into());
                    schema.insert("exclusiveMinimum".into(), 0.into());
                }
                Kind::Choice(words) => {
                    schema.insert("type".into(), "string".into());
                    schema.insert("enum".into(), (*words).into());
                }
                Kind::Color => {
                    schema.insert("type".into(), "string".into());
                    schema.insert("pattern".into(), "^#[0-9A-Fa-f]{6}$".into());
                }
                Kind::Window => {
                    let mut item = Map::new();
                    item.insert("type".into(), "number".into());
                    schema.insert("type".into(), "array".into());
                    schema.insert("items".into(), item.into());
                    schema.insert("minItems".into(), 4.into());
                    schema.insert("maxItems".into(), 4.into());
                }
                Kind::Flag => {
                    schema.insert("type".into(), "boolean".into());
                }
                Kind::Choices(words) => {
                    let mut item = Map::new();
                    item.insert("type".into(), "string".into());
                    item.insert("enum".into(), (*words).into());
                    schema.insert("type".into(), "array".into());
                    schema.insert("items".into(), item.into());
                    schema.insert("uniqueItems".into(), true.into());
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
                if matches!(param.kind, Kind::Flag) {
                    args.insert(param.name.into(), Value::Bool(true));
                    i += 1;
                    continue;
                }
                i += 1;
                let raw = argv.get(i).ok_or_else(|| format!("{word} needs a value"))?;
                let value = match param.kind {
                    Kind::Number | Kind::NonNegative | Kind::Positive => raw
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
                    Kind::Window => Value::Array(
                        raw.split(',')
                            .map(|n| {
                                n.trim()
                                    .parse::<f64>()
                                    .ok()
                                    .and_then(serde_json::Number::from_f64)
                                    .map(Value::Number)
                                    .ok_or_else(|| {
                                        format!(
                                            "{word} must be four numbers x0,y0,x1,y1 (got '{raw}')"
                                        )
                                    })
                            })
                            .collect::<Result<_, _>>()?,
                    ),
                    Kind::Flag => unreachable!("a flag takes no value"),
                    Kind::Choices(_) => Value::Array(
                        raw.split(',')
                            .map(|w| Value::String(w.trim().to_string()))
                            .collect(),
                    ),
                    Kind::Path | Kind::Text | Kind::Choice(_) | Kind::Color => {
                        Value::String(raw.clone())
                    }
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
            } else if matches!(param.kind, Kind::Flag) {
                format!("--{}", cli_spelling(param.name))
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
            Kind::Positive => "a finite number greater than 0".into(),
            Kind::Choice(words) => format!("one of {}", words.join(", ")),
            Kind::Choices(words) => format!("a list of distinct words from {}", words.join(", ")),
            Kind::Color => "a color as #rrggbb".into(),
            Kind::Window => "four finite numbers [x0, y0, x1, y1], x0 <= x1 and y0 <= y1".into(),
            Kind::Flag => "true or false".into(),
        }
    }

    fn placeholder(&self) -> String {
        match self {
            Kind::Path => "path".into(),
            Kind::Integer | Kind::Number | Kind::NonNegative | Kind::Positive => "n".into(),
            Kind::Text => "text".into(),
            Kind::Json => "json".into(),
            Kind::Choice(words) => words.join("|"),
            Kind::Choices(words) => format!("{}[,...]", words.join("|")),
            Kind::Color => "#rrggbb".into(),
            Kind::Window => "x0,y0,x1,y1".into(),
            Kind::Flag => String::new(),
        }
    }
}

/// The box of a `Kind::Window` value, or `None` for anything else.
fn window(value: &Value) -> Option<iron_scout_cad::Bounds> {
    let n: Vec<f64> = value
        .as_array()?
        .iter()
        .map(|v| v.as_f64().filter(|x| x.is_finite()))
        .collect::<Option<_>>()?;
    let [x0, y0, x1, y1] = n[..] else {
        return None;
    };
    (x0 <= x1 && y0 <= y1).then_some(iron_scout_cad::Bounds {
        min: uncad::model::Point2D { x: x0, y: y0 },
        max: uncad::model::Point2D { x: x1, y: y1 },
    })
}

/// The red, green and blue of a `#rrggbb` color, or `None` for anything else.
fn color(text: &str) -> Option<[u8; 3]> {
    let hex = text
        .strip_prefix('#')
        .filter(|h| h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit()))?;
    let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
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
pub fn read(input: &str, warnings: &mut Vec<String>) -> Result<CadDatabase, String> {
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

/// `changes` projected as the `omit` argument asks, or as it is.
fn projected(
    changes: iron_diff_cad::ChangeSet,
    args: &Map<String, Value>,
) -> iron_diff_cad::ChangeSet {
    let Some(words) = args.get("omit").and_then(Value::as_array) else {
        return changes;
    };
    let has = |w: &str| words.iter().any(|v| v == w);
    changes.without(iron_diff_cad::Omit {
        within: has("within"),
        unstated: has("unstated"),
    })
}

fn answer(result: &impl serde::Serialize, warnings: Vec<String>) -> Result<Answer, String> {
    let json = serde_json::to_string(result).map_err(|e| e.to_string())?;
    Ok(Answer { json, warnings })
}

/// How many entities a selection shows when the call does not say.
const SELECTION_LIMIT: u64 = 100;

fn summarize(args: &Map<String, Value>) -> Result<Answer, String> {
    let mut warnings = Vec::new();
    let db = read(&path(args, "input"), &mut warnings)?;
    let selecting = ["type", "layer", "within", "space", "id", "limit", "detail"]
        .iter()
        .any(|name| args.contains_key(*name));
    if !selecting {
        return answer(&iron_scout_cad::summarize(&db), warnings);
    }
    let text = |name: &str| args.get(name).and_then(Value::as_str);
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(SELECTION_LIMIT);
    let mut selection = iron_scout_cad::Selection::new().limit(limit as usize);
    if let Some(t) = text("type") {
        selection = selection.of_type(t);
    }
    if let Some(l) = text("layer") {
        selection = selection.on_layer(l);
    }
    if let Some(w) = args.get("within") {
        selection = selection.crossing(window(w).expect("checked by Verb::call"));
    }
    match text("space") {
        Some("model") => selection = selection.in_space(iron_scout_cad::SpaceFilter::Model),
        Some("paper") => selection = selection.in_space(iron_scout_cad::SpaceFilter::Paper),
        _ => {}
    }
    if let Some(id) = args.get("id").and_then(Value::as_u64) {
        let id: uncad::model::EntityId =
            serde_json::from_value(Value::from(id)).expect("a reference ID is an integer");
        selection = selection.with_ids([id]);
    }
    if args.get("detail").and_then(Value::as_bool) == Some(true) {
        selection = selection.with_detail();
    }
    answer(&iron_scout_cad::summarize_with(&db, &selection), warnings)
}

fn hit_test(args: &Map<String, Value>) -> Result<Answer, String> {
    let mut warnings = Vec::new();
    let db = read(&path(args, "input"), &mut warnings)?;
    let point = uncad::model::Point2D {
        x: number(args, "x").expect("required"),
        y: number(args, "y").expect("required"),
    };
    let tolerance = number(args, "tolerance").expect("required");
    let mut found = iron_scout_cad::hit_test(&db, point, tolerance);
    if let Some(n) = args.get("limit").and_then(Value::as_u64) {
        found = found.limited(n as usize);
    }
    answer(&found, warnings)
}

fn diff(args: &Map<String, Value>) -> Result<Answer, String> {
    let mut warnings = Vec::new();
    let before = read(&path(args, "before"), &mut warnings)?;
    let after = read(&path(args, "after"), &mut warnings)?;
    answer(&compare(&before, &after, args), warnings)
}

/// The change set from `before` to `after` under the comparison arguments
/// (`matching`, the tolerances), projected as `omit` asks.
fn compare(
    before: &CadDatabase,
    after: &CadDatabase,
    args: &Map<String, Value>,
) -> iron_diff_cad::ChangeSet {
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
    projected(iron_diff_cad::diff(before, after, options), args)
}

/// A new file at `output`, refused when one is there already -- checked
/// again, atomically, by the write itself.
fn create_new(output: &str, bytes: &[u8]) -> Result<(), String> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|e| format!("cannot write '{output}': {e}"))?;
    std::io::Write::write_all(&mut file, bytes).map_err(|e| format!("cannot write '{output}': {e}"))
}

fn redline(args: &Map<String, Value>) -> Result<Answer, String> {
    let output = path(args, "output");
    let extension = std::path::Path::new(&output)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let png = match extension.as_deref() {
        Some("svg") => false,
        Some("png") => true,
        _ => {
            return Err(format!(
                "redline writes SVG or PNG: '{output}' ends in neither .svg nor .png"
            ))
        }
    };
    let stroke_px = args.get("stroke").and_then(Value::as_f64);
    // Checked before anything is read, so a refused call costs nothing.
    if stroke_px.is_some() && !png {
        return Err(format!(
            "--stroke is a width in pixels, and an SVG has none: write a .png to set it, or \
             leave it out for '{output}' (MCP: stroke)"
        ));
    }
    if std::path::Path::new(&output).exists() {
        return Err(format!(
            "'{output}' already exists -- redline never writes over a file; give a new path"
        ));
    }
    let fit = args.get("fit").and_then(Value::as_u64);
    let mut warnings = Vec::new();
    let before = read(&path(args, "before"), &mut warnings)?;
    let after = read(&path(args, "after"), &mut warnings)?;
    let changes = compare(&before, &after, args);
    let mut options = iron_render_cad::OverlayOptions::default();
    if args.get("frame").and_then(Value::as_str) == Some("changes") {
        options.frame = iron_render_cad::OverlayFrame::Changes;
    }
    if let Some(given) = args.get("proposal_color").and_then(Value::as_str) {
        options.proposal_color = color(given).expect("checked by Verb::call");
    }
    if args.get("paper").and_then(Value::as_str) == Some("dark") {
        options.svg.paper = iron_render_cad::Paper::Dark;
    }
    let mut overlay = iron_render_cad::overlay_to_svg(&before, &after, &changes, options);
    if let Some(px) = stroke_px {
        // The overlay is drawn at a stroke width in drawing units, and the
        // picture's pixels per unit follow from the window it frames -- which
        // grows with the stroke when a revision cloud reaches past the
        // drawing (a cloud's margin is a multiple of the stroke). Draw at
        // the width the pixels ask for until the window stops moving: once
        // when no cloud reaches out, a few times at most otherwise.
        for _ in 0..STROKE_ROUNDS {
            let units = px / px_per_unit(&overlay, fit)?;
            if (overlay.stroke_width - units).abs() <= units * 1e-9 {
                break;
            }
            options.svg.stroke_width = Some(units);
            overlay = iron_render_cad::overlay_to_svg(&before, &after, &changes, options);
        }
    }
    if !overlay.proposal_color_conflicts.is_empty() {
        let colors: Vec<String> = overlay
            .proposal_color_conflicts
            .iter()
            .map(|c| format!("{} ({} uses)", c.color, c.uses))
            .collect();
        let [r, g, b] = options.proposal_color;
        warnings.push(format!(
            "the original is drawn in colors close to #{r:02x}{g:02x}{b:02x}, the color the \
             changes are drawn in ({}): a change may not stand out from the lines around it \
             -- draw the changes in another color with --proposal-color <#rrggbb> \
             (MCP: proposal_color)",
            colors.join(", ")
        ));
    }
    if !overlay.left_out.is_empty() {
        let names: Vec<String> = overlay
            .left_out
            .iter()
            .map(|l| format!("{} {}", l.type_name, l.id.value()))
            .collect();
        warnings.push(format!(
            "the original is drawn without the entities far larger or farther than the rest \
             of the drawing (listed in left_out): {}",
            names.join(", ")
        ));
    }
    if png {
        let scale = px_per_unit(&overlay, fit)?;
        let bytes = iron_render_cad::svg_to_png(&overlay.svg, scale as f32)
            .map_err(|e| format!("{e} -- ask for a smaller picture with --fit <px> (MCP: fit)"))?;
        create_new(&output, &bytes)?;
    } else {
        create_new(&output, overlay.svg.as_bytes())?;
    }
    answer(&overlay, warnings)
}

/// How many times redline draws the overlay again to settle a stroke given
/// in pixels. The window grows by twenty strokes at most (a cloud's margin
/// on both sides), so each round moves the width by that over the picture's
/// size in pixels -- a few percent for a thick line in a small picture --
/// and a handful of rounds settles it far below a pixel.
const STROKE_ROUNDS: usize = 8;

/// A redline picture's pixels per drawing unit: the longer side of its
/// window at `fit` pixels, or one.
fn px_per_unit(overlay: &iron_render_cad::OverlayResult, fit: Option<u64>) -> Result<f64, String> {
    let Some(fit) = fit else { return Ok(1.0) };
    let longer = overlay.view_box.width().max(overlay.view_box.height());
    if !(longer.is_finite() && longer > 0.0) || fit == 0 {
        return Err(format!(
            "cannot fit a picture of size {longer} into {fit} px"
        ));
    }
    Ok(fit as f64 / longer)
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
    create_new(&output, json.as_bytes())?;
    answer(
        &projected(
            iron_diff_cad::diff(&before, &after, iron_diff_cad::DiffOptions::default()),
            args,
        ),
        warnings,
    )
}
