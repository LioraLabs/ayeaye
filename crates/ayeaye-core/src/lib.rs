//! The pure core of ayeaye.
//!
//! Text and structs in, text and structs out. Nothing in this crate opens a
//! file, starts a process, reads a clock, touches the network, or looks at the
//! environment — including its own tests, which is why fixtures arrive through
//! `include_str!` rather than through anything that can fail at runtime.
//!
//! That is not a convention. The `constitution` crate scans this crate's source
//! and its manifest on every run of the suite, and a reach outside the budget
//! fails the build. The rules, and how to amend one, are in
//! `crates/constitution/README.md`.

pub mod http;
pub mod identity;
pub mod machine;
pub mod projects;
pub mod service;

pub use identity::Identity;
pub use machine::Machine;

/// The version this build claims to be.
///
/// `env!` reads the environment of the *compiler*, at compile time, and is
/// therefore pure — unlike `std::env`, which reads the environment of the
/// running process and is not.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
