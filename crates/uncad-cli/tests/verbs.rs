//! The verbs (`summarize`, `hit-test`, `diff`, `set`, `redline`) and
//! `uncad mcp`, which serves the same verbs as MCP tools.
//!
//! The property that matters most is that the two front ends give the same
//! answer: a tool result's first content block must be byte for byte what
//! the command line prints for the same arguments. Everything else stays
//! reference-free, as in `documented_invocations.rs`: that an answer is
//! JSON of the documented shape, that an option changes the answer, and
//! that a bad call fails with a message -- no bytes or counts copied out of
//! this project's own output.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

const EXE: &str = env!("CARGO_BIN_EXE_uncad");

const CORPUS_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/2000/circle.dwg"
);
const CORPUS_DXF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/2000/entities-2d.dxf"
);

fn run(args: &[&str]) -> Output {
    Command::new(EXE)
        .args(args)
        .output()
        .expect("the test binary should be runnable")
}

/// The command's stdout as one JSON document, after checking it succeeded.
fn answer(args: &[&str]) -> (String, Value) {
    let out = run(args);
    assert!(
        out.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("the answer is UTF-8");
    let line = stdout
        .strip_suffix('\n')
        .expect("the answer ends with a newline");
    assert!(!line.contains('\n'), "the answer is one line: {stdout}");
    let value = serde_json::from_str(line).expect("the answer is JSON");
    (line.to_string(), value)
}

/// The DWG fixture's model export, as JSON. Each call writes its own file --
/// the tests of this binary run in parallel in one process.
fn exported_model() -> Value {
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    let model = std::env::temp_dir().join(format!(
        "uncad-cli-verbs-{}-{}.json",
        std::process::id(),
        CALLS.fetch_add(1, Ordering::Relaxed)
    ));
    let model_arg = model.to_str().expect("temp paths are UTF-8 here");
    assert!(run(&[CORPUS_DWG, "-o", model_arg]).status.success());
    let text = std::fs::read_to_string(&model).expect("the export was written");
    let _ = std::fs::remove_file(&model);
    serde_json::from_str(&text).expect("the export is JSON")
}

/// The one circle of the DWG fixture, read from the model export: the
/// point tests aim at it without copying its numbers here.
fn circle() -> (f64, f64, f64) {
    let db = exported_model();
    let circle = db["entities"]
        .as_array()
        .expect("entities")
        .iter()
        .find(|e| e["type"] == "CIRCLE")
        .expect("the fixture has a circle");
    (
        circle["center"]["x"].as_f64().unwrap(),
        circle["center"]["y"].as_f64().unwrap(),
        circle["radius"].as_f64().unwrap(),
    )
}

#[test]
fn summarize_answers_with_the_summary() {
    let (_, summary) = answer(&["summarize", CORPUS_DXF]);
    assert!(summary["entity_count"].as_u64().unwrap() > 0, "{summary}");
    assert!(summary["by_type"].is_object(), "{summary}");
    assert!(summary["layers"].is_array(), "{summary}");
}

#[test]
fn a_summary_states_the_units_the_model_carries() {
    let db = exported_model();
    let stated = &db["header"]["insunits"];
    assert!(
        stated.is_u64(),
        "an R2000 DWG states $INSUNITS: {}",
        db["header"]
    );

    let (_, summary) = answer(&["summarize", CORPUS_DWG]);
    assert_eq!(&summary["units"]["code"], stated, "{summary}");
    assert!(summary["units"]["name"].is_string(), "{summary}");
}

#[test]
fn hit_test_finds_the_circle_on_its_edge_and_not_away_from_it() {
    let (cx, cy, r) = circle();
    let on_edge = [
        "hit-test",
        CORPUS_DWG,
        "--x",
        &(cx + r).to_string(),
        "--y",
        &cy.to_string(),
        "--tolerance",
        &(r / 100.0).to_string(),
    ];
    let (_, hit) = answer(&on_edge);
    assert_eq!(hit["hits"].as_array().unwrap().len(), 1, "{hit}");

    // Far outside: no hit, and not enclosed either.
    let away = [
        "hit-test",
        CORPUS_DWG,
        "--x",
        &(cx + 10.0 * r).to_string(),
        "--y",
        &cy.to_string(),
        "--tolerance",
        &(r / 100.0).to_string(),
    ];
    let (_, miss) = answer(&away);
    assert!(miss["hits"].as_array().unwrap().is_empty(), "{miss}");
    assert!(miss["enclosing"].as_array().unwrap().is_empty(), "{miss}");
}

#[test]
fn hit_test_has_no_default_tolerance() {
    let out = run(&["hit-test", CORPUS_DWG, "--x", "0", "--y", "0"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("tolerance"), "{stderr}");
}

#[test]
fn diff_of_a_drawing_with_itself_is_empty_and_carries_its_tolerance() {
    let (_, same) = answer(&["diff", CORPUS_DXF, CORPUS_DXF]);
    assert!(same["changes"].as_array().unwrap().is_empty(), "{same}");
    assert_eq!(same["matching"], "REFERENCE");

    // The options reach the diff: both are written into the answer.
    let (_, given) = answer(&[
        "diff",
        CORPUS_DXF,
        CORPUS_DXF,
        "--matching",
        "geometry",
        "--length-tolerance",
        "0.5",
    ]);
    assert_eq!(given["matching"], "GEOMETRY");
    assert_eq!(given["tolerance"]["length"], 0.5);
}

#[test]
fn a_bad_call_fails_with_a_message() {
    for args in [
        vec!["diff", CORPUS_DXF],
        vec!["diff", CORPUS_DXF, CORPUS_DXF, "--matching", "nearest"],
        vec!["diff", CORPUS_DXF, CORPUS_DXF, "--length-tolerance", "-1"],
        vec!["summarize", CORPUS_DXF, "--no-such-option"],
        vec!["summarize", CORPUS_DXF, CORPUS_DXF],
        vec!["summarize", "no-such-file.dwg"],
    ] {
        let out = run(&args);
        assert!(!out.status.success(), "{args:?} should fail");
        assert!(
            String::from_utf8_lossy(&out.stderr).starts_with("error: "),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.stdout.is_empty(), "{args:?} printed an answer");
    }
}

/// A running `uncad mcp`, spoken to one request at a time.
struct Mcp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Mcp {
    fn start() -> Self {
        let mut child = Command::new(EXE)
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("the test binary should be runnable");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut mcp = Mcp {
            child,
            stdin,
            stdout,
            next_id: 1,
        };
        let init = mcp.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "0"}
            }),
        );
        assert_eq!(init["result"]["serverInfo"]["name"], "uncad", "{init}");
        mcp.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        mcp
    }

    fn send(&mut self, message: Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    /// Sends a request and returns its response, skipping notifications.
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let mut line = String::new();
            let read = self.stdout.read_line(&mut line).unwrap();
            assert!(
                read > 0,
                "the server closed stdout before answering {method}"
            );
            let message: Value = serde_json::from_str(&line).expect("a JSON-RPC message");
            if message["id"] == id {
                return message;
            }
        }
    }

    fn call(&mut self, tool: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name": tool, "arguments": arguments}))["result"].clone()
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A client on protocol version 2026-07-28 opens no session: it asks
/// `server/discover`, then names the version in every request's `_meta`.
/// That version requires every list result to say how long it stays fresh
/// (`ttlMs`) and who may cache it (`cacheScope`) -- a client validating
/// the schema drops a list without them, and with it every tool.
#[test]
fn mcp_lists_the_verbs_to_a_client_on_the_stateless_protocol() {
    let mut child = Command::new(EXE)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("the test binary should be runnable");
    let stdin = child.stdin.take().unwrap();
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let mut mcp = Mcp {
        child,
        stdin,
        stdout,
        next_id: 1,
    };
    let meta = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": {"name": "test", "version": "0"},
        "io.modelcontextprotocol/clientCapabilities": {}
    });
    let discover = mcp.request("server/discover", json!({"_meta": meta}));
    let versions = discover["result"]["supportedVersions"]
        .as_array()
        .expect("supportedVersions");
    assert!(versions.contains(&json!("2026-07-28")), "{discover}");

    let list = mcp.request("tools/list", json!({"_meta": meta}));
    let result = &list["result"];
    assert!(result["ttlMs"].is_u64(), "ttlMs is a number: {list}");
    assert!(
        result["cacheScope"] == "public" || result["cacheScope"] == "private",
        "cacheScope is public or private: {list}"
    );
    assert_eq!(
        result["tools"].as_array().expect("tools").len(),
        5,
        "{list}"
    );
}

