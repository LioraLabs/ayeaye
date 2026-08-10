//! The decisions an HTTP request needs, with no HTTP in them.
//!
//! Everything here takes header values and paths as `&str` and returns a
//! verdict. The server that reads a socket, and the framework that parses the
//! request, live above this in the `ayeaye` crate; what is left down here is
//! the part a test can reach without a listening port.

pub mod auth;
pub mod hosts;
pub mod login;
pub mod origin;
pub mod refused;
pub mod route;
