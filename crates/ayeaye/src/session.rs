//! Which agent session is running in a pane — the half that touches the world.
//!
//! Everything here opens a file, walks a directory, or asks tmux. What any of
//! it *means* is in `ayeaye_core::session`, where a test needs no machine: this
//! module is the reading, the walking and the asking, and nothing else.
//!
//! The order is the daemon's and it is load-bearing. The **process** is asked
//! first, and the pane's text only after. That is worth more than the
//! subprocess it saves: a statusline marker outlives the session that printed
//! it, so a pane whose agent has exited — or that was cleared and started again
//! — reads through the marker alone as an agent, and as the wrong one. A
//! process that is not there answers nothing.
//!
//! Two directory scans live here, and both are fixed-depth walks with a
//! prefix-and-suffix test at the bottom rather than a pattern match. That is a
//! decision on the ticket: a glob crate would be a dependency and a wider
//! surface for two questions that are each one `starts_with` and one
//! `ends_with`.
//!
//! `None` is always "could not find out", never an error: this answers inside a
//! request handler, and a pane whose session cannot be identified is an
//! ordinary state of the world.

use std::fs;
use std::path::{Path, PathBuf};

use axum::http::StatusCode;

use ayeaye_core::process::DEPTH;
use ayeaye_core::session::{Kind, Session, claude, codex, session_body};
use ayeaye_core::tmux::Pane;

use crate::config::Settings;
use crate::process::Processes;
use crate::tmux::Tmux;

/// The endpoint this module answers.
const SESSION: &str = "/api/session";

/// The two fields a pane is resolved from, asked for in one query.
///
/// One round trip rather than two, and not only for the cost: between two
/// queries the pane may have been closed and its id handed to another, so two
/// reads of "the same pane" can describe two different panes.
const FIELDS: &str = "#{pane_current_command}\t#{pane_pid}";

/// How much of a pane's history is searched for a statusline marker.
///
/// The daemon's two hundred lines. Claude paints the statusline into the live
/// screen and it reaches the scrollback only as later output pushes it off, so
/// the marker is somewhere above the visible screen when it is there at all.
const SCROLLBACK: &str = "-200";

/// How deep the codex rollout tree goes: `sessions/YYYY/MM/DD`.
const ROLLOUT_DEPTH: usize = 3;

/// How deep claude files its transcripts: `projects/<project>`.
const TRANSCRIPT_DEPTH: usize = 1;

/// What asking the pane's process came to.
///
/// Three answers rather than an `Option`, because "ask the pane's text now" is
/// not the same as "there is nothing here", and the difference is a tmux call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Behind {
    /// Not an agent, or an agent whose session could not be found at all.
    Nothing,
    /// The session this pane is running.
    Found(Session),
    /// A claude with no session file. Its statusline marker is the fallback,
    /// and only the pane's own text has it.
    AskThePane,
}

/// Where the two agents keep what they leave behind.
///
/// The three directories are fields rather than constants for the same reason
/// `process::linux::Linux`'s `/proc` root is: without that, a test either reads
/// whatever the person running it happens to have in their home directory, or
/// does not test this at all.
#[derive(Debug, Clone)]
pub struct Agents {
    /// `~/.claude/sessions`: one small file per live session, named for the
    /// process that owns it.
    claude_sessions: PathBuf,
    /// `~/.claude/projects`: a directory per project, transcripts inside.
    claude_projects: PathBuf,
    /// `~/.codex/sessions`: rollouts, filed by date.
    codex_sessions: PathBuf,
}

impl Default for Agents {
    fn default() -> Agents {
        Agents::here()
    }
}

impl Agents {
    /// The directories in this user's home.
    ///
    /// A home that cannot be read leaves the paths relative, which finds
    /// nothing rather than finding somebody else's: every pane resolves to no
    /// session, which is an answer the panel can render.
    pub fn here() -> Agents {
        Agents::under(std::env::var("HOME").unwrap_or_default())
    }

    /// The same three directories under a home of the caller's choosing.
    pub fn under(home: impl AsRef<Path>) -> Agents {
        let home = home.as_ref();
        Agents {
            claude_sessions: home.join(".claude").join("sessions"),
            claude_projects: home.join(".claude").join("projects"),
            codex_sessions: home.join(".codex").join("sessions"),
        }
    }

