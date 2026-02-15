use std::sync::{Mutex, MutexGuard};

/// Lock a mutex and recover from poisoning by taking the inner guard.
///
/// Poisoning indicates another thread panicked while holding the lock. For this
/// project, it is generally preferable to keep the Web UI/service alive and
/// continue operating, while logging the issue for diagnosis.
pub(crate) fn lock_recover<'a, T>(m: &'a Mutex<T>, name: &'static str) -> MutexGuard<'a, T> {
    match m.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            eprintln!("[Lock] mutex poisoned: {}", name);
            poisoned.into_inner()
        }
    }
}

