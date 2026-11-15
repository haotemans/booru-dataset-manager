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
    ] {
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }
    let mut listed = false;
    let mut policy = false;
    for _ in 0..8 {
        let line = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("MCP response timeout");
        let response: serde_json::Value = serde_json::from_str(&line).unwrap();
        if response["id"] == 2 {
            listed = response["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["name"] == "bdm_agent");
        }
        if response["id"] == 3 {
            let body: serde_json::Value =
                serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                    .unwrap();
            policy = body["protocol"] == "bdm-agent-v3";
        }
        if listed && policy {
            break;
        }
    }
    assert!(listed && policy);
    drop(stdin);
    drop(child);
    reader.join().unwrap();
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