    /// The session running in a pane, if the pane is running an agent.
    ///
    /// `pane` is a [`Pane`] rather than an id, and that is the membership check
    /// rather than a syntactic one: a `Pane` can only have come from a
    /// `list-panes` this process just read, so the id below cannot be a target
    /// nobody offered.
    ///
    /// The backend is built and dropped inside the block that uses it, which is
    /// what keeps this future spawnable: `Box<dyn Processes>` is neither `Send`
    /// nor `Sync`, and holding one across an await would make the whole request
    /// handler unspawnable. Widening that trait to please this one caller would
    /// be the tail wagging the dog — the macOS backend is generic over an
    /// injectable tool, and several of the tools a test injects record what
    /// they were asked.
    pub async fn behind(&self, tmux: &Tmux, pane: &Pane) -> Option<Session> {
        let said = tmux
            .ask(&["display-message", "-p", "-t", pane.id.pane(), FIELDS])
            .await
            .ok()?;
        let next = {
            let processes = crate::process::here();
            self.behind_process(processes.as_ref(), &said)
        };
        match next {
            Behind::Found(session) => Some(session),
            Behind::Nothing => None,
            // A claude too old to keep a session file. The marker is what it
            // had instead, and it still resolves a pane the route above cannot
            // — but only for a pane that is running claude *now*, which is the
            // check the marker on its own has never been able to make and which
            // the route above has already made.
            Behind::AskThePane => {
                let text = tmux
                    .ask(&["capture-pane", "-p", "-t", pane.id.pane(), "-S", SCROLLBACK])
                    .await
                    .ok()?;
                self.from_marker(&text)
            }
        }
    }

    /// What the pane's *process* says, given the line tmux printed about it.
    ///
    /// `said` is the two [`FIELDS`], tab-separated, exactly as
    /// `display-message` printed them. Both come out of one query: between two
    /// queries the pane may have been closed and its id handed to another, so
    /// two reads of "the same pane" can describe two different panes.
    ///
    /// This is where a test drives the whole resolution, because it is the
    /// whole resolution: everything above it is two tmux calls.
    pub fn behind_process(&self, processes: &dyn Processes, said: &str) -> Behind {
        let Some((command, pane_pid)) = said.trim_end().split_once('\t') else {
            return Behind::Nothing;
        };
        let Ok(pane_pid) = pane_pid.trim().parse::<u32>() else {
            return Behind::Nothing;
        };
        match Kind::of(command.trim()) {
            None => Behind::Nothing,
            Some(Kind::Codex) => match self.codex(processes, pane_pid) {
                Some(session) => Behind::Found(session),
                None => Behind::Nothing,
            },
            Some(Kind::Claude) => match self.claude(processes, pane_pid) {
                Some(session) => Behind::Found(session),
                None => Behind::AskThePane,
            },
        }
    }

    /// The claude session behind a pane's shell, through claude's own session
    /// file.
    ///
    /// That file is keyed by pid, so the descendant walk the codex route
    /// already does is the whole lookup: no guessing from a working directory
    /// and an mtime, which is ambiguous the moment two agents share a repo.
    pub fn claude(&self, processes: &dyn Processes, pane_pid: u32) -> Option<Session> {
        let pid = processes.descendant(pane_pid, Kind::Claude.as_str(), DEPTH)?;
        let recorded = text(&self.claude_sessions.join(format!("{pid}.json")))?;
        let started = processes.start_time(pid)?;
        let id = claude::session_id(&recorded, started)?;
        Some(Session::new(Kind::Claude, &id, &self.transcript(&id)?))
    }

    /// The claude session a statusline printed into a pane.
    pub fn from_marker(&self, pane_text: &str) -> Option<Session> {
        let id = claude::marker(pane_text)?;
        Some(Session::new(Kind::Claude, &id, &self.transcript(&id)?))
    }

    /// The codex session behind a pane's shell.
    ///
    /// What the process **holds open** first, because that is a fact about the
    /// process rather than an inference about it — and because it is the only
    /// thing that resolves a *resumed* session, whose rollout was written days
    /// before the process that reopened it.
    pub fn codex(&self, processes: &dyn Processes, pane_pid: u32) -> Option<Session> {
        let pid = processes.descendant(pane_pid, Kind::Codex.as_str(), DEPTH)?;
        if let Some(session) = self.held(processes, pid) {
            return Some(session);
        }
        let started = processes.start_time(pid)?;
        // A second probe on macOS, so it is only asked for once the answer
        // above has failed and it can still help.
        let cwd = processes.cwd(pid)?;
        self.started_with(started, &cwd)
    }

