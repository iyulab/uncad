//! The verbs (`summarize`, `hit-test`, `diff`, `set`, `redline`, `balloon`) and
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

/// A drawing whose dimensions stand on lines, two blocks standing for
/// other drawings beside them.
const XREF_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/2013/gh44-error.dwg"
);

#[test]
fn relate_names_what_each_dimension_stands_on_and_what_it_could_not_look_at() {
    let (_, r) = answer(&["relate", XREF_DWG]);
    let tolerance = r["tolerance"].as_f64().expect("the tolerance is stated");
    assert!(tolerance > 0.0, "{tolerance}");
    let external = r["external_references"]
        .as_array()
        .expect("external references");
    assert_eq!(external.len(), 2, "{external:?}");
    for x in external {
        assert_eq!(x["entities"], 0, "{x}");
        assert_eq!(x["inserts"].as_array().unwrap().len(), 1, "{x}");
    }
    let dimensions = r["dimensions"].as_array().expect("dimensions");
    let at_point = dimensions
        .iter()
        .flat_map(|d| d["points"].as_array().unwrap())
        .flat_map(|p| p["relations"].as_array().unwrap())
        .filter(|r| r["relation"] == "AT_POINT")
        .count();
    assert!(at_point > 0, "some point is a point of the geometry");

    // One dimension, by its ID; the rest of the answer as it was.
    let id = dimensions[0]["id"].as_u64().unwrap().to_string();
    let (_, one) = answer(&["relate", XREF_DWG, "--id", &id]);
    assert_eq!(one["dimensions"].as_array().unwrap(), &dimensions[..1]);
    assert_eq!(one["external_references"], r["external_references"]);

    // A stated tolerance is the one used.
    let (_, wide) = answer(&["relate", XREF_DWG, "--tolerance", "0.5"]);
    assert_eq!(wide["tolerance"], 0.5);
}

#[test]
fn relate_refuses_a_tolerance_that_is_not_positive() {
    for bad in ["0", "-1"] {
        let out = run(&["relate", CORPUS_DWG, "--tolerance", bad]);
        assert!(!out.status.success(), "{bad}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("tolerance"), "{bad}: {stderr}");
    }
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
        "--min-similarity",
        "0.75",
        "--min-margin",
        "0.25",
        "--max-pairs",
        "5",
    ]);
    assert_eq!(given["matching"], "GEOMETRY");
    assert_eq!(given["tolerance"]["length"], 0.5);
    assert_eq!(
        given["pairing"],
        json!({"min_similarity": 0.75, "min_margin": 0.25, "max_pairs": 5})
    );
    // Under reference matching nothing is paired by similarity.
    assert!(same.get("pairing").is_none(), "{same}");
}

/// A drawing of the test corpus, by its path under the corpus's root.
fn corpus(path: &str) -> String {
    format!(
        "{}/../../lib/libredwg/test/test-data/{path}",
        env!("CARGO_MANIFEST_DIR")
    )
}

#[test]
fn auto_matching_pairs_by_reference_only_drawings_the_files_show_to_be_one() {
    // One drawing saved in two formats: one fingerprint, every ID kept.
    let (_, same) = answer(&[
        "diff",
        &corpus("example_2000.dwg"),
        &corpus("example_2004.dwg"),
    ]);
    assert_eq!(same["matching"], "REFERENCE");
    assert_eq!(same["lineage"]["verdict"], "SAME");
    assert_eq!(same["lineage"]["fingerprint_equal"], true);
    assert_eq!(same["lineage"]["shared"], same["lineage"]["smaller"]);

    // Two unrelated drawings made from one template: the fingerprint is the
    // same, but IDs both hold name entities of different types.
    let unrelated = [
        "diff".to_string(),
        corpus("example_2000.dwg"),
        corpus("sample_2000.dwg"),
    ];
    let args: Vec<&str> = unrelated.iter().map(String::as_str).collect();
    let (_, other) = answer(&args);
    assert_eq!(other["matching"], "GEOMETRY");
    assert_eq!(other["lineage"]["verdict"], "DIFFERENT");
    assert_eq!(other["lineage"]["fingerprint_equal"], true);
    assert!(other["lineage"]["cross_type"].as_u64() > Some(0), "{other}");

    // A mode given is kept, and the lineage still says what the files show.
    let (_, forced) = answer(&[args.as_slice(), &["--matching", "reference"]].concat());
    assert_eq!(forced["matching"], "REFERENCE");
    assert_eq!(forced["lineage"]["verdict"], "DIFFERENT");
}

