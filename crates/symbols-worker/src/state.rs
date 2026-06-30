//! The process-global symbolication state.
//!
//! A VGI worker is a single long-lived process that DuckDB talks to over Arrow
//! IPC, so the build-id-keyed debug-info cache lives here, in one
//! `Mutex<SymbolsState>`, resident for the life of the attach. This **is** the
//! moat: every `resolve` across every batch in the session shares it, so each
//! debug module is parsed exactly once. (The serializable manifest — exported /
//! imported through [`symbols_core::SymbolsState`] — additionally drives
//! cold-start across a process restart; the resident parsed modules are the
//! rebuildable RAM artifact and never cross the boundary.)

use std::sync::{Mutex, OnceLock};

use symbols_core::SymbolsState;

/// The single resident state shared by every function call in this worker.
fn global() -> &'static Mutex<SymbolsState> {
    static STATE: OnceLock<Mutex<SymbolsState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(SymbolsState::new()))
}

/// Run `f` with exclusive access to the shared state. Modules are owned and
/// accessed only under this lock, so resolution is serialized per process —
/// correct and simple; each attach has its own worker process, so contention is
/// minimal. A poisoned lock is recovered (the state is plain data + rebuildable
/// caches; a panic mid-resolve never leaves it inconsistent).
pub fn with_state<R>(f: impl FnOnce(&mut SymbolsState) -> R) -> R {
    let mut guard = global().lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}
