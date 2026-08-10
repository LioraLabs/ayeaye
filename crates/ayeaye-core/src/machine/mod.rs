//! What this computer is, and what it can actually do.
//!
//! Every module here is a parser and a judgement: raw probe text in, a word or a
//! number out. Nothing runs a command, opens a file or asks the operating system
//! anything — the shell above captures the text and hands it over, which is what
//! makes the whole verdict reproducible from the fixtures under `tests/fixtures`.
//!
//! It is a port of `lib/steps/20-hardware.sh` and `lib/platform.sh`, and it is
//! held to the same corpus those two are tested against.

pub mod platform;

pub use platform::{Family, Os, Platform};