#[test]
fn one_drawing_saved_in_a_newer_format_differs_only_in_what_the_files_state_differently() {
    // The corpus holds one drawing saved in six formats. Leaving out what the
    // older format had no place for (`unstated`) and what moved within
    // tolerance, the change set is empty -- but for one name the files
    // themselves state differently: from R2007 on, the system lights layer
    // is written `*ADSK_SYSTEM_LIGHTS` (each version's DXF twin states the
    // same).
    let base = corpus("example_2000.dwg");
    for version in ["2004", "2007", "2010", "2013", "2018"] {
        let other = corpus(&format!("example_{version}.dwg"));
        let (_, set) = answer(&["diff", &base, &other, "--omit", "within,unstated"]);
        assert_eq!(set["lineage"]["verdict"], "SAME", "{version}");
        assert_eq!(set["matching"], "REFERENCE", "{version}");
        assert!(
            set["omitted"]["unstated_fields"].as_u64() > Some(0),
            "{version}: the newer format states what the older could not"
        );
        let changes = set["changes"].as_array().unwrap();
        if version == "2004" {
            assert!(changes.is_empty(), "{version}: {changes:?}");
            continue;
        }
        assert_eq!(changes.len(), 1, "{version}: {changes:?}");
        let modified = &changes[0]["data"];
        assert_eq!(changes[0]["type"], "MODIFIED");
        assert_eq!(modified["entity_type"], "LIGHT");
        let fields = modified["fields"].as_array().unwrap();
        assert_eq!(fields.len(), 1, "{version}: {fields:?}");
        assert_eq!(fields[0]["path"], "common.layer.data");
        assert_eq!(
            (&fields[0]["before"], &fields[0]["after"]),
            (&json!("ADSK_SYSTEM_LIGHTS"), &json!("*ADSK_SYSTEM_LIGHTS"))
        );
    }
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
    /// A server that has not been spoken to yet.
    fn spawn() -> Self {
        let mut child = Command::new(EXE)
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("the test binary should be runnable");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Mcp {
            child,
            stdin,
            stdout,
            next_id: 1,
        }
    }

    /// A server in a session on protocol version 2025-06-18.
    fn start() -> Self {
        let mut mcp = Mcp::spawn();
        let init = mcp.initialize("2025-06-18");
        assert_eq!(init["result"]["serverInfo"]["name"], "uncad", "{init}");
        mcp
    }

    /// Opens a session on `version` (a version before 2026-07-28, which
    /// has sessions) and returns the `initialize` response.
    fn initialize(&mut self, version: &str) -> Value {
        let init = self.request(
            "initialize",
            json!({
                "protocolVersion": version,
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "0"}
            }),
        );
        self.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        init
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

/// The first protocol version without sessions: a client asks
/// `server/discover`, then names the version in every request's `_meta`.
const STATELESS: &str = "2026-07-28";

/// Every protocol version the server advertises, spoken the way that
/// version is spoken: each negotiates as itself, lists the same five
/// tools, and answers a call with the same bytes -- what a tool says does
/// not depend on the protocol it is asked over. From 2026-07-28 a list
/// result must also say how long it stays fresh (`ttlMs`) and who may
/// cache it (`cacheScope`): a client validating the schema drops a list
/// without them, and with it every tool.
#[test]
fn every_advertised_protocol_version_lists_and_answers_alike() {
    let stateless_meta = |version: &str| {
        json!({
            "io.modelcontextprotocol/protocolVersion": version,
            "io.modelcontextprotocol/clientInfo": {"name": "test", "version": "0"},
            "io.modelcontextprotocol/clientCapabilities": {}
        })
    };
    let discover = Mcp::spawn().request(
        "server/discover",
        json!({"_meta": stateless_meta(STATELESS)}),
    );
    let versions: Vec<String> = discover["result"]["supportedVersions"]
        .as_array()
        .expect("supportedVersions")
        .iter()
        .map(|v| v.as_str().expect("a version string").to_string())
        .collect();
    assert!(versions.iter().any(|v| v == STATELESS), "{discover}");
    assert!(
        versions.iter().any(|v| v.as_str() < STATELESS),
        "{discover}"
    );

    let mut answers = Vec::new();
    for version in &versions {
        let mut mcp = Mcp::spawn();
        let params = |extra: Value| -> Value {
            let mut p = extra;
            if version.as_str() >= STATELESS {
                p["_meta"] = stateless_meta(version);
            }
            p
        };
        if version.as_str() < STATELESS {
            let init = mcp.initialize(version);
            assert_eq!(init["result"]["protocolVersion"], *version, "{init}");
        }
        let list = mcp.request("tools/list", params(json!({})));
        let result = &list["result"];
        let names: Vec<&str> = result["tools"]
            .as_array()
            .unwrap_or_else(|| panic!("{version}: {list}"))
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "summarize",
                "hit_test",
                "relate",
                "diff",
                "set",
                "redline",
                "balloon"
            ],
            "{version}"
        );
        if version.as_str() >= STATELESS {
            assert!(
                result["ttlMs"].is_u64(),
                "{version}: ttlMs is a number, not {}",
                result["ttlMs"]
            );
            assert!(
                result["cacheScope"] == "public" || result["cacheScope"] == "private",
                "{version}: cacheScope is public or private, not {}",
                result["cacheScope"]
            );
        }
        let call = mcp.request(
            "tools/call",
            params(json!({"name": "summarize", "arguments": {"input": CORPUS_DXF}})),
        );
        let result = &call["result"];
        assert_ne!(result["isError"], true, "{version}: {call}");
        let text = result["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("{version}: {call}"))
            .to_string();
        serde_json::from_str::<Value>(&text).expect("the answer is JSON");
        answers.push((version.clone(), text));
    }
    for (version, text) in &answers[1..] {
        assert_eq!(
            text, &answers[0].1,
            "{version} answers like {}",
            answers[0].0
        );
    }
}

