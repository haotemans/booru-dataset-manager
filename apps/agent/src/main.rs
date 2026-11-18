//! Headless entry point. JSON stdout is reserved for machine-readable results.
use bdm_infrastructure::{
    agent::{jobs, operation_schema, AgentService, SCHEMA_OPERATIONS},
    catalog::SqliteCatalog,
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, io::Read, path::PathBuf};

const HELP: &str = "bdm <operation> --catalog ABS_DB --project-root ABS_DIR --library-root ABS_DIR [--input FILE|-] [--output IMAGE] [--allow-writes]
Schema discovery (no database required): bdm schema [operation]
Operations: policy, describe, doctor, status, collections, list, view, bucket_plan,
character_repair (media_ids, apply=false by default; existing verified rules only),
job_plan, job_submit, jobs_list, job_status, job_control, jobs_tick,
download_submit, download_pixiv_submit, download_status, download_control,
work_claim, work_view, work_preview, work_apply, work_assess, work_annotate,
work_next (claim+view), work_apply_view (apply+view), work_finish (final assessment+annotation),
work_heartbeat, work_release, daemon-serve, daemon_stop, mcp-serve, init.
download_submit source: automatic|danbooru|safebooru|gelbooru. Safebooru uses safebooru.donmai.us (General only, shared Danbooru account/IDs).
Readonly by default. Inputs are JSON objects; '-' reads stdin. View image bytes
are written only to an explicit --output path. No GUI or hosted model is started.
Preview options in view/work_view/work_preview: preview={max_edge:256..2048,format:png|jpeg};
work operations put preview inside payload. Default PNG; JPEG display previews require .jpg/.jpeg output.
work_next uses top-level preview; work_apply_view uses payload={crop,preview}. These two new image operations default to JPEG.
Stored training images and native PNG crops are never recompressed by preview options.
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
    if operation == "schema" {
        let requested = args.next();
        if args.next().is_some() {
            return Err("Usage: bdm schema [operation] (no database flags needed)".into());
        }
        let value = if let Some(operation) = requested {
            json!({"operation":operation,"input_schema":operation_schema(&operation).ok_or("Unknown schema operation")?,"display_defaults":"Legacy CLI image operations default to PNG; compound CLI image operations and typed MCP work tools default to JPEG. Set preview.format explicitly to choose.","envelope":"work payload excludes server-owned identity/overwrite/stage fields"})
        } else {
            json!({"operations":SCHEMA_OPERATIONS})
        };
        println!("{value}");
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
    if matches!(
        operation.as_str(),
        "view" | "work_view" | "work_preview" | "work_next" | "work_apply_view"
    ) {
        let output=flags.get("--output").ok_or("Image operations require --output ABS_PNG; inspect that image before submitting decisions")?;
        let p = PathBuf::from(output);
        if !p.is_absolute() || p.exists() {
            return Err("--output must be an absolute unused image path".into());
        }
        let preview = if operation == "view" || operation == "work_next" {
            &input["preview"]
        } else {
            &input["payload"]["preview"]
        };
        let jpeg = if preview.is_null() {
            matches!(operation.as_str(), "work_next" | "work_apply_view")
        } else {
            preview["format"] == "jpeg"
        };
        let extension = p
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if (jpeg && !matches!(extension.as_str(), "jpg" | "jpeg")) || (!jpeg && extension != "png")
        {
            return Err("--output extension must match the preview format (.png by default, .jpg/.jpeg for JPEG)".into());
        }
    }
    let mut result = service.execute(&operation, input);
    if let Some(image) = result.image {
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
            file.write_all(&image.bytes)?;
            result.metadata["image_file"] = json!(path);
            result.metadata["image_mime_type"] = json!(image.mime_type);
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