#[test]
fn mcp_lists_the_verbs_and_only_set_and_redline_write() {
    let mut mcp = Mcp::start();
    let list = mcp.request("tools/list", json!({}));
    let tools = list["result"]["tools"].as_array().expect("tools");
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["summarize", "hit_test", "diff", "set", "redline"]);
    for tool in tools {
        let writes = tool["name"] == "set" || tool["name"] == "redline";
        let description = tool["description"].as_str().unwrap();
        assert!(
            !description.contains('\\'),
            "no stray backslash: {description}"
        );
        assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
        assert_eq!(tool["annotations"]["readOnlyHint"], !writes, "{tool}");
        assert_eq!(tool["annotations"]["idempotentHint"], !writes, "{tool}");
        // A new file, never one written over: not destructive.
        assert_eq!(tool["annotations"]["destructiveHint"], false, "{tool}");
    }

    // `--help` names the tools `uncad mcp` serves, as the command spells
    // them, and no others: a verb added to one list and not the other
    // fails here.
    let help = String::from_utf8(run(&["--help"]).stdout).expect("UTF-8 help");
    let mcp_line: String = help
        .lines()
        .skip_while(|line| !line.trim_start().starts_with("uncad mcp"))
        .take_while(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let named: Vec<&str> = ["summarize", "hit-test", "diff", "set", "redline", "export"]
        .into_iter()
        .filter(|verb| {
            mcp_line
                .split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
                .any(|word| word == *verb)
        })
        .collect();
    let served: Vec<String> = names.iter().map(|n| n.replace('_', "-")).collect();
    assert_eq!(named, served, "{mcp_line}");
}

