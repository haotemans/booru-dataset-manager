use std::{
    path::Path,
    process::{Command, Output},
};
fn command(root: &Path, operation: &str, writes: bool) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_bdm"));
    cmd.arg(operation)
        .arg("--catalog")
        .arg(root.join("catalog.sqlite3"))
        .arg("--project-root")
        .arg(root)
        .arg("--library-root")
        .arg(root.join("library"));
    if writes {
        cmd.arg("--allow-writes");
    }
    cmd.output().unwrap()
}
#[test]
fn cli_help_init_readonly_errors_and_daemon_protocol_are_headless() {
    let help = Command::new(env!("CARGO_BIN_EXE_bdm"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8(help.stdout).unwrap().contains("No GUI"));
    let root = tempfile::Builder::new()
        .prefix("bdm cli ")
        .tempdir()
        .unwrap();
    std::fs::create_dir(root.path().join("library")).unwrap();
    assert!(!command(root.path(), "policy", false).status.success());
    assert!(!root.path().join("catalog.sqlite3").exists());
    assert!(command(root.path(), "init", true).status.success());
    let policy = command(root.path(), "policy", false);
    assert!(policy.status.success());
    let policy: serde_json::Value = serde_json::from_slice(&policy.stdout).unwrap();
    assert_eq!(policy["protocol"], "bdm-agent-v3");
    assert_eq!(policy["writes_enabled"], false);
    let denied = command(root.path(), "job_submit", false);
    assert!(!denied.status.success());
    let denied: serde_json::Value = serde_json::from_slice(&denied.stdout).unwrap();
    assert_eq!(denied["code"], "read_only");
}

struct ChildGuard(std::process::Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn stdio_mcp_uses_the_same_headless_core_and_exposes_durable_tools() {
    use std::{
        io::{BufRead, BufReader, Write},
        process::Stdio,
        time::Duration,
    };
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("library")).unwrap();
    assert!(command(root.path(), "init", true).status.success());
    let child = Command::new(env!("CARGO_BIN_EXE_bdm"))
        .arg("mcp-serve")
        .arg("--catalog")
        .arg(root.path().join("catalog.sqlite3"))
        .arg("--project-root")
        .arg(root.path())
        .arg("--library-root")
        .arg(root.path().join("library"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(child);
    let mut stdin = child.0.stdin.take().unwrap();
    let stdout = child.0.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    for message in [
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"integration-test","version":"1"}}}),
        serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"bdm_policy","arguments":{}}}),
        serde_json::json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"bdm_job","arguments":{"operation":"job_plan","input":{"mode":"bucket","media_ids":[]}}}}),
        serde_json::json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"bdm_work_annotate","arguments":{"work_id":1,"lease_token":"invalid-test-lease","generation":1,"expected_sha256":"invalid-test-sha","request_id":"invalid-test-request","payload":{"caption":"Do not save","overwrite":true,"reason":"test","model":"test"}}}}),
    ] {
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }
    let mut listed = false;
    let mut policy = false;
    let mut typed_job = false;
    let mut invalid_payload_rejected = false;
    for _ in 0..8 {
        let line = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("MCP response timeout");
        let response: serde_json::Value = serde_json::from_str(&line).unwrap();
        if response["id"] == 2 {
            let tools = response["result"]["tools"].as_array().unwrap();
            listed = [
                "bdm_agent",
                "bdm_job",
                "bdm_download",
                "bdm_work_claim",
                "bdm_work_view",
                "bdm_work_preview",
                "bdm_work_apply",
                "bdm_work_assess",
                "bdm_work_annotate",
                "bdm_work_control",
            ]
            .into_iter()
            .all(|name| {
                tools
                    .iter()
                    .any(|t| t["name"] == name && t["inputSchema"]["type"] == "object")
            });
        }
        if response["id"] == 3 {
            let body: serde_json::Value =
                serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                    .unwrap();
            policy = body["protocol"] == "bdm-agent-v3";
        }
        if response["id"] == 4 {
            let body: serde_json::Value =
                serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                    .unwrap();
            typed_job = body["code"] == "invalid_scope";
        }
        if response["id"] == 5 {
            invalid_payload_rejected =
                response.get("error").is_some() || response["result"]["isError"] == true;
        }
        if listed && policy && typed_job && invalid_payload_rejected {
            break;
        }
    }
    assert!(listed && policy && typed_job && invalid_payload_rejected);
    drop(stdin);
    drop(child);
    reader.join().unwrap();
}

#[test]
fn schema_discovery_needs_no_catalog_and_has_complete_lease_payload() {
    let root = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_bdm"))
        .args(["schema", "work_annotate"])
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let schema: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(schema["input_schema"]["type"], "object");
    assert!(schema["input_schema"]["required"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("lease_token")));
    let text = schema.to_string();
    assert!(
        text.contains("text_regions")
            && text.contains("visual_regions")
            && text.contains("caption")
    );
    assert_eq!(
        std::fs::read_dir(root.path()).unwrap().count(),
        0,
        "schema lookup created files"
    );
    let bad = Command::new(env!("CARGO_BIN_EXE_bdm"))
        .args(["schema", "unknown-test-operation"])
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(2));
}

#[test]
fn daemon_ownership_is_exclusive_and_stop_drains_cleanly() {
    use std::{
        process::Stdio,
        time::{Duration, Instant},
    };
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("library")).unwrap();
    assert!(command(root.path(), "init", true).status.success());
    let child = Command::new(env!("CARGO_BIN_EXE_bdm"))
        .arg("daemon-serve")
        .arg("--catalog")
        .arg(root.path().join("catalog.sqlite3"))
        .arg("--project-root")
        .arg(root.path())
        .arg("--library-root")
        .arg(root.path().join("library"))
        .arg("--allow-writes")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(child);
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let status = command(root.path(), "status", false);
        let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
        if status["daemon"]["alive"] == true {
            break;
        }
        assert!(Instant::now() < deadline, "Daemon never became ready");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!command(root.path(), "daemon-serve", true).status.success());
    assert!(command(root.path(), "daemon_stop", true).status.success());
    while child.0.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "Daemon failed to drain");
        std::thread::sleep(Duration::from_millis(50));
    }
}
