//! Session matching, against a real tmux and a real process.
//!
//! Everything below the endpoint is unit-tested against a directory the suite
//! built and a process tree it wrote down. This is the one test that proves the
//! chain: a pane id, `display-message`, the descendant walk, the kernel's list
//! of what that process has open, and the rollout's own first line.
//!
//! **The processes here are ones the suite started.** They run under a tmux
//! server of the suite's own — see `common` for why the default socket is off
//! limits — and they end when that server is killed on the way out. Nothing
//! here signals anything it did not start, and nothing reads a process it did
//! not start either.

// The harness is shared with the other test binaries and each of them uses a
// different part of it; the `#![allow(dead_code)]` for that lives inside
// `common` itself, so it is stated once rather than once per binary.
mod common;

use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ayeaye::session::Agents;
use ayeaye_core::peer::HostName;
use ayeaye_core::session::Kind;

use common::Private;

/// How long the agent gets to be running before the test gives up on it.
const PATIENCE: Duration = Duration::from_secs(10);

const CODEX_ID: &str = "77770000-1111-2222-3333-444455556666";

/// Where and what the scene's one rollout is called. Spelled once: the scratch
/// pane opens the same file `holding_codex` wrote, and two spellings that drift
/// mean a pane that dies rather than a pane that resolves.
const ROLLOUT_DAY: &str = "2026/03/01";
const ROLLOUT_NAME: &str = "rollout-2026-03-01T09-00-02-7777.jsonl";

fn host() -> HostName {
    HostName::new("desktop").expect("a host name")
}

/// A home and a `codex` to run, both of the suite's own.
struct Scene {
    home: PathBuf,
}

impl Scene {
    fn new(what: &str) -> Scene {
        let home = std::env::temp_dir().join(format!("ayeaye-45-{}-{what}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(&home).expect("a temporary home");
        Scene { home }
    }

    /// Something the kernel will call `codex`.
    ///
    /// A symlink to the shell: `comm` is the last component of the path that
    /// was exec'd, so this really is a process named `codex` as far as every
    /// backend is concerned — and it is a shell, so it can be told to hold a
    /// file open and then wait.
    fn agent(&self) -> PathBuf {
        let path = self.home.join("codex");
        symlink("/bin/sh", &path).expect("a codex of our own");
        path
    }

    /// The same agent, in a session `list-panes` deliberately drops.
    ///
    /// A leading `_` is somebody's floating scratch pane, and the panel must
    /// never offer one as a target. Everything else about it is a resolvable
    /// pane, which is what makes it the only honest test of membership.
    async fn scratch_codex(&self, server: &Private) -> String {
        let script = format!(
            "set -m; {} -c 'exec 9< {}; read line'; :",
            self.home.join("codex").display(),
            self.rollout_path().display()
        );
        server.tmux(&[
            "new-session",
            "-d",
            "-s",
            "_scratch",
            "/bin/sh",
            "-c",
            &script,
        ]);
        assert!(
            until_running_in(server, "_scratch", "codex").await,
            "the scratch pane never came up running codex"
        );
        server
            .layer()
            .ask(&["display-message", "-p", "-t", "_scratch", "#{pane_id}"])
            .await
            .expect("a running tmux answers")
            .trim()
            .to_string()
    }

    /// Settings pointed at this scene's tmux and home.
    fn settings(&self, server: &Private) -> ayeaye::config::Settings {
        settings(server.layer(), Agents::under(&self.home))
    }

    /// Where `holding_codex` files its rollout.
    fn rollout_path(&self) -> PathBuf {
        let mut path = self.home.join(".codex").join("sessions");
        for part in ROLLOUT_DAY.split('/') {
            path = path.join(part);
        }
        path.join(ROLLOUT_NAME)
    }

    /// One rollout, filed the way codex files them.
    fn rollout(&self, day: &str, name: &str, line: &str) -> PathBuf {
        let mut dir = self.home.join(".codex").join("sessions");
        for part in day.split('/') {
            dir = dir.join(part);
        }
        fs::create_dir_all(&dir).expect("a rollout directory");
        let path = dir.join(name);
        fs::write(&path, format!("{line}\n")).expect("a rollout");
        path
    }

    /// A pane running something called `codex`, holding a rollout open.
    ///
    /// A shell above it, because that is what tmux hands out: `pane_pid` is
    /// never the agent, and a walk that started at the pane's own pid would
    /// never have needed writing. The trailing `:` stops the shell exec'ing
    /// into its last command and leaving no shell at all, and `set -m` is what
    /// gives the agent its own process group — without it the child shares the
    /// wrapper's, and tmux reports the wrapper as what the pane is running.
    async fn holding_codex(&self, server: &Private) -> PathBuf {
        let agent = self.agent();
        let rollout = self.rollout(
            ROLLOUT_DAY,
            ROLLOUT_NAME,
            &format!(
                r#"{{"type":"session_meta","payload":{{"id":"{CODEX_ID}","cwd":"/dev/thing","thread_source":"cli"}}}}"#
            ),
        );
        let script = format!(
            "set -m; {} -c 'exec 9< {}; read line'; :",
            agent.display(),
            rollout.display()
        );
        server.tmux(&[
            "new-window",
            "-t",
            "work",
            "-n",
            "agent",
            "-d",
            "/bin/sh",
            "-c",
            &script,
        ]);
        assert!(
            until_running(server, "codex").await,
            "the pane never came up running codex"
        );
        rollout
    }
}

impl Drop for Scene {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.home);
    }
}