#[test]
fn a_tool_answers_with_the_same_bytes_as_the_command_line() {
    let (cx, cy, r) = circle();
    let (x, y, tolerance) = (cx + r, cy, r / 100.0);
    let cases: Vec<(Vec<String>, &str, Value)> = vec![
        (
            vec!["summarize".into(), CORPUS_DXF.into()],
            "summarize",
            json!({"input": CORPUS_DXF}),
        ),
        (
            vec![
                "hit-test".into(),
                CORPUS_DWG.into(),
                "--x".into(),
                x.to_string(),
                "--y".into(),
                y.to_string(),
                "--tolerance".into(),
                tolerance.to_string(),
            ],
            "hit_test",
            json!({"input": CORPUS_DWG, "x": x, "y": y, "tolerance": tolerance}),
        ),
        (
            vec![
                "diff".into(),
                CORPUS_DWG.into(),
                CORPUS_DXF.into(),
                "--matching".into(),
                "geometry".into(),
            ],
            "diff",
            json!({"before": CORPUS_DWG, "after": CORPUS_DXF, "matching": "geometry"}),
        ),
    ];
    let mut mcp = Mcp::start();
    for (argv, tool, arguments) in cases {
        let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
        let (cli, _) = answer(&argv);
        let result = mcp.call(tool, arguments);
        assert_eq!(result["isError"], false, "{tool}: {result}");
        assert_eq!(
            result["content"][0]["text"].as_str().unwrap(),
            cli,
            "{tool}: the tool and `uncad {}` disagree",
            argv[0]
        );
    }
}

#[test]
fn a_bad_tool_call_is_an_error_result_the_caller_can_read() {
    let mut mcp = Mcp::start();
    let result = mcp.call("hit_test", json!({"input": CORPUS_DWG, "x": 0, "y": 0}));
    assert_eq!(result["isError"], true, "{result}");
    let message = result["content"][0]["text"].as_str().unwrap();
    assert!(message.contains("tolerance"), "{message}");

    let result = mcp.call("summarize", json!({"input": CORPUS_DXF, "pretty": true}));
    assert_eq!(result["isError"], true, "{result}");

    // An unknown tool is a protocol error, not a result.
    let response = mcp.request("tools/call", json!({"name": "rotate", "arguments": {}}));
    assert!(response["error"].is_object(), "{response}");
}

#[test]
fn a_render_names_the_leaders_it_could_not_draw() {
    // The drawing has a LEADER whose path is a spline: the file states no
    // curve, so the render leaves it out and says which one.
    let leaders = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../lib/libredwg/test/test-data/2010/Leader.dwg"
    );
    let dir = scratch("undefined-leaders");
    let model = dir.join("model.json");
    assert!(run(&[leaders, "-o", arg(&model)]).status.success());
    let db: Value = serde_json::from_str(&std::fs::read_to_string(&model).unwrap()).unwrap();
    let splines: Vec<String> = db["entities"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["type"] == "LEADER" && e["path_type"] == "SPLINE")
        .map(|e| e["common"]["id"].to_string())
        .collect();
    assert!(
        !splines.is_empty(),
        "the drawing should have a spline leader"
    );

    let out = run(&[leaders, "-o", arg(&dir.join("drawing.svg"))]);
    assert!(out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    let line = stderr
        .lines()
        .find(|l| l.starts_with("warning: leaders not drawn"))
        .unwrap_or_else(|| panic!("no warning names them: {stderr}"));
    for id in &splines {
        assert!(line.contains(id.as_str()), "{line} should name {id}");
    }
}

/// A directory of its own for one test's files, empty.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("uncad-cli-set-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the scratch directory is created");
    dir
}

fn arg(path: &std::path::Path) -> &str {
    path.to_str().expect("temp paths are UTF-8 here")
}

/// The reference ID of the DWG fixture's one circle, from the model export.
fn circle_id(dir: &std::path::Path) -> String {
    let model = dir.join("model.json");
    assert!(run(&[CORPUS_DWG, "-o", arg(&model)]).status.success());
    let db: Value = serde_json::from_str(&std::fs::read_to_string(&model).unwrap())
        .expect("the export is JSON");
    db["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["type"] == "CIRCLE")
        .expect("the fixture has a circle")["common"]["id"]
        .to_string()
}

#[test]
fn set_chains_through_model_json_and_diff_shows_only_the_edits() {
    let dir = scratch("chain");
    let id = circle_id(&dir);
    let original = std::fs::read(CORPUS_DWG).unwrap();
    let (first, second) = (dir.join("first.json"), dir.join("second.json"));

    let (_, answer1) = answer(&[
        "set",
        CORPUS_DWG,
        "--id",
        &id,
        "--path",
        "radius",
        "--value",
        "7.25",
        "-o",
        arg(&first),
    ]);
    // The answer is the change set of the one edit.
    let changes = answer1["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1, "{answer1}");
    let paths: Vec<&str> = changes[0]["data"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["radius"]);

    // The next edit starts from the first one's output.
    answer(&[
        "set",
        arg(&first),
        "--id",
        &id,
        "--path",
        "center.x",
        "--value",
        "-3",
        "-o",
        arg(&second),
    ]);
    let (_, both) = answer(&["diff", CORPUS_DWG, arg(&second)]);
    let changes = both["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1, "{both}");
    let fields: Vec<(&str, &Value)> = changes[0]["data"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["path"].as_str().unwrap(), &f["after"]))
        .collect();
    assert_eq!(
        fields,
        [("center.x", &json!(-3.0)), ("radius", &json!(7.25))]
    );

    // The input is never touched.
    assert_eq!(std::fs::read(CORPUS_DWG).unwrap(), original);
}

#[test]
fn set_writes_nothing_it_should_not() {
    let dir = scratch("refuse");
    let id = circle_id(&dir);
    let taken = dir.join("taken.json");
    std::fs::write(&taken, "keep").unwrap();
    let set = |path: &str, value: &str, output: &str| {
        run(&[
            "set", CORPUS_DWG, "--id", &id, "--path", path, "--value", value, "-o", output,
        ])
    };

    // An existing file -- the input included -- is never written over.
    let out = set("radius", "2", arg(&taken));
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("already exists"));
    assert_eq!(std::fs::read_to_string(&taken).unwrap(), "keep");

    // Model JSON only.
    let dxf = dir.join("out.dxf");
    assert!(!set("radius", "2", arg(&dxf)).status.success());
    assert!(!dxf.exists());

    // A refused edit names its reason and writes nothing.
    let refused = dir.join("refused.json");
    let out = set("radius", "-1", arg(&refused));
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("refused (CONSTRAINT)"), "{stderr}");
    assert!(!refused.exists());

    // A value is JSON: a bare word is not a string.
    let out = set("common.layer", "HIDDEN", arg(&refused));
    assert!(!out.status.success());
    assert!(!refused.exists());
}