    /// The rollout this codex process is holding open, if it is holding one of
    /// its own.
    fn held(&self, processes: &dyn Processes, pid: u32) -> Option<Session> {
        let root = self.codex_sessions.to_string_lossy().into_owned();
        let mut best: Option<(f64, String, String)> = None;
        for path in processes.open_files(pid) {
            if !codex::held(&path, &root) {
                continue;
            }
            let Some(meta) = first_line(Path::new(&path))
                .as_deref()
                .and_then(codex::meta)
            else {
                continue;
            };
            // Codex holds its subagents' rollouts open too; the session sitting
            // in the pane is the one that came from nobody.
            if meta.is_subagent() {
                continue;
            }
            let Some(written) = modified(Path::new(&path)) else {
                continue;
            };
            // More than one survivor is not expected. Deciding between them by
            // last write rather than by whichever descriptor came back first is
            // what keeps the answer the same on two machines; the path breaks a
            // tie, because two files written in the same second must not depend
            // on the order the kernel listed them in.
            if best
                .as_ref()
                .is_none_or(|(when, at, _)| (written, &path) > (*when, at))
            {
                best = Some((written, path, meta.id));
            }
        }
        let (_, path, id) = best?;
        Some(Session::new(Kind::Codex, &id, &path))
    }

    /// The rollout that appeared just after this process started, in the
    /// directory it is running in.
    ///
    /// The route for a codex too old to hold its rollout reliably. A rollout is
    /// created within a second or two of the process starting and its filename
    /// carries that moment; pairing that with the working directory keeps two
    /// codex agents in one directory distinct.
    fn started_with(&self, started: f64, cwd: &str) -> Option<Session> {
        let mut best: Option<(f64, PathBuf, String)> = None;
        for path in walk(&self.codex_sessions, ROLLOUT_DEPTH, &codex::rollout_name) {
            let Some(at) = name(&path).and_then(codex::stamp).and_then(instant) else {
                continue;
            };
            let Some(delta) = codex::delay(at, started) else {
                continue;
            };
            // Strictly closer, and the path breaks a tie: the walk's order is
            // directory order, which is arbitrary.
            if best
                .as_ref()
                .is_some_and(|(best, at, _)| (delta.abs(), &path) >= (best.abs(), at))
            {
                continue;
            }
            let Some(meta) = first_line(&path).as_deref().and_then(codex::meta) else {
                continue;
            };
            // The daemon filters subagents on the route above and not on this
            // one. Filtering on both is the fix rather than a parity break: a
            // subagent's rollout is written in its parent's working directory
            // moments after the parent starts, which is exactly the window and
            // exactly the directory this matches on.
            if meta.is_subagent() || meta.cwd.as_deref() != Some(cwd) {
                continue;
            }
            best = Some((delta, path, meta.id));
        }
        let (_, path, id) = best?;
        Some(Session::new(
            Kind::Codex,
            &id,
            &path.to_string_lossy().into_owned(),
        ))
    }

    /// The transcript file for one claude session id.
    ///
    /// Found by id rather than built from the working directory: how a project
    /// path becomes a directory name under `~/.claude/projects` is claude's
    /// business, and neither route into this has ever had to know it.
    ///
    /// The **first by name** where the daemon takes whichever entry its `glob`
    /// happened to return, so a session with more than one matching file
    /// resolves to the same one twice running.
    fn transcript(&self, id: &str) -> Option<String> {
        let mut found = walk(&self.claude_projects, TRANSCRIPT_DEPTH, &|name| {
            claude::transcript_name(name, id)
        });
        found.sort();
        Some(found.first()?.to_string_lossy().into_owned())
    }
}

/// Answer `/api/session`, or `None` if this is not it.
///
/// There is no route table here: everything under `/api/` reaches this through
/// the server's one handler, which has already applied the Host gate, the CSRF
/// gate and the token gate. An endpoint mounted on the router with `.route(…)`
/// would skip all three and nothing would say so.
pub async fn answer(
    settings: &Settings,
    path: &str,
    query: Option<&str>,
) -> Option<(StatusCode, String)> {
    (path == SESSION).then_some(())?;
    Some(session(settings, query).await)
}

/// Which session is in the pane the caller named.
///
/// A pane nobody offered is answered as "not an agent" rather than refused, and
/// that is the daemon's answer too: the panel asks this about whatever is
/// selected, and a pane that has just closed is an ordinary race rather than a
/// caller doing something wrong.
///
/// **The pane list is what makes the id safe**, and it is membership rather
/// than syntax that does it: the id is looked up in the panes this process just
/// read, and the [`Pane`] found is what goes to tmux. A pattern over the id
/// would not be a substitute.
async fn session(settings: &Settings, query: Option<&str>) -> (StatusCode, String) {
    let here = settings.peers.here().name();
    let asked = first(query, "pane").unwrap_or_default();
    let panes = settings.tmux.panes(here).await.unwrap_or_default();
    let Some(pane) = panes.iter().find(|pane| pane.id.qualified() == asked) else {
        return (StatusCode::OK, session_body(None));
    };
    let found = settings.agents.behind(&settings.tmux, pane).await;
    (StatusCode::OK, session_body(found.as_ref()))
}

