use bdm_config::{ModelScopeConfig, ModelScopeSite};
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub enum Transfer {
    Upload { data: PathBuf, library: PathBuf },
    Receive { directory: PathBuf },
}

#[derive(serde::Deserialize)]
pub struct TransferResult {
    pub backup_id: String,
    pub destination: Option<PathBuf>,
}

pub fn transfer(
    root: &Path,
    site: ModelScopeSite,
    transfer: Transfer,
) -> Result<TransferResult, String> {
    let script = root.join("scripts/modelscope_backup.py");
    if !script.is_file() {
        return Err("找不到项目自带的魔搭备份脚本。请从完整 BDM 目录运行。".into());
    }
    let candidates = if cfg!(windows) {
        [
            "target/modelscope-mcp/venv/Scripts/python.exe",
            "runtime/python.exe",
        ]
    } else {
        [
            "target/modelscope-mcp/venv/bin/python",
            "runtime/bin/python",
        ]
    };
    let python = std::env::var_os("BDM_MODELSCOPE_PYTHON")
        .map(PathBuf::from)
        .or_else(|| {
            candidates
                .into_iter()
                .map(|path| root.join(path))
                .find(|path| path.is_file())
        })
        .unwrap_or_else(|| PathBuf::from("python"));
    let mut command = Command::new(python);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
        .arg(script)
        .arg("--json")
        .arg("--config")
        .arg(root.join("config/modelscope.json"))
        .current_dir(root)
        .env("MODELSCOPE_BACKUP_SITE", site.endpoint())
        .env("PYTHONUTF8", "1")
        .env_remove("MODELSCOPE_TOKEN")
        .env_remove("MODELSCOPE_DATASET_ID")
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    match transfer {
        Transfer::Upload { data, library } => {
            command.arg("backup").arg("--data").arg(&data);
            if library.is_absolute() {
                command.arg("--library").arg(library);
            }
            if let Ok(entries) = fs::read_dir(&data) {
                let mut additional_libraries = Vec::new();
                for entry in entries.flatten() {
                    let path = entry.path();
                    let library_index = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .and_then(|name| name.strip_prefix("library-"))
                        .and_then(|index| index.parse::<u32>().ok());
                    if let Some(index) = library_index.filter(|_| path.is_dir()) {
                        additional_libraries.push((index, path));
                    }
                }
                additional_libraries.sort_unstable();
                for (_, path) in additional_libraries {
                    command.arg("--library").arg(path);
                }
            }
        }
        Transfer::Receive { directory } => {
            command.arg("receive").arg(directory);
        }
    }
    let output = command.output().map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => "找不到魔搭所需的 Python 环境。".to_owned(),
        _ => "无法启动魔搭备份脚本。".to_owned(),
    })?;
    if output.status.success() {
        serde_json::from_slice(&output.stdout).map_err(|_| "魔搭脚本返回了无法识别的结果。".into())
    } else {
        let response: serde_json::Value = serde_json::from_slice(&output.stdout)
            .map_err(|_| "魔搭脚本运行失败，请检查 Python 环境。".to_owned())?;
        let error = response.get("error").and_then(serde_json::Value::as_str);
        Err(match error {
            Some("Remote file is missing: LATEST.json") => {
                "还没有可下载的版本，请先在 A 机上传。".to_owned()
            }
            Some(message) => message.to_owned(),
            None => "魔搭脚本运行失败。".to_owned(),
        })
    }
}

pub fn load_config(root: &Path) -> io::Result<Option<ModelScopeConfig>> {
    let path = root.join("config/modelscope.json");
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes);
    let config: ModelScopeConfig = serde_json::from_slice(bytes)?;
    let mut validated = ModelScopeConfig::new(&config.dataset_id)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    validated.site = config.site;
    Ok(Some(validated))
}

pub fn save_config(root: &Path, config: &ModelScopeConfig) -> io::Result<()> {
    let directory = root.join("config");
    fs::create_dir_all(&directory)?;
    let mut temporary = tempfile::NamedTempFile::new_in(&directory)?;
    serde_json::to_writer_pretty(&mut temporary, config)?;
    temporary.as_file().sync_all()?;
    temporary.persist(directory.join("modelscope.json"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_only_the_dataset_id_and_replaces_existing_config() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(load_config(root.path()).unwrap(), None);
        for dataset in ["owner/old", " owner/new "] {
            let config = ModelScopeConfig::new(dataset).unwrap();
            save_config(root.path(), &config).unwrap();
            assert_eq!(load_config(root.path()).unwrap(), Some(config));
        }
        assert_eq!(
            fs::read_to_string(root.path().join("config/modelscope.json")).unwrap(),
            "{\n  \"dataset_id\": \"owner/new\"\n}"
        );
        assert!(ModelScopeConfig::new("https://modelscope.cn/datasets/owner/new").is_err());
    }

    #[test]
    fn persists_the_selected_site_without_a_token() {
        let root = tempfile::tempdir().unwrap();
        let mut config = ModelScopeConfig::new("owner/data").unwrap();
        config.site = Some(ModelScopeSite::International);
        save_config(root.path(), &config).unwrap();
        assert_eq!(load_config(root.path()).unwrap(), Some(config));
        let bytes = fs::read(root.path().join("config/modelscope.json")).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"dataset_id": "owner/data", "site": "modelscope.ai"})
        );
    }
}