#[test]
fn set_as_a_tool_answers_as_the_command_line_does() {
    let dir = scratch("mcp");
    let id = circle_id(&dir);
    let (by_cli, by_tool) = (dir.join("cli.json"), dir.join("tool.json"));
    let (cli, _) = answer(&[
        "set",
        CORPUS_DWG,
        "--id",
        &id,
        "--path",
        "radius",
        "--value",
        "3",
        "-o",
        arg(&by_cli),
    ]);
    let mut mcp = Mcp::start();
    let result = mcp.call(
        "set",
        json!({"input": CORPUS_DWG, "id": id.parse::<u64>().unwrap(), "path": "radius",
               "value": 3, "output": arg(&by_tool)}),
    );
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(result["content"][0]["text"].as_str().unwrap(), cli);
    // The same edit writes the same bytes.
    assert_eq!(
        std::fs::read(&by_cli).unwrap(),
        std::fs::read(&by_tool).unwrap()
    );
}

#[test]
fn a_verb_reads_model_json_as_the_drawing_it_was_written_from() {
    let dir = scratch("json");
    let model = dir.join("model.json");
    assert!(run(&[CORPUS_DXF, "-o", arg(&model)]).status.success());
    let (from_drawing, _) = answer(&["summarize", CORPUS_DXF]);
    let (from_json, _) = answer(&["summarize", arg(&model)]);
    assert_eq!(from_json, from_drawing);
    // A command that needs the drawing itself says so.
    let out = run(&["export", arg(&model), "-o", arg(&dir.join("package"))]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("model JSON"));
}

#[test]
fn model_json_renders_as_the_drawing_it_was_written_from() {
    let dir = scratch("render");
    let model = dir.join("model.json");
    assert!(run(&[CORPUS_DWG, "-o", arg(&model)]).status.success());
    let (from_drawing, from_json) = (dir.join("drawing.svg"), dir.join("json.svg"));
    assert!(run(&[CORPUS_DWG, "-o", arg(&from_drawing)])
        .status
        .success());
    let out = run(&[arg(&model), "-o", arg(&from_json)]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        std::fs::read(&from_json).unwrap(),
        std::fs::read(&from_drawing).unwrap()
    );
}

#[test]
fn a_diff_can_leave_out_what_the_caller_does_not_need_and_says_how_much() {
    let (cli, projected) = answer(&[
        "diff",
        CORPUS_DWG,
        CORPUS_DXF,
        "--matching",
        "geometry",
        "--omit",
        "within,unstated",
    ]);
    let omitted = &projected["omitted"];
    assert!(omitted["within_fields"].is_u64(), "{projected}");
    assert!(omitted["unstated_fields"].is_u64(), "{projected}");
    assert!(omitted["entities"].is_u64(), "{projected}");
    let (_, full) = answer(&["diff", CORPUS_DWG, CORPUS_DXF, "--matching", "geometry"]);
    assert!(
        full.get("omitted").is_none(),
        "nothing is left out by default"
    );

    let mut mcp = Mcp::start();
    let result = mcp.call(
        "diff",
        json!({"before": CORPUS_DWG, "after": CORPUS_DXF, "matching": "geometry",
               "omit": ["within", "unstated"]}),
    );
    assert_eq!(result["content"][0]["text"].as_str().unwrap(), cli);

    for bad in [
        json!(["within", "within"]),
        json!(["nearby"]),
        json!("within"),
    ] {
        let result = mcp.call(
            "diff",
            json!({"before": CORPUS_DWG, "after": CORPUS_DXF, "omit": bad}),
        );
        assert_eq!(result["isError"], true, "{result}");
    }
}

