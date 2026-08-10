//! The shell: the only crate allowed to touch the world.
//!
//! Subprocesses, the filesystem, sockets and model lifetime live here or below
//! in `ayeaye-infer`. Anything that is a decision rather than an effect belongs
//! in `ayeaye-core`, where a test can reach it without a machine — which is
//! also why the async colouring starts here and goes no deeper.
//!
//! This is a library with a binary on top of it so the server can be driven by
//! an integration test over a real socket, rather than only by starting the
//! process and hoping.

pub mod agent;
pub mod assets;
pub mod audio;
pub mod board;
pub mod cliban;
pub mod command;
pub mod config;
pub mod dictate;
pub mod files;
pub mod fit;
pub mod models;
pub mod process;
pub mod projects;
pub mod recorder;
pub mod server;
pub mod session;
pub mod tmux;
