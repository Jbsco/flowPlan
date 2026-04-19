//! Shader hot-reload (dev-only; feature `shader-hot-reload`).
//!
//! Watches the render crate's `src/` directory for changes and exposes a `take_pending`
//! flag that `Renderer::maybe_reload_shaders` polls each frame. On reload, shader source
//! is re-read from disk and the affected pipelines are rebuilt. This is **strictly dev**:
//! `env!("CARGO_MANIFEST_DIR")` resolves at compile time, so a shipping binary with this
//! feature enabled would look for sources at the build machine's paths. Off by default
//! per [`CODING_RULES.md`](../../../../CODING_RULES.md#dependencies).

#![allow(clippy::module_name_repetitions)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher as _};

/// Filesystem watcher that flips a pending flag when anything in the shader directory
/// changes. The recommended backend is chosen by `notify` at runtime (inotify on Linux,
/// `FSEvents` on macOS, etc.).
pub struct Watcher {
    pending: Arc<AtomicBool>,
    // Keeps the watcher thread alive; dropping stops watching.
    _watcher: RecommendedWatcher,
}

impl Watcher {
    /// Spawn a watcher on the crate's `src/` directory.
    pub fn new() -> std::io::Result<Self> {
        let pending = Arc::new(AtomicBool::new(false));
        let pending_cb = Arc::clone(&pending);
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            // Any event-ok counts as a pending reload. Errors are ignored -- the next
            // real event will retry; spurious errors should not wedge the flag.
            if res.is_ok() {
                pending_cb.store(true, Ordering::Release);
            }
        })
        .map_err(std::io::Error::other)?;

        watcher
            .watch(Path::new(shader_dir()), RecursiveMode::NonRecursive)
            .map_err(std::io::Error::other)?;

        Ok(Self {
            pending,
            _watcher: watcher,
        })
    }

    /// Returns `true` exactly once after a watcher event, then resets.
    pub fn take_pending(&self) -> bool {
        self.pending.swap(false, Ordering::AcqRel)
    }
}

/// The directory watched for shader changes. Set at compile time.
#[must_use]
pub const fn shader_dir() -> &'static str {
    concat!(env!("CARGO_MANIFEST_DIR"), "/src")
}

/// Read a shader source file from the watched directory.
pub fn load_source(name: &str) -> std::io::Result<String> {
    let mut path = PathBuf::from(shader_dir());
    path.push(name);
    std::fs::read_to_string(path)
}