/// Settings pointed wherever the caller says.
///
/// Resolved and then adjusted rather than written out field by field, so a
/// field somebody adds to `Settings` next week does not land here.
fn settings(tmux: ayeaye::tmux::Tmux, agents: Agents) -> ayeaye::config::Settings {
    let mut settings = ayeaye::config::Settings::resolve(
        &[],
        |_| None,
        "test-token-not-a-real-secret".to_string(),
        Some("desktop".to_string()),
        ayeaye::cliban::Cliban::new("/nonexistent/cliban".to_string()),
        std::sync::Arc::new(ayeaye::dictate::Voice::new(
            std::path::PathBuf::from("/nonexistent/store"),
            ayeaye_core::model::settings::ModelSettings::resolve(|_| None, "")
                .expect("the defaults resolve"),
            ayeaye_core::cleanup::Policy::default(),
            "ayeaye-45-no-such-converter".to_string(),
            ayeaye_infer::backend::select(),
        )),
    )
    .expect("settings a test can drive");
    settings.tmux = tmux;
    settings.agents = agents;
    settings
}

/// Wait until tmux says the pane is running `command`.
///
/// Polled rather than slept through: the shell tmux started has to read its
/// argument and fork before any of this is true, and how long that takes is a
/// property of the machine running the suite.
async fn until_running(server: &Private, command: &str) -> bool {
    until_running_in(server, "agent", command).await
}