#[test]
fn the_skeleton_runs_from_a_drawing_to_a_redline_without_a_hand_on_it() {
    // Read, point at the circle, change its radius, draw the change.
    let dir = scratch("skeleton");
    let (cx, cy, r) = circle();
    let original = std::fs::read(CORPUS_DWG).unwrap();
    let (_, summary) = answer(&["summarize", CORPUS_DWG]);
    assert!(
        summary["by_type"]["CIRCLE"].as_u64().unwrap() >= 1,
        "{summary}"
    );

    let (x, y) = ((cx + r).to_string(), cy.to_string());
    let (_, hits) = answer(&[
        "hit-test",
        CORPUS_DWG,
        "--x",
        &x,
        "--y",
        &y,
        "--tolerance",
        "0.001",
    ]);
    let hit = &hits["hits"][0];
    assert_eq!(hit["entity_type"], "CIRCLE", "{hits}");
    let id = hit["id"].to_string();

    let edited = dir.join("edited.json");
    let smaller = (r / 2.0).to_string();
    answer(&[
        "set",
        CORPUS_DWG,
        "--id",
        &id,
        "--path",
        "radius",
        "--value",
        &smaller,
        "-o",
        arg(&edited),
    ]);

    let picture = dir.join("redline.svg");
    let (_, report) = answer(&["redline", CORPUS_DWG, arg(&edited), "-o", arg(&picture)]);
    let marked = report["marked"].as_array().unwrap();
    assert_eq!(marked.len(), 1, "{report}");
    assert_eq!(marked[0]["kind"], "MODIFIED");
    assert_eq!(marked[0]["before"].to_string(), id);
    assert_eq!(marked[0]["after"].to_string(), id);
    assert!(
        report["not_marked"].as_array().unwrap().is_empty(),
        "{report}"
    );
    assert_eq!(report["left_out"], serde_json::json!([]), "{report}");
    assert!(
        report.get("svg").is_none(),
        "the picture is in the file, not the answer"
    );

    let svg = std::fs::read_to_string(&picture).unwrap();
    assert!(svg.starts_with("<svg"));
    assert!(svg.contains("<g id=\"original\">"));
    assert!(svg.contains("class=\"cloud\""));
    // The drawing it all started from is untouched.
    assert_eq!(std::fs::read(CORPUS_DWG).unwrap(), original);
}

/// A redline of a drawing with a block reference scaled 3256 times beside
/// it frames the drawing, not the giant, and names what its original layer
/// leaves out.
#[test]
fn a_redline_frames_the_drawing_and_names_a_giant_it_sets_aside() {
    const GIANT: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../lib/libredwg/test/test-data/example_2000.dwg"
    );
    let dir = scratch("redline-giant");
    let picture = dir.join("redline.svg");
    let (_, report) = answer(&["redline", GIANT, GIANT, "-o", arg(&picture)]);
    let left_out = report["left_out"].as_array().unwrap();
    assert!(
        left_out
            .iter()
            .any(|l| l["type_name"] == "INSERT" && l["reason"] == "scale_outlier"),
        "{report}"
    );
    let width = report["view_box"]["max_x"].as_f64().unwrap()
        - report["view_box"]["min_x"].as_f64().unwrap();
    // The drawing is some 15 000 units across; the giant, 3.4 million.
    assert!(width < 100_000.0, "{report}");
}

#[test]
fn redline_writes_nothing_it_should_not() {
    let dir = scratch("redline-refuse");
    let taken = dir.join("taken.svg");
    std::fs::write(&taken, "keep").unwrap();
    let out = run(&["redline", CORPUS_DWG, CORPUS_DWG, "-o", arg(&taken)]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("already exists"));
    assert_eq!(std::fs::read_to_string(&taken).unwrap(), "keep");

    let wrong = dir.join("picture.pdf");
    let out = run(&["redline", CORPUS_DWG, CORPUS_DWG, "-o", arg(&wrong)]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains(".svg nor .png"));
    assert!(!wrong.exists());

    // A drawing against itself: nothing to mark, and still a picture --
    // at the size asked for.
    let png = dir.join("same.png");
    let (_, report) = answer(&[
        "redline",
        CORPUS_DWG,
        CORPUS_DWG,
        "-o",
        arg(&png),
        "--fit",
        "300",
    ]);
    assert!(report["marked"].as_array().unwrap().is_empty(), "{report}");
    let bytes = std::fs::read(&png).unwrap();
    assert!(bytes.starts_with(b"\x89PNG"));
    let side = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    assert_eq!(
        side(16).max(side(20)),
        300,
        "the longer side is --fit pixels"
    );
}

#[test]
fn redline_as_a_tool_answers_and_draws_as_the_command_line_does() {
    let dir = scratch("redline-mcp");
    let id = circle_id(&dir);
    let edited = dir.join("edited.json");
    answer(&[
        "set",
        CORPUS_DWG,
        "--id",
        &id,
        "--path",
        "radius",
        "--value",
        "1",
        "-o",
        arg(&edited),
    ]);
    let (by_cli, by_tool) = (dir.join("cli.svg"), dir.join("tool.svg"));
    let (cli, _) = answer(&[
        "redline",
        CORPUS_DWG,
        arg(&edited),
        "-o",
        arg(&by_cli),
        "--omit",
        "within",
    ]);
    let mut mcp = Mcp::start();
    let result = mcp.call(
        "redline",
        json!({"before": CORPUS_DWG, "after": arg(&edited), "output": arg(&by_tool),
               "omit": ["within"]}),
    );
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(result["content"][0]["text"].as_str().unwrap(), cli);
    assert_eq!(
        std::fs::read(&by_cli).unwrap(),
        std::fs::read(&by_tool).unwrap()
    );
}

