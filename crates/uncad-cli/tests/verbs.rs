//! The verbs (`summarize`, `hit-test`, `diff`) and `uncad mcp`, which serves
//! the same verbs as MCP tools.
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
/// point tests aim at it without copying its numbers here.
fn circle() -> (f64, f64, f64) {
    let model = std::env::temp_dir().join(format!("uncad-cli-verbs-{}.json", std::process::id()));
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
fn mcp_lists_the_verbs_as_read_only_tools() {
    let mut mcp = Mcp::start();
    let list = mcp.request("tools/list", json!({}));
    let tools = list["result"]["tools"].as_array().expect("tools");
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["summarize", "hit_test", "diff"]);
    for tool in tools {
        assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
        assert_eq!(tool["annotations"]["readOnlyHint"], true, "{tool}");
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
    let response = mcp.request("tools/call", json!({"name": "set", "arguments": {}}));
    assert!(response["error"].is_object(), "{response}");
}
