use std::{
    backtrace::Backtrace,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;
static DIAGNOSTICS: OnceLock<Diagnostics> = OnceLock::new();

struct Diagnostics {
    directory: PathBuf,
    file: Mutex<File>,
}

fn timestamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub(crate) fn install(directory: &Path) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    let path = directory.join("desktop.log");
    if fs::metadata(&path).is_ok_and(|metadata| metadata.len() >= MAX_LOG_BYTES) {
        let previous = directory.join("desktop.previous.log");
        match fs::remove_file(&previous) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
        fs::rename(&path, previous)?;
    }
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    if DIAGNOSTICS
        .set(Diagnostics {
            directory: directory.to_owned(),
            file: Mutex::new(file),
        })
        .is_err()
    {
        return Ok(());
    }
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        let thread = std::thread::current();
        let report = format!(
            "unix_ms={} pid={} version={} revision={} thread={:?}\n{}\nBacktrace:\n{}\n",
            timestamp(),
            std::process::id(),
            env!("CARGO_PKG_VERSION"),
            env!("BDM_GIT_REVISION"),
            thread.name(),
            panic,
            Backtrace::force_capture()
        );
        if let Some(diagnostics) = DIAGNOSTICS.get() {
            // A panic must not wait for a logging lock held by the same thread.
            // The independent crash file is written even if that lock is busy.
            let path = diagnostics.directory.join(format!(
                "crash-{}-{}.log",
                timestamp(),
                std::process::id()
            ));
            if let Ok(mut file) = OpenOptions::new().create_new(true).write(true).open(&path) {
                let _ = file.write_all(report.as_bytes());
                let _ = file.sync_all();
            }
            if let Ok(mut file) = diagnostics.file.try_lock() {
                let _ = writeln!(file, "PANIC crash_report={}\n{report}", path.display());
                let _ = file.sync_all();
            }
        }
        previous_hook(panic);
    }));
    event(format!(
        "startup version={} revision={} os={} arch={} debug={} executable={:?}",
        env!("CARGO_PKG_VERSION"),
        env!("BDM_GIT_REVISION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        cfg!(debug_assertions),
        std::env::current_exe().ok()
    ));
    Ok(())
}

pub(crate) fn event(message: impl AsRef<str>) {
    let Some(diagnostics) = DIAGNOSTICS.get() else {
        return;
    };
    if let Ok(mut file) = diagnostics.file.lock() {
        // Escape newlines so a single action always occupies one log line.
        let message = message.as_ref().replace(['\r', '\n'], " ");
        let _ = writeln!(
            file,
            "unix_ms={} pid={} {message}",
            timestamp(),
            std::process::id()
        );
        let _ = file.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_child() {
        let Some(directory) = std::env::var_os("BDM_DIAGNOSTICS_TEST_DIRECTORY") else {
            return;
        };
        install(Path::new(&directory)).unwrap();
        event("crop_ratio media=42 from=free to=16:9 dimensions=8507x3902");
        if std::env::var_os("BDM_DIAGNOSTICS_TEST_PANIC").is_some() {
            panic!("synthetic crop panic for diagnostic regression");
        }
        event("shutdown result=ok");
    }

    #[test]
    fn panic_report_and_recent_actions_survive_process_exit() {
        let directory = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "diagnostics::tests::panic_child", "--nocapture"])
            .env("BDM_DIAGNOSTICS_TEST_DIRECTORY", directory.path())
            .env("BDM_DIAGNOSTICS_TEST_PANIC", "1")
            .output()
            .unwrap();
        assert!(!output.status.success());
        let log = fs::read_to_string(directory.path().join("desktop.log")).unwrap();
        assert!(log.contains("startup version="));
        assert!(log.contains("revision="));
        assert!(log.contains("to=16:9 dimensions=8507x3902"));
        assert!(log.contains("PANIC crash_report="));
        let crash = fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("crash-")
            })
            .unwrap();
        let report = fs::read_to_string(crash).unwrap();
        assert!(report.contains("synthetic crop panic"));
        assert!(report.contains("diagnostics.rs:"));
        assert!(report.contains("Backtrace:"));
        assert!(report.contains("panic_child"));
    }

    #[test]
    fn oversized_log_rotates_without_losing_previous_session() {
        let directory = tempfile::tempdir().unwrap();
        let file = File::create(directory.path().join("desktop.log")).unwrap();
        file.set_len(MAX_LOG_BYTES).unwrap();
        fs::write(
            directory.path().join("desktop.previous.log"),
            "older session",
        )
        .unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "diagnostics::tests::panic_child"])
            .env("BDM_DIAGNOSTICS_TEST_DIRECTORY", directory.path())
            .env_remove("BDM_DIAGNOSTICS_TEST_PANIC")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            fs::metadata(directory.path().join("desktop.previous.log"))
                .unwrap()
                .len(),
            MAX_LOG_BYTES
        );
        assert!(fs::read_to_string(directory.path().join("desktop.log"))
            .unwrap()
            .contains("shutdown result=ok"));
    }
}
