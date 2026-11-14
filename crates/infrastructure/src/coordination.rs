use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex, OnceLock, Weak},
};

#[derive(Default)]
pub(crate) struct MediaLock {
    local: Mutex<()>,
    file: Option<std::path::PathBuf>,
}

pub(crate) struct MediaGuard<'a> {
    _file: Option<std::fs::File>,
    _local: std::sync::MutexGuard<'a, ()>,
}

impl MediaLock {
    pub(crate) fn lock(&self) -> Result<MediaGuard<'_>, ()> {
        let local = self.local.lock().map_err(|_| ())?;
        let file = self
            .file
            .as_ref()
            .map(|path| {
                let file = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(path)
                    .map_err(|_| ())?;
                fs2::FileExt::lock_exclusive(&file).map_err(|_| ())?;
                Ok::<_, ()>(file)
            })
            .transpose()?;
        Ok(MediaGuard {
            _file: file,
            _local: local,
        })
    }
}

/// Shared across catalog connections to the same file. Only short media mutation
/// phases take this lock; downloads and model inference run independently.
pub(crate) fn media_lock(path: &Path) -> Arc<MediaLock> {
    static LOCKS: OnceLock<Mutex<HashMap<std::path::PathBuf, Weak<MediaLock>>>> = OnceLock::new();
    let key = path.canonicalize().unwrap_or_else(|_| path.to_owned());
    let mut locks = LOCKS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(MediaLock {
        local: Mutex::new(()),
        file: Some(key.with_extension("media.lock")),
    });
    locks.insert(key, Arc::downgrade(&lock));
    lock
}
