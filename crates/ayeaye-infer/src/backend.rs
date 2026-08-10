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

    /// Which backend a device is.
    ///
    /// This is how anything holding a device answers "where am I running",
    /// rather than by asking [`selected`] again. The two are different
    /// questions and they have different answers the moment a fallback
    /// happens, so deriving the report from the device is what makes a report
    /// that contradicts the device impossible to write rather than merely
    /// discouraged.
    pub fn of(device: &candle_core::Device) -> Backend {
        match device {
            candle_core::Device::Cpu => Backend::Cpu,
            candle_core::Device::Cuda(_) => Backend::Cuda,
            candle_core::Device::Metal(_) => Backend::Metal,
        }
    }
}

/// The backend this build was compiled with.
///
/// `cuda` wins over `metal` when a build somehow carries both, which no
/// released row of the matrix does; the tie is broken here rather than left to
/// whichever `cfg` happens to be written first.
///
/// **Metal arrives two ways, and `target_os` is the one that matters.** A cargo
/// feature nobody passes is off, so an Apple build gets Metal from the
/// target-conditional dependency table in this crate's `Cargo.toml` rather than
/// from `--features metal`. The manifest and this function are two halves of
/// one claim, and the test named for it is what stops them drifting.
pub const fn selected() -> Backend {
    #[cfg(feature = "cuda")]
    {
        Backend::Cuda
    }
    #[cfg(all(not(feature = "cuda"), any(feature = "metal", target_os = "macos")))]
    {
        Backend::Metal
    }
    #[cfg(all(
        not(feature = "cuda"),
        not(feature = "metal"),
        not(target_os = "macos")
    ))]
    {
        Backend::Cpu
    }
}

/// Which device inference is actually running on, and why it is that one.
///
/// A build is compiled for one backend and runs on whatever the machine turned
/// out to have, and those are two different facts. Keeping both is the point:
/// "this build has cuda in it" and "this process is using a card" have
/// different answers on a machine whose driver did not load, and reporting the
/// first as though it were the second is the silent degradation the ticket
/// exists to refuse.
#[derive(Debug, Clone)]
pub struct Selection {
    /// The backend this build was compiled for.
    pub asked: Backend,
    /// The device to build tensors on.
    pub device: candle_core::Device,
    /// Why the device is not the one asked for, in words a user can act on.
    /// `None` exactly when nothing was given up.
    pub fallback: Option<String>,
}

impl Selection {
    /// The backend actually in use. Read off the device rather than stored,
    /// so there is no second copy of the answer to fall out of step with it.
    pub fn got(&self) -> Backend {
        Backend::of(&self.device)
    }
}

/// Pick a device for `asked`, falling back to the processor with a reason.
///
/// Total: there is no error type, because a card that will not open is not a
/// reason to refuse to transcribe. Slower is not broken, and the milestone's
/// whole claim is that a CPU build works everywhere — so a GPU build that
/// cannot find its GPU should become that build, out loud.
///
/// `open` is an argument rather than a call, and that is what makes this
/// testable at all: the machines that can exercise a real fallback are exactly
/// the ones nobody develops on. See [`open`] for the one line it stands in for.
pub fn choose<F>(asked: Backend, open: F) -> Selection
where
    F: FnOnce(Backend) -> candle_core::Result<candle_core::Device>,
{
    match open(asked) {
        Ok(device) => Selection {
            asked,
            device,
            fallback: None,
        },
        // A processor that will not open is not a fallback, it is a machine
        // with no arithmetic on it. There is nowhere below this to go, so the
        // reason is stated and the CPU device — which candle builds without
        // asking anything of the system — is used regardless.
        Err(why) => Selection {
            asked,
            device: candle_core::Device::Cpu,
            fallback: (asked != Backend::Cpu).then(|| {
                format!(
                    "this build has {} compiled in, but no {} device would open ({why}); \
                     running on the processor instead",
                    asked.label(),
                    asked.label(),
                )
            }),
        },
    }
}

/// Open the device a backend runs on.
///
/// The one effectful line, kept apart from the decision in [`choose`] so that
/// the decision can be tested on a machine with neither card. It is also the
/// only place that can fail: `Device::Cpu` is a value, not a resource.
pub fn open(backend: Backend) -> candle_core::Result<candle_core::Device> {
    match backend {
        Backend::Cpu => Ok(candle_core::Device::Cpu),
        Backend::Cuda => candle_core::Device::new_cuda(0),
        Backend::Metal => candle_core::Device::new_metal(0),
    }
}

/// What this process should run inference on.
///
/// The whole of the runtime device decision, in one call, so that two models
/// cannot make it two different ways.
///
/// **What this cannot cover, and it matters for the NVIDIA artifact.** A build
/// with `cuda` in it is dynamically linked against `libcudart` — that link is
/// emitted by `candle-kernels`' build script, not by anything here — so a
/// machine with no CUDA runtime at all fails in the loader before `main` is
/// reached. The fallback below covers a device that will not open: no card, a
/// driver that did not load, a card already taken. It does not, and cannot,
/// cover a missing CUDA runtime.
pub fn select() -> Selection {
    choose(selected(), open)
}

