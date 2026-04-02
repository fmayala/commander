use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SingletonError {
    #[error("supervisor already running (lock held on {path})")]
    AlreadyRunning { path: PathBuf },
    #[error("failed to acquire lock: {0}")]
    LockFailed(std::io::Error),
}

/// Exclusive flock-based supervisor lock.
///
/// On `commander run`, acquired before opening SQLite or the Unix socket.
/// If the lock is already held, exits with error.
/// Released automatically when dropped (OS releases flock on process death).
pub struct SupervisorLock {
    _file: File,
    path: PathBuf,
}

impl SupervisorLock {
    pub fn acquire(path: &Path) -> Result<Self, SingletonError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(SingletonError::LockFailed)?;
        }

        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(path)
            .map_err(SingletonError::LockFailed)?;

        // Try non-blocking exclusive lock
        if !try_flock_exclusive(&file) {
            return Err(SingletonError::AlreadyRunning {
                path: path.to_path_buf(),
            });
        }

        Ok(Self {
            _file: file,
            path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(unix)]
fn try_flock_exclusive(file: &File) -> bool {
    use std::os::unix::io::AsRawFd;
    let fd = file.as_raw_fd();
    // LOCK_EX | LOCK_NB = exclusive + non-blocking
    let result = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
    result == 0
}

#[cfg(not(unix))]
fn try_flock_exclusive(_file: &File) -> bool {
    // On non-Unix, always succeed (no flock support).
    // Proper Windows support would use LockFileEx.
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn acquire_and_release() {
        let dir = TempDir::new().unwrap();
        let lock_path = dir.path().join("supervisor.lock");

        let lock = SupervisorLock::acquire(&lock_path).unwrap();
        assert!(lock_path.exists());
        drop(lock);
    }

    #[test]
    fn double_acquire_fails() {
        let dir = TempDir::new().unwrap();
        let lock_path = dir.path().join("supervisor.lock");

        let _lock1 = SupervisorLock::acquire(&lock_path).unwrap();
        let result = SupervisorLock::acquire(&lock_path);
        assert!(matches!(result, Err(SingletonError::AlreadyRunning { .. })));
    }

    #[test]
    fn acquire_after_release() {
        let dir = TempDir::new().unwrap();
        let lock_path = dir.path().join("supervisor.lock");

        let lock1 = SupervisorLock::acquire(&lock_path).unwrap();
        drop(lock1);

        let _lock2 = SupervisorLock::acquire(&lock_path).unwrap();
    }
}
