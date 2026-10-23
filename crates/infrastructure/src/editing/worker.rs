use super::{model_directory, runtime, WORKER};
use crate::ingest::storage;
use bdm_ports::IngestError;
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
        Mutex, TryLockError,
    },
    time::{Duration, Instant, SystemTime},
};

// One cached model keeps repeated edits fast without retaining both sets of weights.
static WORKER_CACHE: Mutex<Option<Worker>> = Mutex::new(None);

struct Worker {
    child: Child,
    input: ChildStdin,
    output: Receiver<String>,
    python: PathBuf,
    weights: PathBuf,
    modified: SystemTime,
    length: u64,
    request: u64,
}

impl Worker {
    fn spawn(root: &Path, model: &str) -> Result<Self, IngestError> {
        let python = runtime(root);
        let weights = storage(
            model_directory(root)
                .join(format!("{model}.pt"))
                .canonicalize(),
        )?;
        let metadata = storage(fs::metadata(&weights))?;
        let mut command = Command::new(&python);
        command
            .args(["-u", "-c", WORKER, "--model", model, "--model-dir"])
            .arg(model_directory(root))
            .arg("--stream")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = storage(command.spawn())?;
        let input = child.stdin.take().ok_or(IngestError::Storage)?;
        let stdout = child.stdout.take().ok_or(IngestError::Storage)?;
        let (sender, output) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                match line {
                    Ok(line) => {
                        if sender.send(line).is_err() {
                            break;
                        }
                    }
                    _ => break,
                }
            }
        });
        Ok(Self {
            child,
            input,
            output,
            python,
            weights,
            modified: storage(metadata.modified())?,
            length: metadata.len(),
            request: 0,
        })
    }

    fn compatible(&mut self, root: &Path, model: &str) -> bool {
        let weights = model_directory(root).join(format!("{model}.pt"));
        self.python == runtime(root)
            && weights
                .canonicalize()
                .is_ok_and(|path| path == self.weights)
            && fs::metadata(weights).is_ok_and(|metadata| {
                metadata.len() == self.length
                    && metadata.modified().is_ok_and(|time| time == self.modified)
            })
            && self.child.try_wait().is_ok_and(|status| status.is_none())
    }

    fn infer(
        &mut self,
        image: &Path,
        mask: &Path,
        output: &Path,
        cancel: &AtomicBool,
    ) -> Result<(), IngestError> {
        self.request += 1;
        storage(writeln!(
            self.input,
            "{}",
            serde_json::json!({"id": self.request, "image": image, "mask": mask, "output": output})
        ))?;
        storage(self.input.flush())?;
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(IngestError::Cancelled);
            }
            if Instant::now() >= deadline {
                return Err(IngestError::StorageDetails("本地修复超时，请重试。".into()));
            }
            match self.output.recv_timeout(Duration::from_millis(50)) {
                Ok(line) => {
                    let response: serde_json::Value = storage(serde_json::from_str(&line))?;
                    if response["id"].as_u64() != Some(self.request) {
                        return Err(IngestError::Integrity);
                    }
                    return match response["error"].as_str() {
                        Some(error) => Err(IngestError::StorageDetails(error.into())),
                        None if response["device"].as_str().is_some() => Ok(()),
                        _ => Err(IngestError::Integrity),
                    };
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(IngestError::StorageDetails(
                        "本地修复进程已退出，请重试。".into(),
                    ));
                }
            }
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(super) fn infer(
    root: &Path,
    model: &str,
    image: &Path,
    mask: &Path,
    output: &Path,
    cancel: &AtomicBool,
) -> Result<(), IngestError> {
    let mut cache = loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(IngestError::Cancelled);
        }
        match WORKER_CACHE.try_lock() {
            Ok(cache) => break cache,
            Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(50)),
            Err(TryLockError::Poisoned(_)) => return Err(IngestError::Storage),
        }
    };
    if !cache
        .as_mut()
        .is_some_and(|worker| worker.compatible(root, model))
    {
        // Drop the old GPU model before loading its replacement.
        *cache = None;
        *cache = Some(Worker::spawn(root, model)?);
    }
    let result = cache
        .as_mut()
        .ok_or(IngestError::Storage)?
        .infer(image, mask, output, cancel);
    if result.is_err() {
        // Killing a cancelled/failed worker also discards its pending response.
        *cache = None;
    }
    result
}