#[test]
fn redline_can_frame_the_changes() {
    let dir = scratch("redline-frame");
    let id = circle_id(&dir);
    let edited = dir.join("edited.json");
    answer(&[
        "set",
        CORPUS_DWG,
        "--id",
        &id,
        "--path",
        "radius",
        "--value",
        "1",
        "-o",
        arg(&edited),
    ]);
    let (whole, framed) = (dir.join("whole.svg"), dir.join("framed.svg"));
    let (_, w) = answer(&["redline", CORPUS_DWG, arg(&edited), "-o", arg(&whole)]);
    let (_, f) = answer(&[
        "redline",
        CORPUS_DWG,
        arg(&edited),
        "-o",
        arg(&framed),
        "--frame",
        "changes",
    ]);
    let width = |v: &Value| {
        v["view_box"]["max_x"].as_f64().unwrap() - v["view_box"]["min_x"].as_f64().unwrap()
    };
    assert!(width(&f) <= width(&w), "{f} {w}");
    // The same change is marked either way.
    assert_eq!(f["marked"], w["marked"]);
}

/// G1's holes are pure red, close to the red the changes are drawn in: the
/// answer lists the clash and the command says so on stderr.
#[test]
fn a_redline_on_a_drawing_already_in_red_says_the_changes_may_not_stand_out() {
    let g1 = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../uncad/tests/golden/g1.expected.json"
    );
    let dir = scratch("redline-red");
    let edited = dir.join("edited.json");
    answer(&[
        "set",
        g1,
        "--id",
        "289",
        "--path",
        "radius",
        "--value",
        "4",
        "-o",
        arg(&edited),
    ]);
    let picture = dir.join("redline.svg");
    let out = run(&["redline", g1, arg(&edited), "-o", arg(&picture)]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: Value = serde_json::from_slice(&out.stdout).expect("JSON");
    let conflicts = report["proposal_color_conflicts"].as_array().unwrap();
    assert_eq!(conflicts.len(), 1, "{report}");
    assert_eq!(conflicts[0]["color"], "#ff0000");
    assert_eq!(conflicts[0]["uses"], 4);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("close to #e4002b"), "{stderr}");
    assert!(stderr.contains("#ff0000 (4 uses)"), "{stderr}");
    assert!(stderr.contains("--proposal-color <#rrggbb>"), "{stderr}");
}