#[test]
fn mcp_lists_the_verbs_and_only_the_file_writers_write() {
    let mut mcp = Mcp::start();
    let list = mcp.request("tools/list", json!({}));
    let tools = list["result"]["tools"].as_array().expect("tools");
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "summarize",
            "hit_test",
            "relate",
            "diff",
            "set",
            "redline",
            "balloon"
        ]
    );
    for tool in tools {
        let writes = ["set", "redline", "balloon"].contains(&tool["name"].as_str().unwrap());
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
    let named: Vec<&str> = [
        "summarize",
        "hit-test",
        "relate",
        "diff",
        "set",
        "redline",
        "balloon",
        "export",
    ]
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
            vec!["relate".into(), XREF_DWG.into()],
            "relate",
            json!({"input": XREF_DWG}),
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

/// The server keeps the drawings it has read and reuses one only for the
/// same bytes: asked again about an unchanged file it answers as before,
/// and asked about a file whose content has changed it answers from the
/// content as it is now -- what the command line prints for it -- never
/// from the drawing it held.
#[test]
fn a_server_answers_from_a_file_as_it_is_now() {
    fn ask(mcp: &mut Mcp, input: &str) -> String {
        let result = mcp.call("summarize", json!({ "input": input }));
        assert_eq!(result["isError"], false, "{result}");
        result["content"][0]["text"]
            .as_str()
            .expect("a text block")
            .to_string()
    }
    let other = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../lib/libredwg/test/test-data/2000/Line.dwg"
    );
    let dir = scratch("held-drawing");
    let drawing = dir.join("drawing.dwg");
    std::fs::copy(CORPUS_DWG, &drawing).expect("the fixture is copied");

    let mut mcp = Mcp::start();
    let first = ask(&mut mcp, arg(&drawing));
    assert_eq!(first, answer(&["summarize", arg(&drawing)]).0);
    assert_eq!(ask(&mut mcp, arg(&drawing)), first);

    // Another drawing under the same name.
    std::fs::copy(other, &drawing).expect("the other drawing is copied");
    let now = ask(&mut mcp, arg(&drawing));
    assert_ne!(
        now, first,
        "the answer should follow the file's new content"
    );
    assert_eq!(now, answer(&["summarize", arg(&drawing)]).0);
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

/// The record of one entity, as `summarize --id --detail` gives it.
fn record(input: &str, id: u64) -> Value {
    let (_, answer) = answer(&["summarize", input, "--id", &id.to_string(), "--detail"]);
    answer["selection"]["entities"][0]["record"].clone()
}

fn xy(p: &Value) -> (f64, f64) {
    (p["x"].as_f64().unwrap(), p["y"].as_f64().unwrap())
}

/// The entities a dimension's points are related to, by role.
fn related(answer: &Value, dimension: u64) -> Vec<(String, Vec<u64>)> {
    let d = answer["dimensions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == dimension)
        .unwrap_or_else(|| panic!("dimension {dimension} in {answer}"));
    d["points"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let ids = p["relations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["entity"].as_u64().unwrap())
                .collect();
            (p["role"].as_str().unwrap().to_string(), ids)
        })
        .collect()
}

/// A circle resized on its own leaves the dimensions that measure it
/// standing on nothing; resized together with them -- found by `relate`,
/// edited by a chain of `set` calls -- every point is on the circle again,
/// one diff lists every field of the chain, and the redline marks all of it.
#[test]
fn a_circle_and_the_dimensions_that_measure_it_are_edited_as_one_chain() {
    let dir = scratch("circle-and-dimensions");
    let input = corpus("2000/TS1.dxf");
    let original = std::fs::read(&input).unwrap();

    // The circle a diameter dimension stands on, and every dimension
    // standing on it.
    let (_, before) = answer(&["relate", &input]);
    let dimensions = before["dimensions"].as_array().unwrap();
    let circle = dimensions
        .iter()
        .find(|d| d["kind"] == "DIAMETER")
        .and_then(|d| d["points"][0]["relations"][0]["entity"].as_u64())
        .expect("a diameter dimension on a circle");
    let measuring: Vec<u64> = dimensions
        .iter()
        .filter(|d| {
            d["points"].as_array().unwrap().iter().any(|p| {
                p["relations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|r| r["entity"] == circle)
            })
        })
        .map(|d| d["id"].as_u64().unwrap())
        .collect();
    assert_eq!(measuring.len(), 2, "a diameter and a radius: {before}");

    let c = record(&input, circle);
    let (cx, cy) = xy(&c["center"]);
    let r = c["radius"].as_f64().unwrap();
    let grown = r * 1.25;

    // The circle alone: its dimensions no longer stand on it.
    let alone = dir.join("alone.json");
    let circle_id = circle.to_string();
    answer(&[
        "set",
        &input,
        "--id",
        &circle_id,
        "--path",
        "radius",
        "--value",
        &grown.to_string(),
        "-o",
        arg(&alone),
    ]);
    let (_, detached) = answer(&["relate", arg(&alone)]);
    // A point on the curve is left standing on nothing; only a point at the
    // centre -- a radius dimension's -- is still related to the circle.
    let on_circle = |answer: &Value, d: u64| -> Vec<(String, String)> {
        let dim = answer["dimensions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["id"] == d)
            .unwrap();
        dim["points"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|p| {
                p["relations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|r| r["entity"] == circle)
                    .map(|r| {
                        let how = format!("{}{}", r["relation"], r["anchor"]);
                        (p["role"].as_str().unwrap().to_string(), how)
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    };
    for &d in &measuring {
        let kept: Vec<_> = on_circle(&before, d)
            .into_iter()
            .filter(|(_, how)| how == "\"AT_POINT\"\"center\"")
            .collect();
        assert_eq!(on_circle(&detached, d), kept, "{d}: {detached}");
    }

    // The chain: each measuring dimension's points on the curve move out
    // with it, and its measurement follows.
    let mut step = alone;
    let mut edited_fields = vec![(circle, vec!["radius".to_string()])];
    for (n, &d) in measuring.iter().enumerate() {
        let dim = record(arg(&step), d);
        let mut fields = Vec::new();
        for path in ["definition_point", "points.radial"] {
            let p = path.split('.').fold(&dim, |v, k| &v[k]);
            let (x, y) = xy(p);
            let off = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
            if (off - r).abs() > 1e-9 {
                continue; // the centre, not a point on the curve
            }
            let moved = json!({
                "x": cx + (x - cx) * 1.25,
                "y": cy + (y - cy) * 1.25,
                "z": p["z"],
            });
            fields.push(path.to_string());
            step = set_step(&dir, &step, d, path, &moved, &format!("{n}-{path}"));
        }
        let measurement = match dim["kind"].as_str().unwrap() {
            "DIAMETER" => grown * 2.0,
            _ => grown,
        };
        step = set_step(
            &dir,
            &step,
            d,
            "measurement",
            &json!(measurement),
            &format!("{n}-measurement"),
        );
        fields.push("measurement".to_string());
        edited_fields.push((d, fields));
    }

    // Every point is on the circle again, and each measurement is the
    // distance its points state: across the circle, or out from its centre.
    let (_, after) = answer(&["relate", arg(&step)]);
    for &d in &measuring {
        assert_eq!(related(&after, d), related(&before, d), "{d}: {after}");
        let dim = record(arg(&step), d);
        let (ax, ay) = xy(&dim["definition_point"]);
        let (bx, by) = xy(&dim["points"]["radial"]);
        let stated = ((ax - bx).powi(2) + (ay - by).powi(2)).sqrt();
        let measurement = dim["measurement"].as_f64().unwrap();
        assert!((stated - measurement).abs() < 1e-9, "{d}: {dim}");
    }

    // One diff lists every field of the chain, and nothing else.
    let (_, diff) = answer(&["diff", &input, arg(&step)]);
    let mut changed: Vec<(u64, Vec<String>)> = diff["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|ch| {
            assert_eq!(ch["type"], "MODIFIED", "{diff}");
            let mut paths: Vec<String> = ch["data"]["fields"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f["path"].as_str().unwrap().to_string())
                .collect();
            paths.sort();
            (ch["data"]["id"].as_u64().unwrap(), paths)
        })
        .collect();
    changed.sort();
    let mut expected: Vec<(u64, Vec<String>)> = edited_fields
        .into_iter()
        .map(|(id, fields)| {
            let mut leaves: Vec<String> = fields
                .iter()
                .flat_map(|f| match f.as_str() {
                    "radius" | "measurement" => vec![f.clone()],
                    point => vec![format!("{point}.x"), format!("{point}.y")],
                })
                .collect();
            leaves.sort();
            (id, leaves)
        })
        .collect();
    expected.sort();
    assert_eq!(changed, expected, "{diff}");

    // The redline marks the circle and both dimensions. The circle is drawn
    // as it became; a dimension's drawing is the block the file made for its
    // old record, so it is clouded and not drawn.
    let picture = dir.join("redline.svg");
    let (_, report) = answer(&["redline", &input, arg(&step), "-o", arg(&picture)]);
    let mut marked: Vec<(u64, Value, Value)> = report["marked"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            let not_drawn = m.get("not_drawn").cloned().unwrap_or(Value::Null);
            (m["before"].as_u64().unwrap(), m["after"].clone(), not_drawn)
        })
        .collect();
    marked.sort_by_key(|m| m.0);
    let mut expected: Vec<(u64, Value, Value)> = measuring
        .iter()
        .map(|&d| (d, Value::Null, json!("BLOCK_OUT_OF_DATE")))
        .chain([(circle, json!(circle), Value::Null)])
        .collect();
    expected.sort_by_key(|m| m.0);
    assert_eq!(marked, expected, "{report}");
    assert!(
        report["not_marked"].as_array().unwrap().is_empty(),
        "{report}"
    );

    assert_eq!(std::fs::read(&input).unwrap(), original);
}

/// A circle inside a block that the drawing places eight times: `set`
/// edits the definition, and the redline marks the circle at each of the
/// eight places, once per top-level reference.
#[test]
fn an_edit_inside_a_block_is_marked_at_every_place_the_block_is_put() {
    let dir = scratch("inside-a-block");
    let input = corpus("example_2000.dwg");
    let model = dir.join("model.json");
    answer_or_written(&[&input, "-o", arg(&model)]);
    let model: Value = serde_json::from_str(&std::fs::read_to_string(&model).unwrap()).unwrap();
    let block = "CIRKLO_PUNKTOJ";
    let circle = model["tables"]["block_records"][block]["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["type"] == "CIRCLE")
        .expect("a circle in the block")["common"]["id"]
        .as_u64()
        .unwrap();
    let mut references: Vec<u64> = model["entities"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["type"] == "INSERT" && e["block_name"]["data"] == block)
        .map(|e| e["common"]["id"].as_u64().unwrap())
        .collect();
    references.sort();
    assert_eq!(references.len(), 8, "{references:?}");

    let edited = set_step(
        &dir,
        std::path::Path::new(&input),
        circle,
        "radius",
        &json!(0.9),
        "edited",
    );
    let picture = dir.join("redline.svg");
    let (_, report) = answer(&["redline", &input, arg(&edited), "-o", arg(&picture)]);
    let mut via: Vec<u64> = report["marked"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            assert_eq!(m["entry"], 0, "{report}");
            assert_eq!(m["after"], circle, "{report}");
            m["via"][0].as_u64().unwrap()
        })
        .collect();
    via.sort();
    assert_eq!(via, references, "{report}");
    assert!(
        report["not_marked"].as_array().unwrap().is_empty(),
        "{report}"
    );
}

/// Runs a command that writes a file and answers nothing on stdout.
fn answer_or_written(args: &[&str]) {
    let out = run(args);
    assert!(
        out.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A change to what a dimension states -- its tolerance, its text -- and
/// nothing else: the stated tolerance is the dimension's own overrides with
/// the tolerance variables added after the ones it had, the text a literal.
/// One diff lists the added overrides as new elements and the text's two
/// fields; the redline clouds both dimensions and draws neither from its
/// old block.
#[test]
fn what_a_dimension_states_is_edited_and_marked_without_its_old_drawing() {
    let dir = scratch("dimension-statement");
    let input = corpus("2000/TS1.dxf");
    let original = std::fs::read(&input).unwrap();
    let (_, relate) = answer(&["relate", &input]);
    let kinds: Vec<(u64, &str)> = relate["dimensions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| Some((d["id"].as_u64()?, d["kind"].as_str()?)))
        .collect();
    let diameter = kinds.iter().find(|k| k.1 == "DIAMETER").unwrap().0;
    let radius = kinds.iter().find(|k| k.1 == "RADIUS").unwrap().0;

    // DIMTOL on, DIMTP and DIMTM, after the overrides the dimension has.
    let had = record(&input, diameter)["style_overrides"].clone();
    let had = had.as_array().unwrap();
    let mut overrides = had.clone();
    overrides.extend([
        json!({"variable": 71, "value": {"type": "INTEGER", "data": 1}}),
        json!({"variable": 47, "value": {"type": "REAL", "data": 0.05}}),
        json!({"variable": 48, "value": {"type": "REAL", "data": 0.02}}),
    ]);
    let toleranced = set_step(
        &dir,
        std::path::Path::new(&input),
        diameter,
        "style_overrides",
        &Value::Array(overrides.clone()),
        "toleranced",
    );
    let text = json!({"type": "LITERAL", "data": "R1.5"});
    let stated = set_step(&dir, &toleranced, radius, "text_override", &text, "stated");

    let (_, diff) = answer(&["diff", &input, arg(&stated)]);
    let mut changed: Vec<(u64, Vec<(String, Value)>)> = diff["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|ch| {
            assert_eq!(ch["type"], "MODIFIED", "{diff}");
            let fields = ch["data"]["fields"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| (f["path"].as_str().unwrap().to_string(), f["after"].clone()))
                .collect();
            (ch["data"]["id"].as_u64().unwrap(), fields)
        })
        .collect();
    changed.sort_by_key(|c| c.0);
    let added: Vec<(String, Value)> = (had.len()..overrides.len())
        .map(|i| (format!("style_overrides[{i}]"), overrides[i].clone()))
        .collect();
    let mut expected = vec![
        (diameter, added),
        (
            radius,
            vec![
                ("text_override.data".to_string(), json!("R1.5")),
                ("text_override.type".to_string(), json!("LITERAL")),
            ],
        ),
    ];
    expected.sort_by_key(|c| c.0);
    assert_eq!(changed, expected, "{diff}");

    let picture = dir.join("redline.svg");
    let (_, report) = answer(&["redline", &input, arg(&stated), "-o", arg(&picture)]);
    let marks = report["marked"].as_array().unwrap();
    assert_eq!(marks.len(), 2, "{report}");
    for m in marks {
        assert_eq!(m["after"], Value::Null, "{report}");
        assert_eq!(m["not_drawn"], "BLOCK_OUT_OF_DATE", "{report}");
    }
    let svg = std::fs::read_to_string(&picture).unwrap();
    let changes = &svg[svg.find("<g id=\"changes\">").unwrap()..];
    assert_eq!(changes.matches("class=\"cloud\"").count(), 2);
    assert!(
        !changes.contains("<text"),
        "no old text drawn as the change"
    );

    assert_eq!(std::fs::read(&input).unwrap(), original);
}

/// One `set` of the chain, from `from` to a new file in `dir`.
fn set_step(
    dir: &std::path::Path,
    from: &std::path::Path,
    id: u64,
    path: &str,
    value: &Value,
    name: &str,
) -> std::path::PathBuf {
    let to = dir.join(format!("{name}.json"));
    answer(&[
        "set",
        arg(from),
        "--id",
        &id.to_string(),
        "--path",
        path,
        "--value",
        &value.to_string(),
        "-o",
        arg(&to),
    ]);
    to
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
    let out = run(&[
        "redline",
        g1,
        arg(&edited),
        "-o",
        arg(&picture),
        "--proposal-color",
        "#e4002b",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: Value = serde_json::from_slice(&out.stdout).expect("JSON");
    assert_eq!(report["proposal_color"], "#e4002b");
    let conflicts = report["proposal_color_conflicts"].as_array().unwrap();
    assert_eq!(conflicts.len(), 1, "{report}");
    assert_eq!(conflicts[0]["color"], "#ff0000");
    assert_eq!(conflicts[0]["uses"], 4);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("close to #e4002b"), "{stderr}");
    assert!(stderr.contains("#ff0000 (4 uses)"), "{stderr}");
    assert!(stderr.contains("--proposal-color <#rrggbb>"), "{stderr}");
}

/// Left to choose, redline moves past the red G1 already uses, to the next
/// candidate, and says nothing about a clash.
#[test]
fn a_redline_chooses_a_color_the_drawing_does_not_use() {
    let g1 = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../uncad/tests/golden/g1.expected.json"
    );
    let dir = scratch("redline-auto");
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
    let [r, g, b] = iron_render_cad::PROPOSAL_CANDIDATES[1];
    let purple = format!("#{r:02x}{g:02x}{b:02x}");
    assert_eq!(report["proposal_color"], purple.as_str(), "{report}");
    assert_eq!(report["proposal_color_conflicts"], json!([]), "{report}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("close to"), "{stderr}");
    let svg = std::fs::read_to_string(&picture).unwrap();
    assert!(
        svg.contains(&format!("#changes *{{stroke:{purple}")),
        "{svg}"
    );

    // The tool's description names every candidate, in order.
    let mut mcp = Mcp::start();
    let list = mcp.request("tools/list", json!({}));
    let redline = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "redline")
        .unwrap()
        .clone();
    let text = redline["inputSchema"]["properties"]["proposal_color"]["description"]
        .as_str()
        .unwrap()
        .to_string();
    let mut at = 0;
    for [r, g, b] in iron_render_cad::PROPOSAL_CANDIDATES {
        let hex = format!("#{r:02x}{g:02x}{b:02x}");
        let found = text[at..]
            .find(&hex)
            .unwrap_or_else(|| panic!("{hex}: {text}"));
        at += found + hex.len();
    }
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

/// A small change in a large drawing: in a 1600 px picture of the whole
/// drawing its mark is a few pixels, and the redline says so and names the
/// way out; framed on the changes it is not small, and nothing is said.
#[test]
fn a_redline_says_when_a_mark_is_too_small_to_find() {
    let dir = scratch("redline-small");
    let input = corpus("2018/Dynblocks.dwg");
    let edited = set_step(
        &dir,
        std::path::Path::new(&input),
        49584,
        "radius",
        &json!(2.5),
        "edited",
    );
    let small_warning = |frame: &str| -> (Value, Option<String>) {
        let picture = dir.join(format!("{frame}.png"));
        let out = run(&[
            "redline",
            &input,
            arg(&edited),
            "-o",
            arg(&picture),
            "--fit",
            "1600",
            "--frame",
            frame,
        ]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let report: Value = serde_json::from_slice(&out.stdout).unwrap();
        let warning = String::from_utf8(out.stderr)
            .unwrap()
            .lines()
            .find(|l| l.contains("marks are under"))
            .map(str::to_string);
        (report, warning)
    };

    let (whole, warning) = small_warning("drawing");
    let share = whole["marked"][0]["share"].as_f64().unwrap();
    assert!(share * 1600.0 < 8.0, "{whole}");
    let warning = warning.expect("a warning for the small mark");
    assert!(
        warning.contains("1 of 1 marks are under 8 px in this 1600 px picture")
            && warning.contains("--frame changes"),
        "{warning}"
    );

    let (framed, warning) = small_warning("changes");
    assert!(
        framed["marked"][0]["share"].as_f64().unwrap() * 1600.0 >= 8.0,
        "{framed}"
    );
    assert_eq!(warning, None);
}

/// An SVG has no pixels of its own: its stroke is pixels on the screen, a
/// style rule; a width that is not a positive number is refused, before
/// anything is read or written.
#[test]
fn a_redline_stroke_on_an_svg_is_pixels_on_the_screen() {
    let dir = scratch("redline-stroke-svg");
    let picture = dir.join("screen.svg");
    let out = run(&[
        "redline",
        CORPUS_DWG,
        CORPUS_DWG,
        "-o",
        arg(&picture),
        "--stroke",
        "3",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let svg = std::fs::read_to_string(&picture).unwrap();
    assert!(
        svg.contains("<style>*{vector-effect:non-scaling-stroke;stroke-width:3px}</style>"),
        "{}",
        &svg[..svg.len().min(400)]
    );
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

/// The top-level fields of each verb's answer. A verb's description names
/// every one, so that a caller reading only the tool list knows where in
/// the answer to look; a field an answer carries that is not here fails the
/// test below, and the description is updated with it.
const ANSWER_FIELDS: [(&str, &[&str]); 5] = [
    (
        "summarize",
        &[
            "units",
            "drawing_ids",
            "entity_count",
            "by_type",
            "layers",
            "blocks",
            "unresolved_inserts",
            "attributes",
            "labelled_texts",
            "unplaced_texts",
            "dimensions",
            "tolerance_frames",
            "extents",
            "signature",
            "selection",
            "confidence",
            "warnings",
        ],
    ),
    (
        "hit_test",
        &[
            "hits",
            "enclosing",
            "unsupported",
            "not_searched",
            "hits_total",
        ],
    ),
    (
        "diff",
        &[
            "matching",
            "tolerance",
            "pairing",
            "unscored",
            "lineage",
            "changes",
            "omitted",
        ],
    ),
    (
        "set",
        &["matching", "tolerance", "lineage", "changes", "omitted"],
    ),
    (
        "redline",
        &[
            "marked",
            "not_marked",
            "view_box",
            "origin",
            "stroke_width",
            "proposal_color",
            "proposal_color_conflicts",
            "left_out",
        ],
    ),
];

#[test]
fn every_verb_description_names_every_field_of_its_answer() {
    let mut mcp = Mcp::start();
    let list = mcp.request("tools/list", json!({}));
    let tools = list["result"]["tools"].as_array().expect("tools");
    for (verb, fields) in ANSWER_FIELDS {
        let tool = tools
            .iter()
            .find(|t| t["name"] == verb)
            .unwrap_or_else(|| panic!("{verb} is listed"));
        let description = tool["description"].as_str().unwrap();
        for field in fields {
            assert!(
                description.contains(&format!("`{field}`")),
                "{verb}'s description does not name `{field}`: {description}"
            );
        }
    }

    // What the answers carry is in the table: every top-level field of
    // answers that reach the optional ones too.
    let dir = scratch("fields");
    let (model, red) = (dir.join("set.json"), dir.join("red.svg"));
    let (model_arg, red_arg) = (arg(&model), arg(&red));
    let id = circle_id(&dir);
    let (cx, cy, r) = circle();
    let calls: Vec<(&str, Vec<String>)> = vec![
        ("summarize", vec!["summarize".into(), CORPUS_DWG.into()]),
        (
            "summarize",
            vec![
                "summarize".into(),
                CORPUS_DWG.into(),
                "--type".into(),
                "circle".into(),
                "--detail".into(),
            ],
        ),
        (
            "hit_test",
            vec![
                "hit-test".into(),
                CORPUS_DWG.into(),
                "--x".into(),
                (cx + r).to_string(),
                "--y".into(),
                cy.to_string(),
                "--tolerance".into(),
                (r * 3.0).to_string(),
                "--limit".into(),
                "1".into(),
            ],
        ),
        (
            "diff",
            vec![
                "diff".into(),
                CORPUS_DWG.into(),
                CORPUS_DXF.into(),
                "--matching".into(),
                "geometry".into(),
                "--max-pairs".into(),
                "0".into(),
                "--omit".into(),
                "within".into(),
            ],
        ),
        (
            "set",
            vec![
                "set".into(),
                CORPUS_DWG.into(),
                "--id".into(),
                id.clone(),
                "--path".into(),
                "radius".into(),
                "--value".into(),
                (r * 2.0).to_string(),
                "-o".into(),
                model_arg.into(),
            ],
        ),
        (
            "redline",
            vec![
                "redline".into(),
                CORPUS_DWG.into(),
                model_arg.into(),
                "--matching".into(),
                "reference".into(),
                "-o".into(),
                red_arg.into(),
            ],
        ),
    ];
    for (verb, args) in calls {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let (_, value) = answer(&args);
        let fields = ANSWER_FIELDS.iter().find(|(v, _)| *v == verb).unwrap().1;
        for key in value.as_object().expect("an object").keys() {
            assert!(
                fields.contains(&key.as_str()),
                "{verb} answers `{key}`, which ANSWER_FIELDS (and the description) lack"
            );
        }
    }
}

#[test]
fn a_verbs_help_explains_every_argument_its_tool_schema_does() {
    // The command line and the MCP tool are one table: whatever the schema
    // tells an agent about an argument, `--help` tells a person.
    let mut mcp = Mcp::start();
    let list = mcp.request("tools/list", json!({}));
    let flat = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    for tool in list["result"]["tools"].as_array().expect("tools") {
        let name = tool["name"].as_str().unwrap();
        let out = run(&[&name.replace('_', "-"), "--help"]);
        assert!(out.status.success(), "{name} --help");
        let help = flat(&String::from_utf8_lossy(&out.stdout));
        assert!(help.contains("Arguments:"), "{name}: {help}");
        let properties = tool["inputSchema"]["properties"].as_object().unwrap();
        for (arg, schema) in properties {
            let description = flat(schema["description"].as_str().unwrap());
            assert!(
                help.contains(&description),
                "{name} --help does not explain `{arg}`: {description}"
            );
        }
    }
}

/// A solid read in full that has no edge -- the one whole torus among the 58
/// solids of `2007/ATMOS-DC22S.dwg` -- is named by its id as having nothing to
/// draw; its type is not called unsupported, since the other 57 are drawn.
#[test]
fn a_picture_names_a_solid_with_no_edge_and_does_not_call_its_type_unsupported() {
    let dir = scratch("edgeless");
    let input = corpus("2007/ATMOS-DC22S.dwg");
    let picture = dir.join("atmos.svg");
    let out = run(&[&input, "-o", arg(&picture)]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(!stderr.contains("unsupported entity types"), "{stderr}");
    let line = stderr
        .lines()
        .find(|l| l.contains("having no edge to draw"))
        .unwrap_or_else(|| panic!("{stderr}"));
    assert!(line.ends_with(": 1106"), "{line}");
}

/// The balloons the tests draw: one on the circle, one on a point, one gone
/// (an entity that does not exist, which the picture cannot show and says so).
fn balloons_for(id: &str) -> Value {
    let (cx, cy, _) = circle();
    json!([
        {"label": "1", "target": {"entity": id.parse::<u64>().unwrap()}},
        {"label": "5.1", "target": {"point": [cx, cy]}, "state": "dropped"},
        {"label": "7", "target": {"entity": 4_000_000_000_u64}},
    ])
}

/// What the verb answers and writes is what the renderer gives for the same
/// drawing and balloons, called directly -- the verb adds no rule of its own.
#[test]
fn balloon_answers_what_the_renderer_draws() {
    let dir = scratch("balloon");
    let id = circle_id(&dir);
    let balloons = balloons_for(&id);
    let picture = dir.join("balloons.svg");
    let (line, report) = answer(&[
        "balloon",
        CORPUS_DWG,
        "--balloons",
        &balloons.to_string(),
        "-o",
        arg(&picture),
    ]);

    let db = uncad::parse_bytes(
        &std::fs::read(CORPUS_DWG).unwrap(),
        uncad::Format::from_path(CORPUS_DWG),
    )
    .expect("the fixture reads");
    let (cx, cy, _) = circle();
    let direct = iron_render_cad::balloons_to_svg(
        &db,
        &[
            iron_render_cad::Balloon::new(
                "1",
                iron_render_cad::BalloonTarget::Entity(uncad::model::EntityId::new(
                    id.parse().unwrap(),
                )),
            ),
            iron_render_cad::Balloon {
                state: iron_render_cad::BalloonState::Dropped,
                ..iron_render_cad::Balloon::new(
                    "5.1",
                    iron_render_cad::BalloonTarget::Point(uncad::model::Point2D { x: cx, y: cy }),
                )
            },
            iron_render_cad::Balloon::new(
                "7",
                iron_render_cad::BalloonTarget::Entity(uncad::model::EntityId::new(4_000_000_000)),
            ),
        ],
        iron_render_cad::BalloonOptions::default(),
    );
    assert_eq!(line, serde_json::to_string(&direct).unwrap());
    assert_eq!(std::fs::read_to_string(&picture).unwrap(), direct.svg);

    // Every balloon is told: two drawn, one named with its reason.
    assert_eq!(report["placed"].as_array().unwrap().len(), 2, "{report}");
    let not_placed = report["not_placed"].as_array().unwrap();
    assert_eq!(not_placed.len(), 1, "{report}");
    assert_eq!(not_placed[0]["index"], 2, "{report}");
    assert_eq!(not_placed[0]["label"], "7", "{report}");
    assert_eq!(report["placed"][1]["label"], "5.1", "{report}");

    // `@file` is the command line's way to give the balloons from a file;
    // the MCP tool answers the same bytes.
    let again = dir.join("again.svg");
    let from_file = dir.join("balloons.json");
    std::fs::write(&from_file, balloons.to_string()).unwrap();
    let at_file = format!("@{}", arg(&from_file));
    let (same, _) = answer(&[
        "balloon",
        CORPUS_DWG,
        "--balloons",
        &at_file,
        "-o",
        arg(&again),
    ]);
    assert_eq!(same, line);
    assert_eq!(
        std::fs::read(&again).unwrap(),
        std::fs::read(&picture).unwrap()
    );
    let mcp_picture = dir.join("mcp.svg");
    let mut mcp = Mcp::start();
    let result = mcp.call(
        "balloon",
        json!({"input": CORPUS_DWG, "balloons": balloons, "output": arg(&mcp_picture)}),
    );
    assert_ne!(result["isError"], true, "{result}");
    assert_eq!(result["content"][0]["text"].as_str().unwrap(), line);
    assert_eq!(
        std::fs::read(&mcp_picture).unwrap(),
        std::fs::read(&picture).unwrap()
    );
}

#[test]
fn balloon_options_change_the_picture() {
    let dir = scratch("balloon-options");
    let id = circle_id(&dir);
    let balloons = json!([{"label": "1", "target": {"entity": id.parse::<u64>().unwrap()}}]);
    let text = balloons.to_string();
    let plain = dir.join("plain.svg");
    let (_, base) = answer(&[
        "balloon",
        CORPUS_DWG,
        "--balloons",
        &text,
        "-o",
        arg(&plain),
    ]);

    let colored = dir.join("colored.svg");
    let (_, report) = answer(&[
        "balloon",
        CORPUS_DWG,
        "--balloons",
        &text,
        "--color",
        "#123456",
        "-o",
        arg(&colored),
    ]);
    assert_eq!(report["color"], "#123456", "{report}");
    assert_ne!(base["color"], report["color"]);

    let sized = dir.join("sized.svg");
    let (_, big) = answer(&[
        "balloon",
        CORPUS_DWG,
        "--balloons",
        &text,
        "--size",
        "50",
        "-o",
        arg(&sized),
    ]);
    assert_eq!(big["placed"][0]["radius"], 25.0, "{big}");
    assert_ne!(base["placed"][0]["radius"], big["placed"][0]["radius"]);
}

#[test]
fn balloon_refuses_input_it_cannot_draw_and_names_where() {
    let dir = scratch("balloon-refuse");
    let id: u64 = circle_id(&dir).parse().unwrap();
    let long = "x".repeat(65);
    let many: Vec<Value> = (0..1001)
        .map(|n| json!({"label": n.to_string(), "target": {"entity": id}}))
        .collect();
    let cases: [(Value, &str); 10] = [
        (json!({"label": "1"}), "must be an array"),
        (json!(["1"]), "balloons[0] must be an object"),
        (
            json!([{"target": {"entity": id}}]),
            "balloons[0] needs a 'label'",
        ),
        (
            json!([{"label": "", "target": {"entity": id}}]),
            "needs a 'label'",
        ),
        (
            json!([{"label": long, "target": {"entity": id}}]),
            "balloons[0].label is 65 characters",
        ),
        (
            json!([{"label": "1", "target": {"entity": id}}, {"label": "2"}]),
            "balloons[1] needs a 'target'",
        ),
        (
            json!([{"label": "1", "target": {"entity": id, "point": [0, 0]}}]),
            "exactly one of 'entity' and 'point'",
        ),
        (
            json!([{"label": "1", "target": {"point": [0]}}]),
            "two finite numbers",
        ),
        (
            json!([{"label": "1", "target": {"entity": id}, "state": "gone"}]),
            "must be \"active\" or \"dropped\"",
        ),
        (Value::Array(many), "at most 1000"),
    ];
    for (n, (balloons, expected)) in cases.iter().enumerate() {
        let picture = dir.join(format!("refused-{n}.svg"));
        // From a file: a thousand balloons do not fit a command line.
        let listed = dir.join(format!("refused-{n}.json"));
        std::fs::write(&listed, balloons.to_string()).unwrap();
        let out = run(&[
            "balloon",
            CORPUS_DWG,
            "--balloons",
            &format!("@{}", arg(&listed)),
            "-o",
            arg(&picture),
        ]);
        assert!(!out.status.success(), "{balloons}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(expected), "{expected}: {stderr}");
        assert!(!picture.exists(), "a refused call writes nothing: {n}");
    }
    let one = json!([{"label": "1", "target": {"entity": id}}]).to_string();
    let taken = dir.join("taken.svg");
    std::fs::write(&taken, "keep").unwrap();
    let out = run(&["balloon", CORPUS_DWG, "--balloons", &one, "-o", arg(&taken)]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("already exists"));
    assert_eq!(std::fs::read_to_string(&taken).unwrap(), "keep");

    let wrong = dir.join("picture.png");
    let out = run(&["balloon", CORPUS_DWG, "--balloons", &one, "-o", arg(&wrong)]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("does not end in .svg"));
    assert!(!wrong.exists());
}
