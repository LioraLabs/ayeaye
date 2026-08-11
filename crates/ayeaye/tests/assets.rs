//! The embedded UI, held against the directory it was built from.
//!
//! This is the one test that has to read `share/` from disk: the claim is that
//! the bytes in the binary *are* the files, and nothing that only looks at the
//! binary can tell you that.

use std::fs;
use std::path::PathBuf;

fn share_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("share")
}

fn files_on_disk() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(share_dir())
        .expect("share/ should be readable from the workspace")
        .map(|entry| entry.expect("a readable directory entry"))
        .filter(|entry| entry.path().is_file())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

// AYEAYE-42 — "the web UI and its icons are compiled into the binary, not read
// from disk beside it". Every file in share/ has to be in there, byte for
// byte, or an icon added to the directory ships as a 404 nobody notices.
#[test]
fn every_file_in_share_is_embedded_byte_for_byte() {
    let on_disk = files_on_disk();
    assert!(
        on_disk.len() >= 10,
        "share/ holds {} files, fewer than the two pages, the manifest and the \
         seven icons that are there today — either this test is looking in the \
         wrong place, or a file was removed and this floor is the deliberate act",
        on_disk.len()
    );

    for name in &on_disk {
        let embedded = ayeaye::assets::bytes(name)
            .unwrap_or_else(|| panic!("{name} is in share/ but not compiled into the binary"));
        let from_disk = fs::read(share_dir().join(name)).expect("the file should be readable");
        assert_eq!(
            embedded.len(),
            from_disk.len(),
            "{name} is embedded at a different length than the file on disk"
        );
        assert!(
            embedded == from_disk.as_slice(),
            "{name} is embedded with different bytes than the file on disk"
        );
    }
}

// AYEAYE-42 — and the other direction: an embedded name that no longer exists
// on disk is a stale `include_bytes!` that will fail to compile on the next
// clean checkout, or worse, keep serving a file somebody deleted.
#[test]
fn nothing_is_embedded_that_share_does_not_hold() {
    let on_disk = files_on_disk();
    for name in ayeaye::assets::files() {
        assert!(
            on_disk.iter().any(|found| found == name),
            "{name} is compiled into the binary but is not in share/"
        );
    }
}

// AYEAYE-42 — the route table names files; this is what says those names
// resolve. A path the daemon answers on whose file is not embedded is a 500 in
// production and nothing at all in the pure tests, which cannot see the bytes.
#[test]
fn every_file_the_route_table_names_is_embedded() {
    for (path, asset) in ayeaye_core::http::route::ASSET_ROUTES {
        assert!(
            ayeaye::assets::bytes(asset.file).is_some(),
            "{path} is served from {}, which is not compiled into the binary",
            asset.file
        );
    }
}

// AYEAYE-85 — the worker does exactly push and click, with no cache/fetch scope.
#[test]
fn the_service_worker_always_notifies_and_clicks_back_into_the_app() {
    let worker = String::from_utf8_lossy(
        ayeaye::assets::bytes("service-worker.js").expect("the worker is embedded"),
    );
    for required in [
        "addEventListener('push'",
        "showNotification",
        "addEventListener('notificationclick'",
        ".focus()",
        "clients.openWindow('/')",
    ] {
        assert!(worker.contains(required), "worker is missing {required:?}");
    }
    assert!(!worker.contains("addEventListener('fetch'"));
    assert!(!worker.contains("caches."));
}

// AYEAYE-85 — the shipped page owns the complete native opt-in state machine.
#[test]
fn the_app_exposes_one_native_notification_control_and_explicit_fallbacks() {
    let page = String::from_utf8_lossy(
        ayeaye::assets::bytes("app.html").expect("the app is embedded"),
    );
    for required in [
        "id=\"notifications\"",
        "aria-live=\"polite\"",
        "serviceWorker.register('/service-worker.js')",
        "Notification.requestPermission()",
        "pushManager.subscribe",
        "'/api/push/public-key'",
        "'/api/push/subscribe'",
        "'/api/push/unsubscribe'",
        ".unsubscribe()",
        "notifications are not supported",
        "notification permission is denied",
        "Share → Add to Home Screen",
        "notifications need HTTPS",
        "href=\"https://github.com/LioraLabs/ayeaye#secure-serving\"",
    ] {
        assert!(page.contains(required), "app is missing {required:?}");
    }
}