/// The first non-empty value of a query parameter.
fn first(query: Option<&str>, name: &str) -> Option<String> {
    form_urlencoded::parse(query.unwrap_or("").as_bytes())
        .find(|(key, value)| key == name && !value.is_empty())
        .map(|(_, value)| value.into_owned())
}

/// Every file exactly `depth` directories below `root` whose name passes
/// `wanted`.
///
/// A fixed-depth walk rather than a pattern match, which is the decision on the
/// ticket: the daemon's `projects/*/<id>*.jsonl` and `sessions/*/*/*/rollout-*`
/// are each one `starts_with` and one `ends_with` once the wildcards are gone,
/// and a glob crate would be a dependency and a wider surface for that.
///
/// Nothing below the depth is opened and nothing above it is followed: a
/// directory that will not read costs itself, which is what keeps one
/// unreadable project from emptying the answer for every pane.
fn walk(root: &Path, depth: usize, wanted: &dyn Fn(&str) -> bool) -> Vec<PathBuf> {
    let mut here = vec![root.to_path_buf()];
    for _ in 0..depth {
        let mut next = Vec::new();
        for directory in here {
            let Ok(entries) = fs::read_dir(&directory) else {
                continue;
            };
            next.extend(
                entries
                    .flatten()
                    .filter_map(|entry| entry.file_type().ok()?.is_dir().then(|| entry.path())),
            );
        }
        here = next;
    }
    let mut found = Vec::new();
    for directory in here {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.file_name().to_str().is_some_and(wanted) {
                found.push(entry.path());
            }
        }
    }
    found
}

/// The last component of a path, as text.
fn name(path: &Path) -> Option<&str> {
    path.file_name()?.to_str()
}

/// Read a file as text, however it happens to be encoded, or `None`.
///
/// Lossily: no filesystem enforces valid UTF-8 in a path, and one odd byte in
/// one agent's file must cost that file rather than the whole board.
fn text(path: &Path) -> Option<String> {
    fs::read(path)
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

/// The first line of a file, without reading the rest of it.
///
/// A rollout grows to megabytes over a long session and only its opening line
/// says whose it is, so this reads a bounded prefix and stops at the first
/// terminator. A first line longer than that bound is not a `session_meta`.
fn first_line(path: &Path) -> Option<String> {
    use std::io::Read;

    let mut file = fs::File::open(path).ok()?;
    let mut head = vec![0; FIRST_LINE];
    let read = file.read(&mut head).ok()?;
    head.truncate(read);
    let text = String::from_utf8_lossy(&head).into_owned();
    match text.split_once('\n') {
        Some((line, _)) => Some(line.to_string()),
        // No terminator in the bound: either the file is shorter than one line,
        // which is a rollout being written right now, or its first line is
        // longer than any `session_meta` has ever been.
        None => (read < FIRST_LINE).then_some(text),
    }
}

/// How much of a file is read looking for its first line.
///
/// A real `session_meta` is a couple of kilobytes. This is generous enough that
/// a version which adds fields does not silently stop resolving, and small
/// enough that a directory of them costs nothing to scan.
const FIRST_LINE: usize = 64 * 1024;

/// When a file was last written, in seconds since the epoch.
fn modified(path: &Path) -> Option<f64> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|since| since.as_secs_f64())
}

/// A local wall-clock moment as an instant.
///
/// `mktime`, which is what the daemon's `time.mktime` is: codex writes a
/// rollout's stamp in the machine's local time, and only the C library knows
/// what that was on the day in question. `tm_isdst = -1` is what asks it to
/// work out whether summer time was in force, rather than being told.
fn instant(stamp: codex::Stamp) -> Option<f64> {
    // Zeroed rather than fielded one by one: `tm` has members beyond the seven
    // set here on every platform that has it, and a stack full of whatever was
    // there before is not an input to give a C function.
    let mut when: libc::tm = unsafe { std::mem::zeroed() };
    when.tm_year = stamp.year.checked_sub(1900)?;
    when.tm_mon = i32::try_from(stamp.month).ok()? - 1;
    when.tm_mday = i32::try_from(stamp.day).ok()?;
    when.tm_hour = i32::try_from(stamp.hour).ok()?;
    when.tm_min = i32::try_from(stamp.minute).ok()?;
    when.tm_sec = i32::try_from(stamp.second).ok()?;
    when.tm_isdst = -1;
    // The one call in this module that is not a file read. `mktime` reads the
    // timezone and writes back into `when`, and does not touch anything else.
    let seconds = unsafe { libc::mktime(&mut when) };
    (seconds != -1).then(|| seconds as f64)
}

