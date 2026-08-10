//! The binary: the only crate allowed to touch the world.
//!
//! Subprocesses, the filesystem, sockets, and model lifetime live here or
//! below in `ayeaye-infer`. Anything that is a decision rather than an effect
//! belongs in `ayeaye-core`, where a test can reach it without a machine.

fn main() {
    let backend = ayeaye_infer::backend::selected();
    let identity = ayeaye_core::Identity {
        version: ayeaye_core::VERSION,
        capabilities: &[backend.label()],
    };
    println!("{}", identity.banner());
}
