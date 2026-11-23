//! One process at a time per device.
//!
//! The CLI keeps a device's MLS state and message store in files and writes
//! them back after each operation. Two processes for one device would each
//! ack frames the other never sees and overwrite each other's state; a copy
//! that missed a removal would go on encrypting to a removed member. So a
//! command that opens a device holds an exclusive lock on it until it
//! exits, and a second command waits for the first.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long a command waits for another command on the same device.
pub const WAIT: Duration = Duration::from_secs(120);

/// An exclusive lock on one device's state; released on drop.
#[derive(Debug)]
pub struct DeviceLock {
    _file: File,
}

impl DeviceLock {
    /// Take the lock for the device whose session lives in `state_file`,
    /// waiting up to `wait` for another process to release it.
    pub fn acquire(state_file: &Path, wait: Duration) -> Result<Self, String> {
        let path = lock_path(state_file);
        let mut options = OpenOptions::new();
        options.create(true).write(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(&path)
            .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
        let deadline = Instant::now() + wait;
        let mut told = false;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Self { _file: file }),
                Err(TryLockError::WouldBlock) => {}
                Err(TryLockError::Error(error)) => {
                    return Err(format!("cannot lock {}: {error}", path.display()));
                }
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "another thrum command is still using this device ({}); \
                     let it finish or stop it, then try again",
                    state_file.display()
                ));
            }
            if !told {
                eprintln!("waiting for another thrum command on this device to finish...");
                told = true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// The lock file lives next to the session state file.
fn lock_path(state_file: &Path) -> PathBuf {
    let mut name = state_file.as_os_str().to_os_string();
    name.push(".lock");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_holder_waits_and_then_gives_up() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state.json");
        let first = DeviceLock::acquire(&state, Duration::ZERO).expect("first");
        let started = Instant::now();
        let error = DeviceLock::acquire(&state, Duration::from_millis(300))
            .expect_err("the device is in use");
        assert!(started.elapsed() >= Duration::from_millis(300));
        assert!(error.contains("another thrum command"), "{error}");
        drop(first);
        DeviceLock::acquire(&state, Duration::ZERO).expect("free again");
    }

    #[test]
    fn a_waiting_holder_gets_the_lock_when_it_is_released() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state.json");
        let first = DeviceLock::acquire(&state, Duration::ZERO).expect("first");
        let path = state.clone();
        let waiter =
            std::thread::spawn(move || DeviceLock::acquire(&path, Duration::from_secs(10)));
        std::thread::sleep(Duration::from_millis(300));
        drop(first);
        waiter.join().unwrap().expect("acquired after release");
    }
}
