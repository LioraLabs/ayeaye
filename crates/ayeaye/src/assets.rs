//! The web UI, compiled in.
//!
//! `include_bytes!` reads `share/` at *compile* time, so the running binary
//! never opens any of it. That is the point: the daemon today hunts for a
//! `share/` directory across three candidate paths and exits if it finds none,
//! and a single portable binary cannot depend on a directory travelling beside
//! it.

/// Every file under `share/`, keyed by the name the route table uses.
///
/// Written out rather than globbed: a build script that walked the directory
/// would embed whatever happened to be there, and this list is a claim about
/// what the binary ships. `tests/assets.rs` holds it against the directory in
/// both directions, so a file added to `share/` and forgotten here fails the
/// suite rather than becoming a silent 404.
const EMBEDDED: &[(&str, &[u8])] = &[
    ("app.html", include_bytes!("../../../share/app.html")),
    ("board.html", include_bytes!("../../../share/board.html")),
    (
        "manifest.webmanifest",
        include_bytes!("../../../share/manifest.webmanifest"),
    ),
    ("favicon.ico", include_bytes!("../../../share/favicon.ico")),
    ("icon-64.png", include_bytes!("../../../share/icon-64.png")),
    (
        "icon-180.png",
        include_bytes!("../../../share/icon-180.png"),
    ),
    (
        "icon-192.png",
        include_bytes!("../../../share/icon-192.png"),
    ),
    (
        "icon-512.png",
        include_bytes!("../../../share/icon-512.png"),
    ),
    (
        "icon-maskable-192.png",
        include_bytes!("../../../share/icon-maskable-192.png"),
    ),
    (
        "icon-maskable-512.png",
        include_bytes!("../../../share/icon-maskable-512.png"),
    ),
];

/// The bytes of an embedded file, if it is one.
pub fn bytes(file: &str) -> Option<&'static [u8]> {
    EMBEDDED
        .iter()
        .find(|(name, _)| *name == file)
        .map(|(_, bytes)| *bytes)
}

/// The names of every embedded file, for the tests that hold this table
/// against the directory it was built from.
pub fn files() -> impl Iterator<Item = &'static str> {
    EMBEDDED.iter().map(|(name, _)| *name)
}
