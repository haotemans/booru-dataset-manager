//! Headless entry point. JSON stdout is reserved for machine-readable results.
use bdm_infrastructure::{
    agent::{jobs, AgentService},
    catalog::SqliteCatalog,
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, io::Read, path::PathBuf};

const HELP: &str = "bdm <operation> --catalog ABS_DB --project-root ABS_DIR --library-root ABS_DIR [--input FILE|-] [--output PNG] [--allow-writes]
Operations: policy, describe, doctor, status, collections, list, view, bucket_plan,
job_plan, job_submit, jobs_list, job_status, job_control, jobs_tick,
download_submit, download_status, download_control,
work_claim, work_view, work_preview, work_apply, work_assess, work_annotate,
work_heartbeat, work_release, daemon-serve, daemon_stop, mcp-serve, init.
Readonly by default. Inputs are JSON objects; '-' reads stdin. View image bytes
are written only to an explicit --output path. No GUI or hosted model is started.
Examples: bdm job_plan ... --input plan.json
          bdm job_submit ... --input plan.json --allow-writes
          bdm daemon-serve ... --allow-writes";

fn run() -> Result<bool, Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let Some(operation) = args.next() else {
        println!("{HELP}");
        return Ok(true);
    };
    if matches!(operation.as_str(), "--help" | "-h" | "help") {
        println!("{HELP}");
        return Ok(true);
    }
    let mut flags = BTreeMap::new();
    let mut writes = false;
    while let Some(key) = args.next() {
        if key == "--allow-writes" {
            writes = true;
            continue;
        }
        if ![
            "--catalog",
            "--project-root",
            "--library-root",
            "--input",
            "--output",
        ]
        .contains(&key.as_str())
        {
            return Err(format!("Unknown option: {key}").into());
        }
        let value = args.next().ok_or("Missing option value")?;
        if flags.insert(key, value).is_some() {
            return Err("Duplicate option".into());
        }
    }
    let path = |key: &str| -> Result<PathBuf, Box<dyn std::error::Error>> {
        let p = PathBuf::from(flags.get(key).ok_or_else(|| format!("Required: {key}"))?);
        if !p.is_absolute() {
            return Err(format!("{key} must be absolute").into());
        }
        Ok(p)
    };
    let catalog = path("--catalog")?;
    let project = path("--project-root")?;
    let library = path("--library-root")?;
    if operation == "init" {
        if !writes {
            return Err("init requires --allow-writes".into());
        }
        if !project.is_dir() || !library.is_dir() {
            return Err("Project/library directories must already exist".into());
        }
        let _repo = SqliteCatalog::open(&catalog)?;
        println!("{}", json!({"initialized":true}));
        return Ok(true);
    }
    let service = AgentService::open(&catalog, &project, &library, writes)?;
    if operation == "daemon-serve" {
        jobs::serve_daemon(service)?;
        return Ok(true);
    }
    if operation == "mcp-serve" {
        bdm_infrastructure::agent::serve_stdio(&catalog, &project, &library, writes)?;
        return Ok(true);
    }
    let input: Value = if let Some(file) = flags.get("--input") {
        let mut body = String::new();
        if file == "-" {
            std::io::stdin()
                .take(4 * 1024 * 1024 + 1)
                .read_to_string(&mut body)?;
        } else {
            body = std::fs::read_to_string(file)?;
        }
        if body.len() > 4 * 1024 * 1024 {
            return Err("Input exceeds 4 MiB".into());
        }
        serde_json::from_str(&body)?
    } else {
        json!({})
    };
    if !input.is_object() {
        return Err("Input must be a JSON object".into());
    }
    if matches!(operation.as_str(), "view" | "work_view" | "work_preview") {
        let output=flags.get("--output").ok_or("Image operations require --output ABS_PNG; inspect that image before submitting decisions")?;
        let p = PathBuf::from(output);
        if !p.is_absolute() || p.exists() {
            return Err("--output must be an absolute unused image path".into());
        }
    }
    let mut result = service.execute(&operation, input);
    if let Some(png) = result.image_png {
        if let Some(output) = flags.get("--output") {
            let path = PathBuf::from(output);
            if !path.is_absolute() {
                return Err("--output must be absolute".into());
            }
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(&png)?;
            result.metadata["image_file"] = json!(path);
        } else {
            result.metadata["image_output_required"] = json!(true);
        }
    }
    println!("{}", result.metadata);
    Ok(!result.is_error)
}
fn main() {
    match run() {
        Ok(true) => {}
        Ok(false) => std::process::exit(2),
        Err(e) => {
            println!(
                "{}",
                json!({"error":{"code":"cli_error","message":e.to_string()}})
            );
            std::process::exit(2)
        }
    }
}
