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

/// The one circle of the DWG fixture, read from the model export: the
/// point tests aim at it without copying its numbers here. Each call writes
/// its own file -- the tests of this binary run in parallel in one process.
fn circle() -> (f64, f64, f64) {
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
    let db: Value = serde_json::from_str(&text).expect("the export is JSON");
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
        vec!["summarize", CORPUS_DXF, "--pretty"],
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
