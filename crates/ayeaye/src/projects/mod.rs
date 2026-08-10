//! The project picker's effects.
//!
//! Every decision the picker makes is in `ayeaye_core::projects`; what is
//! here is the walking, the clock, the store on disk and the one search in
//! flight. The split is the point: the ranking rungs and the bounds are
//! testable without a machine, and this crate is where the machine is.

pub mod walk;

/// A real directory tree, for the tests that need one.
///
/// The walk's rules — the skip list, a symlink not followed, `.git` as a file
/// — are claims about a filesystem, and the only way to test a claim about a
/// filesystem is against one. Everything else in this module is driven through
/// an in-memory lister instead.
#[cfg(test)]
pub(crate) struct TempTree {
    /// The root, which this owns and removes.
    pub path: std::path::PathBuf,
}

#[cfg(test)]
impl TempTree {
    /// A directory of this run's own, under the system temporary directory.
    pub fn named(label: &str) -> TempTree {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "ayeaye-50-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a temporary directory");
        TempTree { path }
    }

    /// Make a directory, and everything above it.
    pub fn dir(&self, relative: &str) {
        std::fs::create_dir_all(self.path.join(relative)).expect("a directory");
    }

    /// Write a file, making everything above it.
    pub fn file(&self, relative: &str, contents: &str) {
        let at = self.path.join(relative);
        if let Some(parent) = at.parent() {
            std::fs::create_dir_all(parent).expect("a parent");
        }
        std::fs::write(at, contents).expect("a file");
    }

    /// Point `link` at `target`, both relative to the root.
    pub fn link(&self, target: &str, link: &str) {
        std::os::unix::fs::symlink(self.path.join(target), self.path.join(link))
            .expect("a symlink");
    }
}

#[cfg(test)]
impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
