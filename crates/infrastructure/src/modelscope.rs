use bdm_config::ModelScopeConfig;
use std::{fs, io, path::Path};

pub fn load_config(root: &Path) -> io::Result<Option<ModelScopeConfig>> {
    let path = root.join("config/modelscope.json");
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes);
    let config: ModelScopeConfig = serde_json::from_slice(bytes)?;
    ModelScopeConfig::new(&config.dataset_id)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
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
}