#[cfg(test)]
mod tests {
    use super::{Agents, Behind, instant};
    use crate::process::Processes;
    use ayeaye_core::process::Source;
    use ayeaye_core::session::{Kind, codex};
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;

    /// A home of one's own.
    ///
    /// Every question this module answers is a file at a path derived from a
    /// home directory, so a home the test built is the whole of what it needs —
    /// and building one is the only way to test the answers without reading the
    /// agents of whoever is running the suite.
    struct Home(PathBuf);

    impl Home {
        fn new(what: &str) -> Home {
            let dir =
                std::env::temp_dir().join(format!("ayeaye-agents-{}-{what}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("a temporary home");
            Home(dir)
        }

        fn agents(&self) -> Agents {
            Agents::under(&self.0)
        }

        /// One `~/.claude/sessions/<pid>.json`, as claude writes it.
        fn claude_session(&self, pid: u32, id: &str, started: f64) -> &Home {
            let dir = self.0.join(".claude").join("sessions");
            fs::create_dir_all(&dir).expect("a sessions directory");
            fs::write(
                dir.join(format!("{pid}.json")),
                format!(
                    r#"{{"sessionId":"{id}","cwd":"/dev/thing","startedAt":{}}}"#,
                    (started * 1000.0) as u64
                ),
            )
            .expect("a session file");
            self
        }

        /// One transcript, in the project directory claude filed it under.
        fn transcript(&self, project: &str, name: &str) -> PathBuf {
            let dir = self.0.join(".claude").join("projects").join(project);
            fs::create_dir_all(&dir).expect("a project directory");
            let path = dir.join(name);
            fs::write(&path, "{}\n").expect("a transcript");
            path
        }

        /// One rollout, filed the way codex files them.
        fn rollout(&self, day: &str, name: &str, line: &str) -> PathBuf {
            let mut dir = self.0.join(".codex").join("sessions");
            for part in day.split('/') {
                dir = dir.join(part);
            }
            fs::create_dir_all(&dir).expect("a rollout directory");
            let path = dir.join(name);
            fs::write(&path, format!("{line}\n")).expect("a rollout");
            path
        }
    }

    impl Drop for Home {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A `session_meta` line for a rollout the test is writing.
    fn meta_line(id: &str, cwd: &str, parent: Option<&str>) -> String {
        let parent = match parent {
            Some(parent) => format!(r#","parent_thread_id":"{parent}","thread_source":"subagent""#),
            None => r#","thread_source":"cli""#.to_string(),
        };
        format!(r#"{{"type":"session_meta","payload":{{"id":"{id}","cwd":"{cwd}"{parent}}}}}"#)
    }

    /// A process tree somebody wrote down.
    ///
    /// The real backends are pinned against captured output in their own suite;
    /// what is being tested here is what this module does with their answers, so
    /// the answers are stated rather than read off a machine.
    #[derive(Default)]
    struct Fake {
        kids: BTreeMap<u32, Vec<u32>>,
        names: BTreeMap<u32, String>,
        started: BTreeMap<u32, f64>,
        cwds: BTreeMap<u32, String>,
        open: BTreeMap<u32, Vec<String>>,
    }

    impl Fake {
        /// A pane shell at 100 with one agent under it at 200.
        fn pane(name: &str) -> Fake {
            let mut fake = Fake::default();
            fake.kids.insert(100, vec![200]);
            fake.names.insert(100, "bash".to_string());
            fake.names.insert(200, name.to_string());
            fake
        }

        fn started(mut self, pid: u32, when: f64) -> Fake {
            self.started.insert(pid, when);
            self
        }

        fn cwd(mut self, pid: u32, where_: &str) -> Fake {
            self.cwds.insert(pid, where_.to_string());
            self
        }

        fn holding(mut self, pid: u32, paths: &[PathBuf]) -> Fake {
            self.open.insert(
                pid,
                paths
                    .iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect(),
            );
            self
        }
    }

    impl Source for Fake {
        fn children(&self, pid: u32) -> Vec<u32> {
            self.kids.get(&pid).cloned().unwrap_or_default()
        }

        fn comm(&self, pid: u32) -> Option<String> {
            self.names.get(&pid).cloned()
        }
    }

    impl Processes for Fake {
        fn start_time(&self, pid: u32) -> Option<f64> {
            self.started.get(&pid).copied()
        }

        fn cwd(&self, pid: u32) -> Option<String> {
            self.cwds.get(&pid).cloned()
        }

        fn open_files(&self, pid: u32) -> Vec<String> {
            self.open.get(&pid).cloned().unwrap_or_default()
        }

        fn exists(&self, pid: u32) -> bool {
            self.names.contains_key(&pid)
        }

        fn ssh_peer(&self, _pid: u32) -> Option<String> {
            None
        }
    }

    const CLAUDE_ID: &str = "0123abcd-dead-beef-0000-111122223333";
    const CODEX_ID: &str = "77770000-1111-2222-3333-444455556666";
    const STARTED: f64 = 1_772_614_802.0;

    // AYEAYE-45 — the whole claude route: the pane's shell in, the agent under
    // it, its session file, and the transcript that file names.
    #[test]
    fn a_claude_pane_resolves_through_its_session_file() {
        let home = Home::new("claude");
        home.claude_session(200, CLAUDE_ID, STARTED);
        let transcript = home.transcript("-dev-thing", &format!("{CLAUDE_ID}.jsonl"));
        let fake = Fake::pane("claude").started(200, STARTED);

        let found = home.agents().behind_process(&fake, "claude\t100\n");
        let Behind::Found(session) = found else {
            panic!("no session: {found:?}");
        };
        assert_eq!(session.kind, Kind::Claude);
        assert_eq!(session.id, "0123abcd");
        assert_eq!(session.path, transcript.to_string_lossy());
    }

    // AYEAYE-45 — a claude too old to write a session file is not "no session":
    // it is the one case where the pane's own text has to be asked, and that
    // costs a tmux call the other answers do not.
    #[test]
    fn a_claude_with_no_session_file_sends_us_to_the_pane() {
        let home = Home::new("marker");
        home.transcript("-dev-thing", &format!("{CLAUDE_ID}.jsonl"));
        let fake = Fake::pane("claude").started(200, STARTED);

        assert_eq!(
            home.agents().behind_process(&fake, "claude\t100"),
            Behind::AskThePane
        );

        let marker = format!("output\n{}cc:0123abcd{} ~/dev\n", '\u{27ea}', '\u{27eb}');
        let session = home
            .agents()
            .from_marker(&marker)
            .expect("the marker names a session that is on disk");
        assert_eq!(session.id, "0123abcd");
    }

    // AYEAYE-45 — a session file that outlived a crash names a pid the system
    // is free to hand out again. The next holder of that pid must not inherit a
    // stranger's conversation, and must not be reported as that stranger.
    #[test]
    fn a_session_file_about_another_process_is_not_this_one() {
        let home = Home::new("recycled");
        home.claude_session(200, CLAUDE_ID, STARTED - 86_400.0);
        home.transcript("-dev-thing", &format!("{CLAUDE_ID}.jsonl"));
        let fake = Fake::pane("claude").started(200, STARTED);

        assert_eq!(
            home.agents().behind_process(&fake, "claude\t100"),
            Behind::AskThePane,
            "the file was refused, so the marker is what is left to try"
        );
    }

    // AYEAYE-45 — the acceptance criterion: a *resumed* codex session. Its
    // rollout was written days before the process that reopened it, so no
    // window around a start time contains it and only the descriptor does.
    #[test]
    fn a_resumed_codex_session_resolves_through_what_it_holds_open() {
        let home = Home::new("resumed");
        let rollout = home.rollout(
            "2026/03/01",
            "rollout-2026-03-01T09-00-02-7777.jsonl",
            &meta_line(CODEX_ID, "/dev/thing", None),
        );
        // Started days after the rollout was written, and in a directory the
        // second route could not match on either.
        let fake = Fake::pane("codex")
            .started(200, STARTED)
            .cwd(200, "/dev/somewhere-else")
            .holding(200, &[rollout.clone()]);

        let found = home.agents().behind_process(&fake, "codex\t100");
        let Behind::Found(session) = found else {
            panic!("a resumed session must resolve: {found:?}");
        };
        assert_eq!(session.kind, Kind::Codex);
        assert_eq!(session.id, "77770000");
        assert_eq!(session.path, rollout.to_string_lossy());
    }

    // AYEAYE-45 — codex holds its subagents' rollouts open too, so the process
    // in the pane has several and only one of them is its own. A spawned thread
    // records the parent it came from; the session in the pane came from
    // nobody.
    #[test]
    fn a_subagents_rollout_is_not_the_session_in_the_pane() {
        let home = Home::new("subagent");
        let mine = home.rollout(
            "2026/03/01",
            "rollout-2026-03-01T09-00-02-7777.jsonl",
            &meta_line(CODEX_ID, "/dev/thing", None),
        );
        // Written later, so it wins on last-write and must still lose.
        let theirs = home.rollout(
            "2026/03/01",
            "rollout-2026-03-01T09-04-11-8888.jsonl",
            &meta_line("88880000-aaaa", "/dev/thing", Some(CODEX_ID)),
        );
        let fake = Fake::pane("codex")
            .started(200, STARTED)
            .holding(200, &[mine.clone(), theirs]);

        let found = home.agents().behind_process(&fake, "codex\t100");
        let Behind::Found(session) = found else {
            panic!("no session: {found:?}");
        };
        assert_eq!(session.path, mine.to_string_lossy());
    }

    // AYEAYE-45 — a descriptor is not a session. A `.jsonl` the agent happens
    // to be editing in somebody's repository is an ordinary open file.
    #[test]
    fn a_jsonl_outside_the_sessions_directory_is_not_a_session() {
        let home = Home::new("elsewhere");
        let stray = home.0.join("dev").join("thing").join("rollout-notes.jsonl");
        fs::create_dir_all(stray.parent().expect("a parent")).expect("a directory");
        fs::write(
            &stray,
            format!("{}\n", meta_line(CODEX_ID, "/dev/thing", None)),
        )
        .expect("a stray file");
        let fake = Fake::pane("codex")
            .started(200, STARTED)
            .cwd(200, "/dev/thing")
            .holding(200, &[stray]);

        assert_eq!(
            home.agents().behind_process(&fake, "codex\t100"),
            Behind::Nothing
        );
    }

    // AYEAYE-45 — the second route, for a codex too old to hold its rollout
    // reliably: the rollout that appeared just after the process started, in
    // the directory it is running in. The directory is what keeps two codex
    // agents started moments apart distinct.
    #[test]
    fn a_codex_holding_nothing_is_found_by_when_and_where_it_started() {
        let home = Home::new("timestamp");
        let started = instant(codex::Stamp {
            year: 2026,
            month: 3,
            day: 4,
            hour: 9,
            minute: 0,
            second: 0,
        })
        .expect("a local instant");

        let mine = home.rollout(
            "2026/03/04",
            "rollout-2026-03-04T09-00-02-7777.jsonl",
            &meta_line(CODEX_ID, "/dev/thing", None),
        );
        // Same moment, another directory: the pairing is what tells them apart.
        home.rollout(
            "2026/03/04",
            "rollout-2026-03-04T09-00-01-9999.jsonl",
            &meta_line("99990000-bbbb", "/dev/other", None),
        );
        // Right directory, yesterday: outside the window.
        home.rollout(
            "2026/03/03",
            "rollout-2026-03-03T09-00-02-6666.jsonl",
            &meta_line("66660000-cccc", "/dev/thing", None),
        );

        let fake = Fake::pane("codex")
            .started(200, started)
            .cwd(200, "/dev/thing");

        let found = home.agents().behind_process(&fake, "codex\t100");
        let Behind::Found(session) = found else {
            panic!("no session: {found:?}");
        };
        assert_eq!(session.id, "77770000");
        assert_eq!(session.path, mine.to_string_lossy());
    }

    // AYEAYE-45 — the daemon filters subagents on the held-open route and not
    // on this one, and a subagent's rollout is written in its parent's working
    // directory moments after the parent starts: exactly this window and
    // exactly this directory. Filtering on both is the fix, not a parity break.
    #[test]
    fn a_subagent_is_filtered_on_the_timestamp_route_too() {
        let home = Home::new("subagent-timestamp");
        let started = instant(codex::Stamp {
            year: 2026,
            month: 3,
            day: 4,
            hour: 9,
            minute: 0,
            second: 0,
        })
        .expect("a local instant");
        home.rollout(
            "2026/03/04",
            "rollout-2026-03-04T09-00-01-8888.jsonl",
            &meta_line("88880000-aaaa", "/dev/thing", Some(CODEX_ID)),
        );
        let mine = home.rollout(
            "2026/03/04",
            "rollout-2026-03-04T09-00-05-7777.jsonl",
            &meta_line(CODEX_ID, "/dev/thing", None),
        );

        let fake = Fake::pane("codex")
            .started(200, started)
            .cwd(200, "/dev/thing");
        let found = home.agents().behind_process(&fake, "codex\t100");
        let Behind::Found(session) = found else {
            panic!("no session: {found:?}");
        };
        assert_eq!(
            session.path,
            mine.to_string_lossy(),
            "the subagent's rollout is nearer the start time and is still not it"
        );
    }

    // AYEAYE-45 — a pane running something else resolves to nothing rather than
    // to a guess, and so does a line tmux did not print in the shape expected.
    #[test]
    fn a_pane_that_is_not_an_agent_resolves_to_nothing() {
        let home = Home::new("nothing");
        let fake = Fake::pane("codex").started(200, STARTED);
        for said in [
            "sh\t100",
            "vim\t100",
            "",
            "codex",
            "codex\tnot-a-pid",
            "\t100",
        ] {
            assert_eq!(
                home.agents().behind_process(&fake, said),
                Behind::Nothing,
                "resolved something out of {said:?}"
            );
        }
    }

    // AYEAYE-45 — an agent that started, wrote nothing, and has no transcript
    // yet. There is a session id and no file to open, which is not a session
    // anyone can be shown.
    #[test]
    fn a_session_with_no_transcript_yet_is_not_a_session() {
        let home = Home::new("unwritten");
        home.claude_session(200, CLAUDE_ID, STARTED);
        let fake = Fake::pane("claude").started(200, STARTED);
        assert_eq!(
            home.agents().behind_process(&fake, "claude\t100"),
            Behind::AskThePane
        );
    }

    // AYEAYE-45 — the daemon takes whichever entry its `glob` happened to
    // return, so a session with more than one matching file can resolve to a
    // different one on two runs of the same machine. Sorted, so it does not.
    #[test]
    fn a_session_with_two_matching_files_resolves_the_same_way_twice() {
        let home = Home::new("two-files");
        home.claude_session(200, CLAUDE_ID, STARTED);
        home.transcript("-dev-thing", &format!("{CLAUDE_ID}.jsonl"));
        // First by path, which is the whole rule: which of the two is "right"
        // is not knowable from here, and answering the same way twice is.
        let first = home.transcript("-dev-other", &format!("{CLAUDE_ID}-1.jsonl"));
        let fake = Fake::pane("claude").started(200, STARTED);

        for _ in 0..5 {
            let found = home.agents().behind_process(&fake, "claude\t100");
            let Behind::Found(session) = found else {
                panic!("no session: {found:?}");
            };
            assert_eq!(session.path, first.to_string_lossy());
        }
    }

    // AYEAYE-45 — the walk is fixed-depth on purpose. A rollout filed deeper
    // than codex files them is not one this looks at, and neither is a
    // transcript sitting loose at the top of the projects directory.
    #[test]
    fn the_walk_stops_at_the_depth_the_agents_file_at() {
        let home = Home::new("depth");
        home.claude_session(200, CLAUDE_ID, STARTED);
        let projects = home.0.join(".claude").join("projects");
        fs::create_dir_all(&projects).expect("a projects directory");
        fs::write(projects.join(format!("{CLAUDE_ID}.jsonl")), "{}\n").expect("a loose transcript");
        let fake = Fake::pane("claude").started(200, STARTED);
        assert_eq!(
            home.agents().behind_process(&fake, "claude\t100"),
            Behind::AskThePane,
            "a transcript at the wrong depth is not the session's"
        );
    }

    // AYEAYE-45 — a home that is not there resolves every pane to nothing, and
    // does it without an error: this answers inside a request handler.
    #[test]
    fn a_home_with_nothing_in_it_answers_rather_than_fails() {
        let agents = Agents::under("/nonexistent/home");
        let fake = Fake::pane("codex")
            .started(200, STARTED)
            .cwd(200, "/dev/thing");
        assert_eq!(agents.behind_process(&fake, "codex\t100"), Behind::Nothing);
    }

    // AYEAYE-45 — codex writes a rollout's stamp in local time, and only the C
    // library knows what that was on the day in question. Round-tripped rather
    // than compared against a constant, because a constant would be this
    // machine's timezone written into the suite.
    #[test]
    fn a_local_moment_becomes_an_instant() {
        let stamp = codex::Stamp {
            year: 2026,
            month: 3,
            day: 4,
            hour: 9,
            minute: 0,
            second: 2,
        };
        let at = instant(stamp).expect("a local instant");
        let back = instant(codex::Stamp { second: 3, ..stamp }).expect("a local instant");
        assert!(
            (back - at - 1.0).abs() < f64::EPSILON,
            "one second apart is one second apart: {at} then {back}"
        );
    }
}
