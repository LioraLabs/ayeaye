//! File references and previews — the half that touches the machine.
//!
//! Everything here asks tmux where a pane is working, asks git what that
//! repository tracks, or opens a file. What any of it *means* — which tracked
//! file a reference names, how much of it a preview may carry, what the bodies
//! look like — is `ayeaye_core::files`, where a test needs no machine.
//!
//! Two boundaries are load-bearing, and both are the daemon's:
//!
//! - **Only tracked files.** The resolve offers nothing `git ls-files` did not
//!   print, and the preview re-derives that list for itself rather than
//!   trusting that the path it was handed was once on it.
//! - **The walk refuses links.** The preview opens its file component by
//!   component with `O_NOFOLLOW` at every step, from a descriptor for the
//!   repository root. A symlink anywhere in the path — planted after the
//!   tracked list was read, or tracked itself — fails the open rather than
//!   being followed to wherever it points. The standard library has no
//!   `openat`, which is why this reaches for `libc`, the crate's one
//!   syscall dependency.

use std::fs::File;
use std::io::Read;
use std::os::fd::FromRawFd;
use std::time::Duration;

use axum::http::StatusCode;

use ayeaye_core::files::{self, Candidate, Kind, MAX_IMAGE_PREVIEW_BYTES, MAX_TEXT_SCAN_BYTES};
use ayeaye_core::json;
use ayeaye_core::peer::PaneId;

use crate::command;
use crate::config::Settings;
use crate::server::local_pane;

/// How long git gets to name the repository root.
const ROOT_LIMIT: Duration = Duration::from_secs(5);

/// How long git gets to list the tracked files. Twice the root's, as the
/// daemon allows: a monorepo's list is real output, a `--show-toplevel` is a
/// line.
const LIST_LIMIT: Duration = Duration::from_secs(10);

/// The most candidates a resolve answers with. The daemon's twenty, and
/// `share/app.html` slices to twenty on its side too.
const MAX_CANDIDATES: usize = 20;

/// What a preview answers with.
///
/// Its own type rather than a `Response` so this module names no HTTP body
/// machinery: the server turns it into one, and a test can match on it.
pub enum Preview {
    /// A JSON answer: text rows, the binary stub, or a refusal.
    Json(StatusCode, String),
    /// An image's own bytes, labelled and — for SVG, which can carry script —
    /// sandboxed by the server's headers.
    Bytes {
        content_type: &'static str,
        body: Vec<u8>,
        /// Whether the server must add the sandboxing `Content-Security-Policy`.
        svg: bool,
    },
}

/// Answer `/api/files/resolve`: the tracked files a reference could mean.
///
/// Almost every way this can fail is an *empty answer* rather than a refusal,
/// and that is the daemon's shape: a pane that has just closed, a directory
/// that is not a repository, a git that would not answer — none of them is the
/// caller's fault, and the page renders all of them as "no tracked file
/// found".
pub async fn resolve(settings: &Settings, asked: &json::Value) -> (StatusCode, String) {
    let pane = text_of(asked, "pane");
    let reference = text_of(asked, "reference");
    if pane.is_empty() || reference.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            r#"{"error":"pane and reference required"}"#.to_string(),
        );
    }
    let (wanted, line) = files::reference(reference);
    let empty = || (StatusCode::OK, files::resolve_body(&[], line));
    if wanted.is_empty() {
        return empty();
    }
    let Some(id) = local_pane(settings, pane).await else {
        return empty();
    };
    let Some(cwd) = working_directory(settings, &id).await else {
        return empty();
    };
    let Some(root) = repo_root(&cwd).await else {
        return empty();
    };
    let Some(tracked) = tracked_files(&cwd).await else {
        return empty();
    };
    let pane_dir = files::relative(&root, &cwd);
    let ranked: Vec<String> =
        files::candidates(&wanted, &pane_dir, tracked.iter().map(String::as_str))
            .into_iter()
            .map(str::to_string)
            .collect();
    // The stats are one syscall each on a local path, but the path is only
    // *usually* local: under a hung network mount a blocking stat would take
    // a runtime worker with it, which is the same reason `agent::spawn` stats
    // on the blocking pool.
    let candidates = tokio::task::spawn_blocking(move || sized(&root, &ranked))
        .await
        .unwrap_or_default();
    (StatusCode::OK, files::resolve_body(&candidates, line))
}

