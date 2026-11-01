use std::path::{Path, PathBuf};

pub fn project_root() -> PathBuf {
    let executable = std::env::current_exe().ok();
    // A portable package must never inherit a nearby development catalog.
    if let Some(directory) = executable.as_deref().and_then(portable_root) {
        return directory;
    }
    executable
        .as_deref()
        .and_then(repository_root)
        .unwrap_or_else(|| {
            executable
                .as_deref()
                .and_then(Path::parent)
                .unwrap_or(Path::new("."))
                .to_owned()
        })
}

fn portable_root(executable: &Path) -> Option<PathBuf> {
    let directory = executable.parent()?;
    directory
        .join("portable.flag")
        .is_file()
        .then(|| directory.to_owned())
}

fn repository_root(executable: &Path) -> Option<PathBuf> {
    executable
        .parent()?
        .ancestors()
        .find(|directory| {
            directory.join("Cargo.toml").is_file() && directory.join("apps/desktop").is_dir()
        })
        .map(Path::to_owned)
}

pub fn data_directory() -> PathBuf {
    project_root().join("data")
}

pub fn library_directory() -> PathBuf {
    data_directory().join("library")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_marker_takes_precedence_over_repository_discovery() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(directory.path().join("apps/desktop")).unwrap();
        std::fs::write(directory.path().join("Cargo.toml"), "").unwrap();
        let portable = directory.path().join("dist/portable");
        std::fs::create_dir_all(&portable).unwrap();
        let executable = portable.join("bdm-desktop.exe");
        assert!(portable_root(&executable).is_none());
        assert_eq!(
            repository_root(&executable),
            Some(directory.path().to_owned())
        );
        std::fs::write(portable.join("portable.flag"), "").unwrap();
        assert_eq!(portable_root(&executable), Some(portable));
    }
    #[test]
    fn development_defaults_stay_in_repository() {
        let root = project_root();
        assert!(root.join("Cargo.toml").is_file());
        assert_eq!(data_directory(), root.join("data"));
        assert_eq!(library_directory(), root.join("data/library"));
        let app =
            crate::app::BdmApp::from_preferences(crate::model::Preferences::default(), false, None);
        assert_eq!(PathBuf::from(app.prefs.root), library_directory());
        assert_eq!(app.root_draft, library_directory().display().to_string());
    }
}