#[cfg(test)]
mod tests {
    use super::{Backend, choose, selected};
    use candle_core::Device;

    /// An opener that refuses, the way a driver that is not there refuses.
    fn refuses(message: &'static str) -> impl FnOnce(Backend) -> candle_core::Result<Device> {
        move |_| Err(candle_core::Error::Msg(message.to_string()))
    }

    // AYEAYE-57 — the acceptance criterion, and the only way to watch it happen
    // on a machine with no card: the effect is the argument.
    #[test]
    fn a_device_that_will_not_open_falls_back_to_the_processor_with_a_reason() {
        let selection = choose(Backend::Cuda, refuses("no CUDA-capable device is detected"));

        assert_eq!(selection.asked, Backend::Cuda);
        assert_eq!(selection.got(), Backend::Cpu);
        assert!(matches!(selection.device, Device::Cpu));
        let why = selection.fallback.expect("a stated reason, not a silence");
        assert!(why.contains("cuda"), "{why}");
        assert!(
            why.contains("no CUDA-capable device is detected"),
            "the driver's own words are the useful half: {why}"
        );
    }

    // AYEAYE-41
    //
    // The assertion is chosen at compile time, because so is the answer: run
    // the suite with `--features cuda` and this test changes what it demands.
    // AYEAYE-57 amends it: Metal arrives two ways, and being an Apple build is
    // the one that matters, because nobody passes a feature. This arm is red on
    // macOS until `selected()` learns it, and unreachable anywhere else — which
    // is why `an_apple_build_gets_metal_without_anyone_passing_a_feature` above
    // is the half that runs here.
    #[test]
    fn selected_reports_the_acceleration_this_build_was_compiled_with() {
        #[cfg(feature = "cuda")]
        assert_eq!(selected(), Backend::Cuda);
        #[cfg(all(not(feature = "cuda"), any(feature = "metal", target_os = "macos")))]
        assert_eq!(selected(), Backend::Metal);
        #[cfg(all(
            not(feature = "cuda"),
            not(feature = "metal"),
            not(target_os = "macos")
        ))]
        assert_eq!(selected(), Backend::Cpu);
    }

    // AYEAYE-57 — the other half, and the one that stops the rule above being
    // satisfied by a `choose` that always falls back.
    //
    // The opener hands back a CPU device while claiming to have opened a Metal
    // one, which is a lie no real opener can tell. That is deliberate and it is
    // the shape of the design: `got()` is read off the device, so a fake cannot
    // forge it, and what is left to assert here is the only thing `choose`
    // decides on its own — whether there is anything to explain. A `choose`
    // that reported a fallback for a device that opened would fail here.
    #[test]
    fn a_device_that_opens_leaves_nothing_to_explain() {
        let selection = choose(Backend::Metal, |_| Ok(Device::Cpu));

        assert_eq!(selection.asked, Backend::Metal);
        assert_eq!(selection.fallback, None, "nothing happened worth reporting");
    }

    // AYEAYE-57 — a CPU build has nowhere below it to go. Reporting a fallback
    // there would be a warning about a degradation that did not happen, which
    // is how a real one stops being read.
    #[test]
    fn a_processor_build_never_reports_falling_back_to_the_processor() {
        let selection = choose(Backend::Cpu, refuses("something impossible"));

        assert_eq!(selection.got(), Backend::Cpu);
        assert!(matches!(selection.device, Device::Cpu));
        assert_eq!(selection.fallback, None);
    }

    // AYEAYE-57 — "the Apple feature is on by default in Apple builds", as
    // something other than an intention.
    //
    // A cargo feature nobody passes is off, so `[features] metal` is not how an
    // Apple build gets Metal; a target-conditional dependency table is. That
    // table and the `target_os = "macos"` arm of `selected()` are two halves of
    // one claim written in two files, and neither half can be compiled on this
    // machine — so this is the test that holds them together here. It reads the
    // manifest through the compiler, which is the only way a test gets at a
    // file it must not open.
    #[test]
    fn an_apple_build_gets_metal_without_anyone_passing_a_feature() {
        const MANIFEST: &str = include_str!("../Cargo.toml");

        let table = MANIFEST
            .split("[target.'cfg(target_os = \"macos\")'.dependencies]")
            .nth(1)
            .expect("a macOS dependency table, or an Apple build has no acceleration in it");
        // To the next table header, so a later section cannot satisfy this.
        let table = table.split("\n[").next().unwrap_or(table);

        for crate_name in ["candle-core", "candle-nn", "candle-transformers"] {
            let line = table
                .lines()
                .find(|line| line.trim_start().starts_with(crate_name))
                .unwrap_or_else(|| panic!("{crate_name} is not in the macOS table: {table}"));
            assert!(
                line.contains("\"metal\""),
                "{crate_name} is in the macOS table without metal on: {line}"
            );
        }
    }

    // AYEAYE-41
    #[test]
    fn every_backend_has_a_short_label() {
        assert_eq!(Backend::Cpu.label(), "cpu");
        assert_eq!(Backend::Cuda.label(), "cuda");
        assert_eq!(Backend::Metal.label(), "metal");
    }
}