/// The ranked candidates that are really on disk, sized, capped.
///
/// The cap lives here rather than in the ranking because a candidate that
/// fails its stat drops out — the daemon takes twenty *stattable* files, not
/// the first twenty ranked.
fn sized(root: &str, ranked: &[String]) -> Vec<Candidate> {
    let mut sized = Vec::new();
    for path in ranked {
        // `symlink_metadata`, matching the daemon's `follow_symlinks=False`:
        // the size offered is the size of the thing the preview would open,
        // and for a symlink that open is going to be refused anyway.
        let Ok(meta) = std::fs::symlink_metadata(format!("{root}/{path}")) else {
            continue;
        };
        sized.push(Candidate {
            path: path.clone(),
            size: meta.len(),
        });
        if sized.len() == MAX_CANDIDATES {
            break;
        }
    }
    sized
}

/// Answer `/api/files/preview`: one tracked file, freshly revalidated.
///
/// Unlike the resolve, failures here are *refusals*: the caller named one
/// exact file, so "it is not there" is an answer about that file rather than
/// an empty search. Every answer is deliberately one of two — `bad path` for
/// a request that could never name a tracked file, `not found` for everything
/// else — so what exists outside the tracked list cannot be probed through
/// the error text.
pub async fn preview(settings: &Settings, pane: &str, path: &str, line: &str) -> Preview {
    if pane.is_empty() || path.is_empty() {
        return refuse(StatusCode::BAD_REQUEST, "pane and path required");
    }
    let Ok(line) = files::line_param(line) else {
        return refuse(StatusCode::BAD_REQUEST, "line must be positive");
    };
    if !files::preview_path_ok(path) {
        return refuse(StatusCode::BAD_REQUEST, "bad path");
    }
    let Some(id) = local_pane(settings, pane).await else {
        return refuse(StatusCode::NOT_FOUND, "not found");
    };
    let Some(cwd) = working_directory(settings, &id).await else {
        return refuse(StatusCode::NOT_FOUND, "not found");
    };
    let Some(root) = repo_root(&cwd).await else {
        return refuse(StatusCode::NOT_FOUND, "not found");
    };
    // Re-derived, never trusted: the path arrived from a client, and the only
    // thing that makes it previewable is being on the list *right now*.
    let Some(tracked) = tracked_files(&cwd).await else {
        return refuse(StatusCode::NOT_FOUND, "not found");
    };
    if !tracked.iter().any(|had| had == path) {
        return refuse(StatusCode::NOT_FOUND, "not found");
    }

    let owned_path = path.to_string();
    // The walk and the read together on the blocking pool: between them they
    // are an open per component and up to eight megabytes off a disk that is
    // only usually a disk.
    tokio::task::spawn_blocking(move || read_preview(&root, &owned_path, line))
        .await
        .unwrap_or_else(|_| refuse(StatusCode::NOT_FOUND, "not found"))
}

/// Open one tracked file the safe way and read what the preview carries.
fn read_preview(root: &str, path: &str, line: Option<usize>) -> Preview {
    let Some(mut file) = open_tracked(root, path) else {
        return refuse(StatusCode::NOT_FOUND, "not found");
    };
    let Ok(meta) = file.metadata() else {
        return refuse(StatusCode::NOT_FOUND, "not found");
    };
    let size = meta.len();
    match Kind::of(path) {
        kind @ (Kind::Image | Kind::Svg) => {
            if size > MAX_IMAGE_PREVIEW_BYTES {
                return refuse(StatusCode::PAYLOAD_TOO_LARGE, "image too large");
            }
            // One byte past the limit, so a file that grew between the fstat
            // and the read is caught by what actually arrived.
            let Some(body) = read_up_to(&mut file, MAX_IMAGE_PREVIEW_BYTES as usize + 1) else {
                return refuse(StatusCode::NOT_FOUND, "not found");
            };
            if body.len() as u64 > MAX_IMAGE_PREVIEW_BYTES {
                return refuse(StatusCode::PAYLOAD_TOO_LARGE, "image too large");
            }
            let Some(content_type) = files::mime(path) else {
                // Unreachable while `Kind::of` and `mime` share a table.
                return refuse(StatusCode::NOT_FOUND, "not found");
            };
            Preview::Bytes {
                content_type,
                body,
                svg: kind == Kind::Svg,
            }
        }
        Kind::Binary => Preview::Json(StatusCode::OK, files::binary_body(path, size)),
        Kind::Text => {
            let Some(bytes) = read_up_to(&mut file, MAX_TEXT_SCAN_BYTES) else {
                return refuse(StatusCode::NOT_FOUND, "not found");
            };
            let rows = files::excerpt(&bytes, size, line);
            Preview::Json(StatusCode::OK, files::text_body(path, size, &rows, line))
        }
    }
}