/// The same, for a target other than the window this file usually makes.
async fn until_running_in(server: &Private, target: &str, command: &str) -> bool {
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        let said = server
            .layer()
            .ask(&[
                "display-message",
                "-p",
                "-t",
                target,
                "#{pane_current_command}",
            ])
            .await
            .unwrap_or_default();
        if said.trim() == command {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

// AYEAYE-45 — the whole chain, and the acceptance criterion that needs it: a
// *resumed* codex session, whose rollout was written long before the process
// that reopened it. No window around a start time contains that, so the only
// thing that can resolve it is the descriptor the process is holding — which
// means a real process really holding a real file.
#[tokio::test]
async fn a_pane_resolves_to_the_session_its_process_is_holding_open() {
    let Some(server) = Private::named("session") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let scene = Scene::new("held");
    let rollout = scene.holding_codex(&server).await;

    let panes = server
        .layer()
        .panes(&host())
        .await
        .expect("a running tmux answers");
    let pane = panes
        .iter()
        .find(|pane| pane.name == "agent")
        .expect("the window the test made");

    let session = Agents::under(&scene.home)
        .behind(&server.layer(), pane)
        .await
        .expect("the pane is holding a rollout open");

    assert_eq!(session.kind, Kind::Codex);
    assert_eq!(session.id, "77770000");
    assert_eq!(session.path, rollout.to_string_lossy());
}

// AYEAYE-45 — a pane running a shell is not an agent, and asking about one
// costs a `display-message` and nothing else. This is the common case: most
// panes on most machines are shells.
#[tokio::test]
async fn a_pane_running_a_shell_resolves_to_no_session() {
    let Some(server) = Private::named("shell") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let scene = Scene::new("shell");

    let panes = server
        .layer()
        .panes(&host())
        .await
        .expect("a running tmux answers");
    let pane = panes.first().expect("the session the harness made");

    assert_eq!(
        Agents::under(&scene.home)
            .behind(&server.layer(), pane)
            .await,
        None
    );
}

// AYEAYE-45 — what the ticket delivers, through the endpoint the panel calls:
// a pane id in, the session running in it out. The settings are *resolved* and
// then pointed at this test's tmux and home rather than written out field by
// field, so a field somebody adds to `Settings` next week does not land here.
#[tokio::test]
async fn the_endpoint_names_the_session_running_in_a_pane() {
    let Some(server) = Private::named("endpoint") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let scene = Scene::new("endpoint");
    let rollout = scene.holding_codex(&server).await;

    let settings = scene.settings(&server);

    let panes = settings
        .tmux
        .panes(&host())
        .await
        .expect("a running tmux answers");
    let pane = panes
        .iter()
        .find(|pane| pane.name == "agent")
        .expect("the window the test made");

    let asked = format!("pane={}", pane.id.qualified().replace('%', "%25"));
    let (status, body) = ayeaye::session::answer(&settings, "/api/session", Some(&asked))
        .await
        .expect("the session endpoint owns this path");

    assert_eq!(status, 200);
    assert_eq!(body, r#"{"kind":"codex","id":"77770000"}"#);
    // And the rollout it named is the one on disk, which is what the transcript
    // view will go on to open.
    assert!(rollout.exists());
}

// AYEAYE-45 — nothing else in `/api/` belongs to this module, and saying so is
// what keeps the server's 404 reachable for a path nobody has written yet.
#[tokio::test]
async fn the_module_owns_one_path_and_no_other() {
    // No tmux and no home: this is a question about paths, and it must answer
    // it on a machine that has neither.
    let settings = settings(
        common::nowhere("session-paths"),
        Agents::under("/nonexistent/home"),
    );
    for path in ["/api/sessions", "/api/session/", "/api/panes", "/", ""] {
        assert!(
            ayeaye::session::answer(&settings, path, None)
                .await
                .is_none(),
            "{path} is not this module's"
        );
    }
}

// AYEAYE-45 — **membership, not syntax**, and this is the test that can tell
// the difference. The scratch pane below is real, is running the same agent,
// and is holding the same rollout open — everything the resolution needs. The
// only reason it must not resolve is that `list-panes` deliberately drops a
// session whose name starts with `_`, and the endpoint answers out of that
// list. Delete the membership check and this pane starts resolving; no test
// that asks about an id nobody ever offered can say that.
#[tokio::test]
async fn a_pane_the_list_excludes_is_never_a_target() {
    let Some(server) = Private::named("membership") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let scene = Scene::new("membership");
    scene.holding_codex(&server).await;
    let scratch = scene.scratch_codex(&server).await;

    let settings = scene.settings(&server);
    let panes = settings
        .tmux
        .panes(&host())
        .await
        .expect("a running tmux answers");

    // The pane in `work` is offered, and resolves.
    let offered = panes
        .iter()
        .find(|pane| pane.name == "agent")
        .expect("the window the test made");
    let (_, body) = ayeaye::session::answer(
        &settings,
        "/api/session",
        Some(&format!("pane=desktop/{}", pane_query(offered.id.pane()))),
    )
    .await
    .expect("the session endpoint owns this path");
    assert_eq!(body, r#"{"kind":"codex","id":"77770000"}"#);

    // The pane in `_scratch` is not offered, and must not resolve — even though
    // it is running the same agent, holding the same kind of file open.
    assert!(
        !panes.iter().any(|pane| pane.id.pane() == scratch),
        "a scratch session must not be in the list at all"
    );
    let (status, body) = ayeaye::session::answer(
        &settings,
        "/api/session",
        Some(&format!("pane=desktop/{}", pane_query(&scratch))),
    )
    .await
    .expect("the session endpoint owns this path");
    assert_eq!(status, 200);
    assert_eq!(
        body, r#"{"kind":null}"#,
        "a pane the list excludes resolved anyway: {scratch}"
    );

    // And it is not that the scratch pane was unresolvable. Handed the `Pane`
    // directly — the type the endpoint can only get from the list — the same
    // `Agents` resolves it, so membership really is the only thing refusing it.
    let excluded = ayeaye_core::tmux::Pane {
        id: ayeaye_core::peer::PaneId::new(host(), &scratch).expect("a pane id"),
        session: "_scratch".to_string(),
        window: "0".to_string(),
        name: "sh".to_string(),
        cmd: "codex".to_string(),
        active: true,
    };
    assert!(
        Agents::under(&scene.home)
            .behind(&server.layer(), &excluded)
            .await
            .is_some(),
        "the excluded pane was refused for the wrong reason"
    );
}

/// Percent-encode a bare tmux pane id for a query string.
fn pane_query(pane: &str) -> String {
    pane.replace('%', "%25")
}
