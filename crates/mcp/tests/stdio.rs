use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[test]
fn stdio_initialize_lists_tools_and_creates_a_plan() {
    let dir = temp_dir();
    let bin = std::env::var("CARGO_BIN_EXE_term_organize_mcp")
        .or_else(|_| std::env::var("CARGO_BIN_EXE_term-organize-mcp"))
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/debug/term-organize-mcp")
                .to_string_lossy()
                .into_owned()
        });
    let mut child = Command::new(bin)
        .arg("--data-dir")
        .arg(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn term-organize-mcp");
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(line) => {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let mut send = |message: Value| {
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    };
    send(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "term-organize-test", "version": "0.0.0" }
        }
    }));
    let initialized = wait_id(&rx, 1);
    assert_eq!(
        initialized["result"]["protocolVersion"], "2025-06-18",
        "{initialized}"
    );
    assert!(initialized["result"]["instructions"]
        .as_str()
        .unwrap()
        .contains("Not affiliated"));

    send(json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {}
    }));
    send(json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list"
    }));
    let listed = wait_id(&rx, 2);
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"list_terms"));
    assert!(names.contains(&"create_plan"));
    assert!(names.contains(&"delete_plan"));

    send(json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": { "name": "list_terms", "arguments": {} }
    }));
    let terms = wait_id(&rx, 3);
    let body = structured(&terms);
    let codes: Vec<&str> = body["terms"]
        .as_array()
        .unwrap()
        .iter()
        .map(|term| term["term"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["202730", "202710"]);

    send(json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "create_plan",
            "arguments": { "name": "From stdio", "terms": ["Fall 2026"] }
        }
    }));
    let created = wait_id(&rx, 4);
    let plan = structured(&created);
    assert_eq!(plan["name"], "From stdio");
    assert_eq!(plan["revision"], 1);
    assert!(plan["id"].as_str().unwrap().len() > 8);

    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn read_only_call_returns_read_only_and_hides_writes() {
    let dir = temp_dir();
    let bin = std::env::var("CARGO_BIN_EXE_term_organize_mcp")
        .or_else(|_| std::env::var("CARGO_BIN_EXE_term-organize-mcp"))
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/debug/term-organize-mcp")
                .to_string_lossy()
                .into_owned()
        });
    let mut child = Command::new(bin)
        .args(["--read-only", "--data-dir"])
        .arg(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn term-organize-mcp");
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(line) => {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    let mut send = |message: Value| {
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    };
    send(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "term-organize-test", "version": "0.0.0" }
        }
    }));
    let _initialized = wait_id(&rx, 1);
    send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
    send(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));
    let listed = wait_id(&rx, 2);
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"list_plans"));
    assert!(!names.contains(&"create_plan"));
    send(json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": { "name": "create_plan", "arguments": { "name": "Nope" } }
    }));
    let called = wait_id(&rx, 3);
    assert_eq!(called["result"]["isError"], true);
    let body = structured(&called);
    assert_eq!(body["error"]["code"], "read_only");
    assert!(!body["error"]["message"].as_str().unwrap().is_empty());
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn argument_errors_use_the_invalid_argument_object() {
    let dir = temp_dir();
    let bin = std::env::var("CARGO_BIN_EXE_term_organize_mcp")
        .or_else(|_| std::env::var("CARGO_BIN_EXE_term-organize-mcp"))
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/debug/term-organize-mcp")
                .to_string_lossy()
                .into_owned()
        });
    let mut child = Command::new(bin)
        .arg("--data-dir")
        .arg(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn term-organize-mcp");
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(line) => {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    let mut send = |message: Value| {
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    };
    send(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "term-organize-test", "version": "0.0.0" }
        }
    }));
    let _initialized = wait_id(&rx, 1);
    send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));

    let cases = [
        json!({ "name": "get_plan", "arguments": {} }),
        json!({ "name": "create_plan", "arguments": { "name": "Bad terms", "terms": [{ "term": "Fall 2026" }] } }),
        json!({ "name": "rename_plan", "arguments": { "plan_id": "11111111-1111-1111-1111-111111111111", "name": "Untitled" } }),
        json!({ "name": "rename_plan", "arguments": { "plan_id": "11111111-1111-1111-1111-111111111111", "name": "Untitled", "expected_revision": "1" } }),
    ];
    for (index, arguments) in cases.into_iter().enumerate() {
        let id = (index + 2) as u64;
        send(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": arguments,
        }));
        let called = wait_id(&rx, id);
        assert_eq!(called["result"]["isError"], true, "{called}");
        let body = structured(&called);
        assert_eq!(body["error"]["code"], "invalid_argument", "{called}");
        assert!(
            !body["error"]["message"].as_str().unwrap().is_empty(),
            "{called}"
        );
    }

    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
}

fn structured(message: &Value) -> Value {
    message["result"]["structuredContent"].clone()
}

fn wait_id(rx: &mpsc::Receiver<String>, id: u64) -> Value {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(30) {
        match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(line) => {
                let value: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
                if value.get("id").and_then(|value| value.as_u64()) == Some(id) {
                    assert!(value.get("error").is_none(), "{line}");
                    return value;
                }
            }
            Err(_) => break,
        }
    }
    panic!("timed out waiting for response {id}");
}

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "termorganize-mcp-stdio-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