/// Open `root/path` without following a single link.
///
/// This is the security control on the preview endpoint, and it is a walk
/// rather than one `open` because the refusal has to hold for every component:
/// `O_NOFOLLOW` on a whole-path open only guards the last one. Each directory
/// is opened *at* the previous descriptor, so no component is ever re-resolved
/// from the root — a directory swapped for a link mid-walk fails the next
/// `openat` instead of redirecting it.
///
/// `None` for anything that is not a plain regular file at the end of plain
/// directories: a link anywhere, a missing file, a FIFO, a directory.
fn open_tracked(root: &str, path: &str) -> Option<File> {
    let mut components = path.split('/').peekable();
    let mut here = open_dir_at(None, root)?;
    while let Some(component) = components.next() {
        if components.peek().is_some() {
            here = open_dir_at(Some(&here), component)?;
        } else {
            let file = open_at(&here, component, libc::O_RDONLY)?;
            // Only a regular file is a file here — the daemon's `S_ISREG`
            // check. It refuses a device and a directory; a FIFO would park
            // this thread at the *open*, before any check runs, which is the
            // daemon's exposure too and needs write access to the repository
            // to arrange — git cannot track a FIFO.
            return file.metadata().ok()?.is_file().then_some(file);
        }
    }
    // An empty path has no components to land on; `preview_path_ok` already
    // refused it, but this function must not rely on its callers.
    None
}

/// One directory, opened at a parent descriptor — or absolutely for the root.
fn open_dir_at(parent: Option<&File>, name: &str) -> Option<File> {
    open_impl(parent, name, libc::O_RDONLY | libc::O_DIRECTORY)
}

/// One file at a parent descriptor, never through a link.
fn open_at(parent: &File, name: &str, flags: libc::c_int) -> Option<File> {
    open_impl(Some(parent), name, flags)
}

/// The one place a descriptor is made.
///
/// `O_NOFOLLOW` on every open below the root, `O_CLOEXEC` on all of them so a
/// spawned agent never inherits a descriptor into somebody's repository. The
/// root itself may be reached through links — it is the answer git gave for
/// the pane's own directory, not client input.
fn open_impl(parent: Option<&File>, name: &str, flags: libc::c_int) -> Option<File> {
    use std::os::fd::AsRawFd;

    let name = std::ffi::CString::new(name).ok()?;
    let flags = flags
        | libc::O_CLOEXEC
        | match parent {
            Some(_) => libc::O_NOFOLLOW,
            None => 0,
        };
    let fd = match parent {
        // SAFETY: the pointer is a live CString's, the flags are constants,
        // and the descriptor is checked and immediately owned below.
        Some(parent) => unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) },
        None => unsafe { libc::open(name.as_ptr(), flags) },
    };
    if fd < 0 {
        return None;
    }
    // SAFETY: `fd` is a descriptor this process just opened and nothing else
    // owns; `File` takes over closing it, on every path out of this module.
    Some(unsafe { File::from_raw_fd(fd) })
}

/// Read at most `limit` bytes, or `None` if the file would not read.
fn read_up_to(file: &mut File, limit: usize) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    file.take(limit as u64).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// Where one pane is working right now, as tmux reports it.
async fn working_directory(settings: &Settings, id: &PaneId) -> Option<String> {
    let said = settings
        .tmux
        .ask(&[
            "display-message",
            "-p",
            "-t",
            id.pane(),
            "#{pane_current_path}",
        ])
        .await
        .ok()?;
    let cwd = said.trim();
    (!cwd.is_empty()).then(|| cwd.to_string())
}

/// The root of the repository the pane is working in, or `None` for a pane
/// that is not in one.
async fn repo_root(cwd: &str) -> Option<String> {
    let ran = command::run(
        &["git", "-C", cwd, "rev-parse", "--show-toplevel"],
        ROOT_LIMIT,
    )
    .await
    .ok()?;
    if !ran.ok {
        return None;
    }
    let root = ran.stdout.trim();
    (!root.is_empty()).then(|| root.to_string())
}