/// The same G1 edit drawn in blue: the changes are in the color asked for,
/// the red holes no longer clash, and the tool draws the same picture.
#[test]
fn a_redline_draws_the_changes_in_the_color_asked_for() {
    let g1 = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../uncad/tests/golden/g1.expected.json"
    );
    let dir = scratch("redline-color");
    let edited = dir.join("edited.json");
    answer(&[
        "set",
        g1,
        "--id",
        "289",
        "--path",
        "radius",
        "--value",
        "4",
        "-o",
        arg(&edited),
    ]);
    let picture = dir.join("blue.svg");
    let out = run(&[
        "redline",
        g1,
        arg(&edited),
        "-o",
        arg(&picture),
        "--proposal-color",
        "#0057B8",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: Value = serde_json::from_slice(&out.stdout).expect("JSON");
    assert_eq!(report["proposal_color_conflicts"], json!([]), "{report}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("close to"), "{stderr}");
    let svg = std::fs::read_to_string(&picture).unwrap();
    // The change layer is colored by its style rule, which outranks the
    // entities' own stroke attributes.
    let style = &svg[svg.find("<style>").unwrap()..svg.find("</style>").unwrap()];
    assert!(style.contains("#changes *{stroke:#0057b8"), "{style}");
    assert!(!svg.contains("#e4002b"), "the default is not used");

    let by_tool = dir.join("tool.svg");
    let mut mcp = Mcp::start();
    let result = mcp.call(
        "redline",
        json!({"before": g1, "after": arg(&edited), "output": arg(&by_tool),
               "proposal_color": "#0057B8"}),
    );
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(
        std::fs::read(&picture).unwrap(),
        std::fs::read(&by_tool).unwrap()
    );
}

/// Anything but `#rrggbb` is refused before anything is read or written, and
/// the tool's schema says the same.
#[test]
fn a_redline_color_is_six_hex_digits_after_a_hash() {
    let dir = scratch("redline-color-refuse");
    for bad in ["red", "e4002b", "#e4002", "#e4002bb", "#g4002b"] {
        let picture = dir.join("never.svg");
        let out = run(&[
            "redline",
            CORPUS_DWG,
            CORPUS_DWG,
            "-o",
            arg(&picture),
            "--proposal-color",
            bad,
        ]);
        assert!(!out.status.success(), "{bad}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("a color as #rrggbb"), "{bad}: {stderr}");
        assert!(!picture.exists(), "{bad}");
    }

    let mut mcp = Mcp::start();
    let list = mcp.request("tools/list", json!({}));
    let redline = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "redline")
        .unwrap()
        .clone();
    assert_eq!(
        redline["inputSchema"]["properties"]["proposal_color"]["pattern"],
        "^#[0-9A-Fa-f]{6}$"
    );
}

/// G1 with one hole made smaller, as model JSON in `dir`: a redline's
/// second state.
fn g1_edited(dir: &std::path::Path) -> (&'static str, std::path::PathBuf) {
    let g1 = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../uncad/tests/golden/g1.expected.json"
    );
    let edited = dir.join("edited.json");
    answer(&[
        "set",
        g1,
        "--id",
        "289",
        "--path",
        "radius",
        "--value",
        "4",
        "-o",
        arg(&edited),
    ]);
    (g1, edited)
}

/// A redline's stroke is given in pixels of the picture: the answer's
/// stroke width, in drawing units, times the picture's pixels per unit (the
/// longer side of its view box at `--fit` pixels) is the width asked for --
/// framing the whole drawing or only the changes.
#[test]
fn a_redline_stroke_is_in_pixels_of_the_picture() {
    let dir = scratch("redline-stroke");
    let (g1, edited) = g1_edited(&dir);
    let longer = |v: &Value| {
        let b = &v["view_box"];
        let w = b["max_x"].as_f64().unwrap() - b["min_x"].as_f64().unwrap();
        let h = b["max_y"].as_f64().unwrap() - b["min_y"].as_f64().unwrap();
        w.max(h)
    };
    for frame in ["drawing", "changes"] {
        let thin = dir.join(format!("{frame}-thin.png"));
        let thick = dir.join(format!("{frame}-thick.png"));
        let common = ["--fit", "600", "--frame", frame];
        let (_, plain) = answer(
            &[
                &["redline", g1, arg(&edited), "-o", arg(&thin)][..],
                &common,
            ]
            .concat(),
        );
        let (_, stroked) = answer(
            &[
                &[
                    "redline",
                    g1,
                    arg(&edited),
                    "-o",
                    arg(&thick),
                    "--stroke",
                    "3",
                ][..],
                &common,
            ]
            .concat(),
        );
        let px = stroked["stroke_width"].as_f64().unwrap() * 600.0 / longer(&stroked);
        assert!((px - 3.0).abs() < 1e-9, "{frame}: {px} px");
        assert!(
            plain["stroke_width"].as_f64().unwrap() < stroked["stroke_width"].as_f64().unwrap(),
            "{frame}: {plain} {stroked}"
        );
        assert_ne!(
            std::fs::read(&thin).unwrap(),
            std::fs::read(&thick).unwrap(),
            "{frame}"
        );
    }
}

/// An SVG has no pixels: a stroke in pixels is refused for one, before
/// anything is read or written.
#[test]
fn a_redline_stroke_needs_a_png() {
    let dir = scratch("redline-stroke-svg");
    let picture = dir.join("never.svg");
    let out = run(&[
        "redline",
        CORPUS_DWG,
        CORPUS_DWG,
        "-o",
        arg(&picture),
        "--stroke",
        "3",
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("an SVG has none"), "{stderr}");
    assert!(!picture.exists());
    for bad in ["0", "-1", "nan"] {
        let picture = dir.join("never.png");
        let out = run(&[
            "redline",
            CORPUS_DWG,
            CORPUS_DWG,
            "-o",
            arg(&picture),
            "--stroke",
            bad,
        ]);
        assert!(!out.status.success(), "{bad}");
        assert!(!picture.exists(), "{bad}");
    }
}

/// On a dark page the redline starts with a black rectangle covering its
/// view, the original's white is kept white, and the tool draws the same
/// picture as the command line.
#[test]
fn a_redline_on_a_dark_page_starts_with_the_page() {
    let dir = scratch("redline-dark");
    let (g1, edited) = g1_edited(&dir);
    let (by_cli, by_tool) = (dir.join("cli.svg"), dir.join("tool.svg"));
    let (cli, _) = answer(&[
        "redline",
        g1,
        arg(&edited),
        "-o",
        arg(&by_cli),
        "--paper",
        "dark",
    ]);
    let svg = std::fs::read_to_string(&by_cli).unwrap();
    let attribute = |element: &str, name: &str| -> f64 {
        let key = format!(" {name}=\"");
        let start = element.find(&key).unwrap() + key.len();
        let len = element[start..].find('"').unwrap();
        element[start..start + len].parse().unwrap_or(f64::NAN)
    };
    let root = svg.lines().next().unwrap();
    let start = root.find("viewBox=\"").unwrap() + "viewBox=\"".len();
    let len = root[start..].find('"').unwrap();
    let view_box: Vec<f64> = root[start..start + len]
        .split(' ')
        .map(|n| n.parse().unwrap())
        .collect();
    let first = svg
        .lines()
        .skip(1)
        .map(str::trim)
        .find(|l| !l.starts_with("<style") && !l.starts_with("<defs"))
        .unwrap();
    assert!(
        first.starts_with("<rect") && first.contains("fill=\"#000000\""),
        "{first}"
    );
    let rect: Vec<f64> = ["x", "y", "width", "height"]
        .iter()
        .map(|n| attribute(first, n))
        .collect();
    assert_eq!(rect, view_box);
    // G1's outline is ACI 7: white on the dark page.
    let original =
        &svg[svg.find("<g id=\"original\">").unwrap()..svg.find("<g id=\"changes\">").unwrap()];
    assert!(original.contains("stroke=\"#ffffff\""), "{original}");
    assert!(!original.contains("\"#000000\""), "{original}");

    let mut mcp = Mcp::start();
    let result = mcp.call(
        "redline",
        json!({"before": g1, "after": arg(&edited), "output": arg(&by_tool), "paper": "dark"}),
    );
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(result["content"][0]["text"].as_str().unwrap(), cli);
    assert_eq!(svg.as_bytes(), std::fs::read(&by_tool).unwrap());
}

/// A summary asked for a selection picks the entities by what they are,
/// and the tool and the command line agree on it byte for byte; a record
/// asked for is the model's own form, whose fields `set` takes.
#[test]
fn a_summary_selects_entities_and_reads_them() {
    let (_, plain) = answer(&["summarize", CORPUS_DWG]);
    assert!(
        plain.get("selection").is_none(),
        "no selection unless asked"
    );
    let circles = plain["by_type"]["CIRCLE"]
        .as_u64()
        .expect("the fixture has circles");

    let (cli, value) = answer(&["summarize", CORPUS_DWG, "--type", "circle", "--detail"]);
    let selection = &value["selection"];
    assert_eq!(selection["total"], circles, "{selection}");
    let first = &selection["entities"][0];
    assert_eq!(first["entity_type"], "CIRCLE");
    assert!(first["bounds"]["min"]["x"].is_number(), "{first}");
    let record = &first["record"];
    assert_eq!(record["type"], "CIRCLE");
    assert_eq!(record["common"]["id"], first["id"]);
    let (_, _, r) = circle();
    assert_eq!(
        record["radius"], r,
        "the record carries the field `set` edits"
    );

    let mut mcp = Mcp::start();
    let result = mcp.call(
        "summarize",
        json!({"input": CORPUS_DWG, "type": "circle", "detail": true}),
    );
    assert_eq!(result["content"][0]["text"].as_str().unwrap(), cli);

    // One entity by its ID; a limit keeps the first, the total counts all.
    let id = first["id"].to_string();
    let (_, one) = answer(&["summarize", CORPUS_DWG, "--id", &id]);
    assert_eq!(one["selection"]["total"], 1);
    assert!(one["selection"]["entities"][0].get("record").is_none());
    let (_, limited) = answer(&["summarize", CORPUS_DXF, "--limit", "1"]);
    assert_eq!(limited["selection"]["total"], limited["entity_count"]);
    assert_eq!(
        limited["selection"]["entities"].as_array().unwrap().len(),
        1
    );

    // A box must be four numbers, the smaller corner first.
    for bad in ["1,2,3", "5,0,1,1", "a,b,c,d"] {
        let out = run(&["summarize", CORPUS_DWG, "--within", bad]);
        assert!(!out.status.success(), "--within {bad} is refused");
    }
}

#[test]
fn a_limited_hit_test_keeps_the_nearest_and_counts_the_rest() {
    let (cx, cy, r) = circle();
    let (x, y) = (cx.to_string(), cy.to_string());
    let tolerance = (r * 1000.0).to_string();
    let (_, all) = answer(&[
        "hit-test",
        CORPUS_DWG,
        "--x",
        &x,
        "--y",
        &y,
        "--tolerance",
        &tolerance,
    ]);
    let hits = all["hits"].as_array().unwrap().len();
    assert!(hits > 0, "{all}");
    assert!(all.get("hits_total").is_none());
    let (_, none) = answer(&[
        "hit-test",
        CORPUS_DWG,
        "--x",
        &x,
        "--y",
        &y,
        "--tolerance",
        &tolerance,
        "--limit",
        "0",
    ]);
    assert_eq!(none["hits"], json!([]));
    assert_eq!(none["hits_total"], hits);
}

/// The JSON text with the whitespace outside strings taken out.
fn compact(json: &str) -> String {
    let (mut out, mut in_string, mut escaped) = (String::new(), false, false);
    for c in json.chars() {
        if in_string {
            let was_escaped = escaped;
            escaped = !was_escaped && c == '\\';
            in_string = was_escaped || c != '"';
            out.push(c);
        } else if c == '"' {
            in_string = true;
            out.push(c);
        } else if !c.is_whitespace() {
            out.push(c);
        }
    }
    out
}

/// `--pretty`, which the help lists for every command that answers with
/// JSON, lays the same answer out over lines: nothing but whitespace
/// outside strings is added, so keys keep their order.
#[test]
fn every_json_answer_takes_pretty() {
    let (cx, cy, r) = circle();
    let (x, y, tolerance) = (cx.to_string(), cy.to_string(), r.to_string());
    let calls: Vec<Vec<&str>> = vec![
        vec!["summarize", CORPUS_DWG],
        vec!["summarize", CORPUS_DWG, "--type", "circle"],
        vec![
            "hit-test",
            CORPUS_DWG,
            "--x",
            &x,
            "--y",
            &y,
            "--tolerance",
            &tolerance,
        ],
        vec!["diff", CORPUS_DWG, CORPUS_DXF, "--matching", "geometry"],
    ];
    for argv in calls {
        let (line, _) = answer(&argv);
        let mut with = argv.clone();
        with.push("--pretty");
        let out = run(&with);
        assert!(out.status.success(), "{with:?}");
        let text = String::from_utf8(out.stdout).unwrap();
        assert!(text.lines().count() > 1, "{with:?} is laid out over lines");
        assert_eq!(compact(&text), line, "{with:?} is the same answer");
    }
    // `set` answers with JSON too.
    let dir = std::env::temp_dir().join(format!("uncad-cli-pretty-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out_file = dir.join("set.json");
    let _ = std::fs::remove_file(&out_file);
    let (_, summary) = answer(&["summarize", CORPUS_DWG, "--type", "circle", "--limit", "1"]);
    let id = summary["selection"]["entities"][0]["id"].to_string();
    let out = run(&[
        "set",
        CORPUS_DWG,
        "--id",
        &id,
        "--path",
        "radius",
        "--value",
        "1",
        "-o",
        out_file.to_str().unwrap(),
        "--pretty",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8(out.stdout).unwrap().lines().count() > 1);
    let _ = std::fs::remove_dir_all(&dir);
}
