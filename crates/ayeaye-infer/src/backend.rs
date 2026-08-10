//! Which device this build can run inference on.

/// The compute device inference runs on.
///
/// Which one is available is fixed when the binary is compiled, so this is a
/// report of the build rather than a choice made at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Pure Rust, everywhere, no toolchain beyond cargo.
    Cpu,
    /// NVIDIA, compiled in behind the `cuda` feature.
    Cuda,
    /// Apple, compiled in behind the `metal` feature.
    Metal,
}

impl Backend {
    /// The short name this backend goes by in output and configuration.
    pub fn label(self) -> &'static str {
        match self {
            Backend::Cpu => "cpu",
            Backend::Cuda => "cuda",
            Backend::Metal => "metal",
        }
    }
}

/// The backend this build was compiled with.
///
/// `cuda` wins over `metal` when a build somehow carries both, which no
/// released row of the matrix does; the tie is broken here rather than left to
/// whichever `cfg` happens to be written first.
pub const fn selected() -> Backend {
    #[cfg(feature = "cuda")]
    {
        Backend::Cuda
    }
    #[cfg(all(feature = "metal", not(feature = "cuda")))]
    {
        Backend::Metal
    }
    #[cfg(not(any(feature = "cuda", feature = "metal")))]
    {
        Backend::Cpu
    }
}

#[cfg(test)]
mod tests {
    use super::{Backend, selected};

    // AYEAYE-41
    //
    // The assertion is chosen at compile time, because so is the answer: run
    // the suite with `--features cuda` and this test changes what it demands.
    #[test]
    fn selected_reports_the_acceleration_this_build_was_compiled_with() {
        #[cfg(feature = "cuda")]
        assert_eq!(selected(), Backend::Cuda);
        #[cfg(all(feature = "metal", not(feature = "cuda")))]
        assert_eq!(selected(), Backend::Metal);
        #[cfg(not(any(feature = "cuda", feature = "metal")))]
        assert_eq!(selected(), Backend::Cpu);
    }

    // AYEAYE-41
    #[test]
    fn every_backend_has_a_short_label() {
        assert_eq!(Backend::Cpu.label(), "cpu");
        assert_eq!(Backend::Cuda.label(), "cuda");
        assert_eq!(Backend::Metal.label(), "metal");
    }
}