/// Every path the repository tracks, exactly as git prints them.
///
/// `-z` so a filename with a newline in it arrives as itself; the split is on
/// NUL, which no path can contain. `--full-name` with `:/` asks from the
/// root, wherever in the repository the pane sits.
async fn tracked_files(cwd: &str) -> Option<Vec<String>> {
    let ran = command::run(
        &[
            "git",
            "-C",
            cwd,
            "ls-files",
            "--full-name",
            "-z",
            "--",
            ":/",
        ],
        LIST_LIMIT,
    )
    .await
    .ok()?;
    if !ran.ok {
        return None;
    }
    Some(
        ran.stdout
            .split('\0')
            .filter(|path| !path.is_empty())
            .map(str::to_string)
            .collect(),
    )
}

/// One refusal, in the JSON shape every `/api/` error takes.
fn refuse(status: StatusCode, why: &str) -> Preview {
    Preview::Json(status, format!("{{\"error\":{}}}", json::string(why)))
}

/// One member of a request body, as the string it has to be.
fn text_of<'a>(body: &'a json::Value, name: &str) -> &'a str {
    body.get(name).and_then(json::Value::text).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::open_tracked;
    use std::fs;
    use std::io::Read;
    use std::path::PathBuf;

    /// A repository-shaped tree of the test's own.
    struct Tree(PathBuf);

    impl Tree {
        fn new(what: &str) -> Tree {
            let dir =
                std::env::temp_dir().join(format!("ayeaye-files-{}-{what}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("a temporary tree");
            Tree(dir)
        }

        fn root(&self) -> &str {
            self.0.to_str().expect("a UTF-8 temp dir")
        }

        fn file(&self, path: &str, content: &str) -> &Tree {
            let full = self.0.join(path);
            fs::create_dir_all(full.parent().expect("a parent")).expect("directories");
            fs::write(full, content).expect("a file");
            self
        }

        fn link(&self, path: &str, target: &str) -> &Tree {
            let full = self.0.join(path);
            fs::create_dir_all(full.parent().expect("a parent")).expect("directories");
            std::os::unix::fs::symlink(target, full).expect("a symlink");
            self
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    // AYEAYE-52 — the ordinary case the control must not break: a plain file
    // at the end of plain directories opens, and reads as itself.
    #[test]
    fn a_plain_tracked_file_opens_and_reads() {
        let tree = Tree::new("plain");
        tree.file("src/main.rs", "fn main() {}\n");
        let mut file = open_tracked(tree.root(), "src/main.rs").expect("a plain file opens");
        let mut read = String::new();
        file.read_to_string(&mut read).expect("it reads");
        assert_eq!(read, "fn main() {}\n");
    }

    // AYEAYE-52 — the spec's security decision: the walk refuses to follow
    // links. A symlink *is* what a tracked entry can be, so this is not a
    // theoretical input — `git ls-files` lists them happily.
    #[test]
    fn a_symlinked_file_is_refused() {
        let tree = Tree::new("leaf-link");
        tree.file("real.txt", "the real bytes\n")
            .link("alias.txt", "real.txt");
        assert!(
            open_tracked(tree.root(), "alias.txt").is_none(),
            "a link at the leaf must not be followed"
        );
        // And an absolute one pointing out of the tree entirely.
        let outside = Tree::new("leaf-link-outside");
        outside.file("secret.txt", "not yours\n");
        tree.link("out.txt", &format!("{}/secret.txt", outside.root()));
        assert!(open_tracked(tree.root(), "out.txt").is_none());
    }

    // AYEAYE-52 — `O_NOFOLLOW` on one open guards only the last component;
    // the reason this is a walk is that the refusal holds in the middle too.
    #[test]
    fn a_symlinked_directory_on_the_way_is_refused() {
        let tree = Tree::new("mid-link");
        tree.file("real/notes.md", "notes\n").link("alias", "real");
        assert!(
            open_tracked(tree.root(), "real/notes.md").is_some(),
            "the straight path still opens"
        );
        assert!(
            open_tracked(tree.root(), "alias/notes.md").is_none(),
            "a link in the middle must not be followed"
        );
    }

    // AYEAYE-52 — only a regular file is a file: a directory is not a
    // preview, and neither is anything else `S_ISREG` refuses.
    #[test]
    fn a_directory_is_not_a_previewable_file() {
        let tree = Tree::new("directory");
        tree.file("src/lib.rs", "");
        assert!(open_tracked(tree.root(), "src").is_none());
        assert!(open_tracked(tree.root(), "missing.txt").is_none());
        assert!(open_tracked(tree.root(), "").is_none());
    }
}
