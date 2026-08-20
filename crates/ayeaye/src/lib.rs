//! The shell: the only crate allowed to touch the world.
//!
//! Subprocesses, the filesystem and sockets live here. Anything that is a
//! decision rather than an effect belongs in `ayeaye-core`, where a test can
//! reach it without a machine — which is also why the async colouring starts
//! here and goes no deeper.
//!
//! Inference is no longer among the effects: since AYEAYE-101 the models live
//! behind `llama-swap`, and `crate::swap` is a socket rather than a stratum.
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
pub mod health;
pub mod models;
pub mod notify;
pub mod overview;
pub mod probe;
pub mod process;
pub mod projects;
pub mod push;
pub mod recorder;
pub mod server;
pub mod service;
pub mod session;
pub mod setup;
pub mod swap;
pub mod tmux;
pub mod transcript;
