//! The cut-offs, and the arithmetic that turns them into a recommendation.
//!
//! A machine either is or is not over a line, the lines are named constants in
//! one place, and nothing anywhere else writes a number down. Every constant is
//! deliberately shy of the round figure it is named after, because a machine sold
//! as 16 GB reports something between 15.2 and 15.9 once the firmware has taken
//! its share, and a cut-off written as 16384 would put every 16 GB machine on the
//! wrong side of its own line.
//!
//! The shape of it: start at the top, and let every constraint push down. A
//! constraint can only ever lower the answer, and it lowers it *strictly*, so the
//! first constraint to bind is the one that gets to explain itself. There is
//! deliberately no path by which a graphics card raises a tier that memory or
//! free space did not already support — a card cannot hold a model there is no
//! room to download.
//!
//! # The one place this port does not agree with the shell
//!
//! `lib/steps/20-hardware.sh` treats an AMD card like an NVIDIA one: its
//! `hw_accel_usable` calls a card of 4 GB or more usable whatever made it, and
//! its `_hw_tier_compute` lets an 8 GB Radeon on a large machine reach
//! `maximum`. This crate does not, because ayeaye's inference is candle and
//! candle has no ROCm backend — see `ayeaye-infer`, whose only acceleration
//! features are `cuda` and `metal`.
//!
//! The concrete difference, for whoever does the cutover (**AYEAYE-66**): an AMD
//! card at or above [`VRAM_MB_MAXIMUM`] on a machine that clears every other
//! line drops from `maximum` to `recommended`, and its cause word changes from
//! `graphics-small`/`graphics-unknown` to `graphics-unusable`. Nothing else
//! moves; no other acceleration is touched, and no machine is offered *more*
//! than the shell offered it.
//!
//! Two things in the shell still encode the old promise and must go with it, not
//! before it and not after: `test_rocm_is_treated_exactly_like_cuda_at_the_same
//! _size` in `tests/cases/hardware_tier_test.sh`, and the paragraph about AMD in
//! `hw_accel_usable`'s comment. Deleting the shell without deleting them is
//! fine; deleting them without this crate in place is not.

use super::{Acceleration, Graphics, Limits};

// --- the threshold table ---------------------------------------------------
//
// The only place in this crate where a cut-off is written down.
//
// What each line protects against:
//
//   lightweight   below this, speech-to-text is slower than typing and the
//                 machine is better off being told so than discovering it
//   recommended   the comfortable middle: a small transcription model and a
//                 7B instruct model to tidy up what it heard
//   maximum       the largest transcription model resident in memory, which is
//                 only comfortable with a card to hold it

/// A nominal 4 GB machine.
pub const RAM_MB_LIGHTWEIGHT: u64 = 3584;
/// A nominal 8 GB machine.
pub const RAM_MB_RECOMMENDED: u64 = 7168;
/// A nominal 16 GB machine.
pub const RAM_MB_MAXIMUM: u64 = 15360;

pub const CORES_LIGHTWEIGHT: u64 = 2;
pub const CORES_RECOMMENDED: u64 = 4;
pub const CORES_MAXIMUM: u64 = 8;

// Free space where the models land. A base transcription model is about 150 MB,
// a small one 500 MB, the largest 3 GB, and the instruct model that cleans up
// dictation about 4.7 GB. Each line is the models plus room to download them
// before they are unpacked.
pub const DISK_MB_LIGHTWEIGHT: u64 = 1536;
pub const DISK_MB_RECOMMENDED: u64 = 6144;
pub const DISK_MB_MAXIMUM: u64 = 12288;

/// A nominal 4 GB card: this card is worth using at all.
pub const VRAM_MB_ACCELERATED: u64 = 3584;
/// A nominal 8 GB card: this card can hold the largest model.
pub const VRAM_MB_MAXIMUM: u64 = 7168;

// --- end of the threshold table --------------------------------------------

/// How much of what ayeaye offers this machine can actually do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    TextOnly,
    Lightweight,
    Recommended,
    Maximum,
}

impl Tier {
    /// The lower-case word a caller matches on.
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::TextOnly => "text-only",
            Tier::Lightweight => "lightweight",
            Tier::Recommended => "recommended",
            Tier::Maximum => "maximum",
        }
    }
}

/// What held the tier back, for a caller that wants to branch rather than print.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    Ram,
    Disk,
    Cores,
    RamUnknown,
    DiskUnknown,
    CoresUnknown,
    GraphicsNone,
    GraphicsSmall,
    GraphicsUnknown,
    /// There is acceleration here and ayeaye's inference cannot use it.
    GraphicsUnusable,
    Container,
}

impl Cause {
    /// The lower-case word a caller matches on.
    pub fn as_str(self) -> &'static str {
        match self {
            Cause::Ram => "ram",
            Cause::Disk => "disk",
            Cause::Cores => "cores",
            Cause::RamUnknown => "ram-unknown",
            Cause::DiskUnknown => "disk-unknown",
            Cause::CoresUnknown => "cores-unknown",
            Cause::GraphicsNone => "graphics-none",
            Cause::GraphicsSmall => "graphics-small",
            Cause::GraphicsUnknown => "graphics-unknown",
            Cause::GraphicsUnusable => "graphics-unusable",
            Cause::Container => "container",
        }
    }

    /// True when the tier is the least this machine could be rather than what it
    /// measured, so a caller can say so instead of implying a measurement.
    ///
    /// [`Cause::Container`] counts, where the shell's `*-unknown` glob does not:
    /// a container whose share could not be read has not been measured either,
    /// and the numbers it reported describe the host. So a container says "that
    /// is the least this computer could be" where the shell stayed silent. It is
    /// a deliberate second, smaller departure, and the honest direction.
    pub fn is_ignorance(self) -> bool {
        matches!(
            self,
            Cause::RamUnknown
                | Cause::DiskUnknown
                | Cause::CoresUnknown
                | Cause::GraphicsUnknown
                | Cause::Container
        )
    }
}

/// The recommendation, and why it is not higher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verdict {
    /// What this machine can do.
    pub tier: Tier,
    /// What held it back, or `None` when nothing did.
    pub cause: Option<Cause>,
    /// One plain sentence saying why it is not higher, or `None` when nothing
    /// held it back. Never a number and never a unit — short enough that
    /// "Why not more than that: " and a full stop still fit on one line of a
    /// narrow terminal, because a reason that wraps looks like an error message.
    pub reason: Option<&'static str>,
}

/// Whether the acceleration that is there is worth acting on.
///
/// The verdict and the size are different questions, and this is where they
/// meet. A two-gigabyte NVIDIA card is honestly `cuda` and is honestly no use for
/// holding a listening model, so every sentence and every later stage that would
/// otherwise say "your graphics card will do this" asks here first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Usability {
    /// Worth acting on.
    Usable,
    /// There is a card and it is too small to hold a listening model.
    TooSmall,
    /// There is a card and it would not say how big it is. Not a kind of
    /// `TooSmall`: telling somebody their card is too small when what happened
    /// is that a command did not answer is a lie about their hardware.
    Unsized,
    /// There is acceleration here and ayeaye's inference cannot use it at all.
    /// Carries the reason, because detecting it and staying silent is worse than
    /// not detecting it.
    Unsupported(&'static str),
    /// No acceleration this project can name; the processor does the work.
    None,
}

impl Usability {
    /// Status for the one question most callers have.
    pub fn is_usable(self) -> bool {
        self == Usability::Usable
    }
}

/// Why an AMD card is detected and then declined.
///
/// ayeaye's inference is candle, which has no ROCm backend, so an AMD card is an
/// honest `rocm` verdict *and* a card nothing here can run a model on. Saying so
/// is the difference between a true sentence and a useful one; saying nothing
/// would leave somebody with a perfectly good card wondering why it is idle.
const NO_ROCM: &str = "ayeaye cannot use an AMD graphics card";

/// Whether this machine's acceleration is worth acting on, and why not.
pub fn usability(graphics: &Graphics) -> Usability {
    match graphics.acceleration {
        // Apple Silicon is always usable: its memory is the machine's memory,
        // there is no second figure to compare, and how much of it there is has
        // already been answered by the tier.
        Acceleration::Metal => Usability::Usable,
        Acceleration::Rocm => Usability::Unsupported(NO_ROCM),
        Acceleration::Cuda => match graphics.vram_mb {
            None => Usability::Unsized,
            Some(mb) if mb >= VRAM_MB_ACCELERATED => Usability::Usable,
            Some(_) => Usability::TooSmall,
        },
        Acceleration::Cpu => Usability::None,
    }
}

/// Work out what this machine can do, and what stopped it doing more.
pub fn judge(
    ram_mb: Option<u64>,
    cores: Option<u64>,
    disk_mb: Option<u64>,
    graphics: &Graphics,
    limits: Limits,
) -> Verdict {
    let mut verdict = Verdict {
        tier: Tier::Maximum,
        cause: None,
        reason: None,
    };

    match ram_mb {
        Some(mb) => cap(
            &mut verdict,
            dimension(mb, RAM_MB_LIGHTWEIGHT, RAM_MB_RECOMMENDED, RAM_MB_MAXIMUM),
            Cause::Ram,
            "there is not much memory in this computer",
        ),
        None => cap(
            &mut verdict,
            Tier::Lightweight,
            Cause::RamUnknown,
            "setup could not read this computer's memory",
        ),
    }

    match disk_mb {
        Some(mb) => cap(
            &mut verdict,
            dimension(
                mb,
                DISK_MB_LIGHTWEIGHT,
                DISK_MB_RECOMMENDED,
                DISK_MB_MAXIMUM,
            ),
            Cause::Disk,
            "there is not much free space left on the disk",
        ),
        None => cap(
            &mut verdict,
            Tier::Lightweight,
            Cause::DiskUnknown,
            "setup could not read the disk's free space",
        ),
    }

    match cores {
        Some(n) => cap(
            &mut verdict,
            dimension(n, CORES_LIGHTWEIGHT, CORES_RECOMMENDED, CORES_MAXIMUM),
            Cause::Cores,
            "this computer has only a few processors",
        ),
        // Cores are the least of the three, so not knowing them costs the least.
        None => cap(
            &mut verdict,
            Tier::Recommended,
            Cause::CoresUnknown,
            "setup could not count the processors",
        ),
    }

    // Acceleration caps and never lifts, and it asks [`usability`] rather than
    // comparing against VRAM_MB_ACCELERATED a second time. Two copies of that
    // comparison could drift into a machine whose card is reported usable and
    // whose reason says it is not.
    match usability(graphics) {
        // Metal has one pool of memory, shared with the processor, so the memory
        // line above has already said everything there is to say and there is no
        // second figure to be ignorant of. This is the one branch that caps
        // nothing, and it is deliberate: a Mac reaches the top tier only by
        // having the memory for it, which is the same test every other machine
        // passes.
        Usability::Usable if graphics.acceleration == Acceleration::Metal => {}
        // A usable card still has to be big enough for the largest model, which
        // is the second and higher of the two graphics lines.
        Usability::Usable => {
            if graphics.vram_mb.unwrap_or(0) < VRAM_MB_MAXIMUM {
                cap(
                    &mut verdict,
                    Tier::Recommended,
                    Cause::GraphicsSmall,
                    "the graphics card cannot hold the largest model",
                );
            }
        }
        // Below the first line the card is not worth using at all, and this is a
        // processor-only machine that happens to have a card in it. Saying so is
        // the difference between a true sentence and a useful one.
        Usability::TooSmall => cap(
            &mut verdict,
            Tier::Recommended,
            Cause::GraphicsSmall,
            "the graphics card is too small to be any use",
        ),
        Usability::Unsized => cap(
            &mut verdict,
            Tier::Recommended,
            Cause::GraphicsUnknown,
            "setup could not read the graphics card's size",
        ),
        Usability::Unsupported(why) => cap(
            &mut verdict,
            Tier::Recommended,
            Cause::GraphicsUnusable,
            why,
        ),
        Usability::None => cap(
            &mut verdict,
            Tier::Recommended,
            Cause::GraphicsNone,
            "there is no graphics card ayeaye can use here",
        ),
    }

    if limits == Limits::Unknown {
        cap(
            &mut verdict,
            Tier::Recommended,
            Cause::Container,
            "only part of this computer is yours to use",
        );
    }

    verdict
}

/// The tier one measurement supports on its own.
fn dimension(value: u64, lightweight: u64, recommended: u64, maximum: u64) -> Tier {
    if value >= maximum {
        Tier::Maximum
    } else if value >= recommended {
        Tier::Recommended
    } else if value >= lightweight {
        Tier::Lightweight
    } else {
        Tier::TextOnly
    }
}

/// Lower the working answer, and say why.
///
/// Strictly lower, so the first constraint to bind is the one that gets to
/// explain itself and a later constraint of equal weight cannot overwrite a more
/// specific sentence with a vaguer one.
fn cap(verdict: &mut Verdict, tier: Tier, cause: Cause, reason: &'static str) {
    if tier >= verdict.tier {
        return;
    }
    verdict.tier = tier;
    verdict.cause = Some(cause);
    verdict.reason = Some(reason);
}

#[cfg(test)]
mod tests {
    use super::{
        CORES_LIGHTWEIGHT, CORES_MAXIMUM, CORES_RECOMMENDED, Cause, DISK_MB_LIGHTWEIGHT,
        DISK_MB_MAXIMUM, DISK_MB_RECOMMENDED, RAM_MB_LIGHTWEIGHT, RAM_MB_MAXIMUM,
        RAM_MB_RECOMMENDED, Tier, Usability, VRAM_MB_ACCELERATED, VRAM_MB_MAXIMUM, judge,
        usability,
    };
    use crate::machine::{Acceleration, Graphics, Limits};

    fn card(acceleration: Acceleration, vram_mb: Option<u64>) -> Graphics {
        Graphics {
            acceleration,
            name: None,
            vram_mb,
        }
    }

    fn processor_only() -> Graphics {
        card(Acceleration::Cpu, None)
    }

    /// A card big enough to hold anything, so only the value under test varies.
    fn big_card() -> Graphics {
        card(Acceleration::Cuda, Some(VRAM_MB_MAXIMUM))
    }

    /// A machine over every line except whatever the caller varies.
    fn tier_of(ram: Option<u64>, cores: Option<u64>, disk: Option<u64>, gpu: &Graphics) -> Tier {
        judge(ram, cores, disk, gpu, Limits::None).tier
    }

    fn big(gpu: &Graphics) -> Tier {
        tier_of(
            Some(RAM_MB_MAXIMUM),
            Some(CORES_MAXIMUM),
            Some(DISK_MB_MAXIMUM),
            gpu,
        )
    }

    // AYEAYE-60 — every cut-off pinned one under the line and exactly at it. A
    // number that moves without this test moving with it moved by accident.
    #[test]
    fn every_threshold_is_pinned_one_under_and_exactly_at_the_line() {
        let ram = |mb| {
            tier_of(
                Some(mb),
                Some(CORES_MAXIMUM),
                Some(DISK_MB_MAXIMUM),
                &big_card(),
            )
        };
        assert_eq!(ram(RAM_MB_LIGHTWEIGHT - 1), Tier::TextOnly);
        assert_eq!(ram(RAM_MB_LIGHTWEIGHT), Tier::Lightweight);
        assert_eq!(ram(RAM_MB_RECOMMENDED - 1), Tier::Lightweight);
        assert_eq!(ram(RAM_MB_RECOMMENDED), Tier::Recommended);
        assert_eq!(ram(RAM_MB_MAXIMUM - 1), Tier::Recommended);
        assert_eq!(ram(RAM_MB_MAXIMUM), Tier::Maximum);

        let cores = |n| {
            tier_of(
                Some(RAM_MB_MAXIMUM),
                Some(n),
                Some(DISK_MB_MAXIMUM),
                &big_card(),
            )
        };
        assert_eq!(cores(CORES_LIGHTWEIGHT - 1), Tier::TextOnly);
        assert_eq!(cores(CORES_LIGHTWEIGHT), Tier::Lightweight);
        assert_eq!(cores(CORES_RECOMMENDED - 1), Tier::Lightweight);
        assert_eq!(cores(CORES_RECOMMENDED), Tier::Recommended);
        assert_eq!(cores(CORES_MAXIMUM - 1), Tier::Recommended);
        assert_eq!(cores(CORES_MAXIMUM), Tier::Maximum);

        let disk = |mb| {
            tier_of(
                Some(RAM_MB_MAXIMUM),
                Some(CORES_MAXIMUM),
                Some(mb),
                &big_card(),
            )
        };
        assert_eq!(disk(DISK_MB_LIGHTWEIGHT - 1), Tier::TextOnly);
        assert_eq!(disk(DISK_MB_LIGHTWEIGHT), Tier::Lightweight);
        assert_eq!(disk(DISK_MB_RECOMMENDED - 1), Tier::Lightweight);
        assert_eq!(disk(DISK_MB_RECOMMENDED), Tier::Recommended);
        assert_eq!(disk(DISK_MB_MAXIMUM - 1), Tier::Recommended);
        assert_eq!(disk(DISK_MB_MAXIMUM), Tier::Maximum);

        let vram = |mb| big(&card(Acceleration::Cuda, Some(mb)));
        assert_eq!(vram(VRAM_MB_ACCELERATED - 1), Tier::Recommended);
        assert_eq!(
            vram(VRAM_MB_ACCELERATED),
            Tier::Recommended,
            "useful and sufficient are two different lines"
        );
        assert_eq!(vram(VRAM_MB_MAXIMUM - 1), Tier::Recommended);
        assert_eq!(vram(VRAM_MB_MAXIMUM), Tier::Maximum);
    }

    // AYEAYE-60 — one card is too small to hold the biggest model and the other
    // is too small to be worth using at all. A single sentence for both would
    // tell somebody with a perfectly good card to go and buy one.
    #[test]
    fn the_two_graphics_lines_give_two_different_reasons() {
        let just_useful = judge(
            Some(RAM_MB_MAXIMUM),
            Some(CORES_MAXIMUM),
            Some(DISK_MB_MAXIMUM),
            &card(Acceleration::Cuda, Some(VRAM_MB_ACCELERATED)),
            Limits::None,
        );
        assert_eq!(just_useful.tier, Tier::Recommended);
        assert!(
            just_useful
                .reason
                .is_some_and(|r| r.contains("cannot hold the largest model")),
            "{:?}",
            just_useful.reason
        );

        let too_small = judge(
            Some(RAM_MB_MAXIMUM),
            Some(CORES_MAXIMUM),
            Some(DISK_MB_MAXIMUM),
            &card(Acceleration::Cuda, Some(VRAM_MB_ACCELERATED - 1)),
            Limits::None,
        );
        assert!(
            too_small
                .reason
                .is_some_and(|r| r.contains("too small to be any use")),
            "{:?}",
            too_small.reason
        );
    }

    // AYEAYE-60 — acceleration caps and never lifts.
    #[test]
    fn what_acceleration_does_and_does_not_do() {
        assert_eq!(
            tier_of(
                Some(RAM_MB_MAXIMUM * 4),
                Some(CORES_MAXIMUM * 4),
                Some(DISK_MB_MAXIMUM * 4),
                &processor_only()
            ),
            Tier::Recommended,
            "a large processor-only machine can transcribe, just not with the biggest model"
        );
        assert_eq!(
            big(&card(Acceleration::Metal, None)),
            Tier::Maximum,
            "Apple Silicon has one pool, so asking for a second figure would cap every Mac"
        );
        assert_eq!(
            tier_of(
                Some(RAM_MB_LIGHTWEIGHT),
                Some(CORES_MAXIMUM),
                Some(DISK_MB_MAXIMUM),
                &card(Acceleration::Metal, None)
            ),
            Tier::Lightweight,
            "and a small Mac still obeys the memory line"
        );
        assert_eq!(
            tier_of(
                Some(RAM_MB_MAXIMUM),
                Some(CORES_MAXIMUM),
                Some(DISK_MB_LIGHTWEIGHT - 1),
                &big_card()
            ),
            Tier::TextOnly,
            "a card cannot rescue a machine with no room to download the model"
        );
    }

    // AYEAYE-60 — the ticket's decision: AMD is detected and reported unusable
    // for inference, with the reason stated. ayeaye's inference is candle, which
    // has no ROCm backend, so a 24 GB Radeon is an honest rocm verdict and a
    // card nothing here can run a model on.
    //
    // This is the one place the port deliberately departs from the shell, whose
    // hw_accel_usable treats a large AMD card as usable and whose tier lets it
    // reach maximum.
    #[test]
    fn an_amd_card_is_detected_and_declined_out_loud() {
        let radeon = card(Acceleration::Rocm, Some(24560));
        assert_eq!(
            usability(&radeon),
            Usability::Unsupported("ayeaye cannot use an AMD graphics card"),
            "detecting it and staying silent is worse than not detecting it"
        );
        assert!(!usability(&radeon).is_usable());

        let verdict = judge(
            Some(RAM_MB_MAXIMUM),
            Some(CORES_MAXIMUM),
            Some(DISK_MB_MAXIMUM),
            &radeon,
            Limits::None,
        );
        assert_eq!(
            verdict.tier,
            Tier::Recommended,
            "a card ayeaye cannot use is not a card, as far as the model size goes"
        );
        assert_eq!(verdict.cause, Some(Cause::GraphicsUnusable));
        assert_eq!(
            verdict.reason,
            Some("ayeaye cannot use an AMD graphics card")
        );
    }

    // AYEAYE-60 — the four things there can be to say about graphics, in one
    // place, so the detect line and the report paragraph cannot drift apart.
    #[test]
    fn usability_is_the_one_answer_about_whether_a_card_is_worth_acting_on() {
        assert_eq!(
            usability(&card(Acceleration::Metal, None)),
            Usability::Usable
        );
        assert_eq!(
            usability(&card(Acceleration::Cuda, Some(VRAM_MB_ACCELERATED))),
            Usability::Usable
        );
        assert_eq!(
            usability(&card(Acceleration::Cuda, Some(VRAM_MB_ACCELERATED - 1))),
            Usability::TooSmall
        );
        assert_eq!(
            usability(&card(Acceleration::Cuda, None)),
            Usability::Unsized,
            "a card whose size would not read is not a small card"
        );
        assert_eq!(usability(&processor_only()), Usability::None);
    }

    // AYEAYE-60 — every unknown lowers the recommendation rather than raising
    // it, and names a cause a caller can branch on.
    #[test]
    fn not_knowing_costs_a_tier_and_says_which_one_it_was() {
        let unknown_ram = judge(
            None,
            Some(CORES_MAXIMUM),
            Some(DISK_MB_MAXIMUM),
            &big_card(),
            Limits::None,
        );
        assert_eq!(
            unknown_ram.tier,
            Tier::Lightweight,
            "an unreadable memory figure must never read as a large machine"
        );
        assert_eq!(unknown_ram.cause, Some(Cause::RamUnknown));
        assert!(unknown_ram.reason.is_some_and(|r| r.contains("could not")));
        assert!(unknown_ram.reason.is_some_and(|r| r.contains("memory")));

        assert_eq!(
            tier_of(Some(RAM_MB_MAXIMUM), Some(CORES_MAXIMUM), None, &big_card()),
            Tier::Lightweight
        );
        assert_eq!(
            tier_of(
                Some(RAM_MB_MAXIMUM),
                None,
                Some(DISK_MB_MAXIMUM),
                &big_card()
            ),
            Tier::Recommended,
            "cores are the least of the three, so not knowing them costs the least"
        );

        let unsized_card = judge(
            Some(RAM_MB_MAXIMUM),
            Some(CORES_MAXIMUM),
            Some(DISK_MB_MAXIMUM),
            &card(Acceleration::Cuda, None),
            Limits::None,
        );
        assert_eq!(unsized_card.tier, Tier::Recommended);
        assert_eq!(unsized_card.cause, Some(Cause::GraphicsUnknown));

        assert_eq!(
            judge(
                Some(RAM_MB_MAXIMUM),
                Some(CORES_MAXIMUM),
                Some(DISK_MB_MAXIMUM),
                &processor_only(),
                Limits::None
            )
            .cause,
            Some(Cause::GraphicsNone)
        );
    }

    // AYEAYE-60 — inside a container the numbers describe the host, not the
    // share of it. And the reason says so without the word "container", which
    // the audience does not have.
    #[test]
    fn a_container_whose_share_could_not_be_read_is_capped_and_says_why() {
        let opaque = judge(
            Some(RAM_MB_MAXIMUM),
            Some(CORES_MAXIMUM),
            Some(DISK_MB_MAXIMUM),
            &big_card(),
            Limits::Unknown,
        );
        assert_eq!(opaque.tier, Tier::Recommended);
        assert_eq!(opaque.cause, Some(Cause::Container));
        assert!(
            opaque
                .reason
                .is_some_and(|r| r.contains("only part of this computer is yours"))
        );
        assert!(opaque.reason.is_some_and(|r| !r.contains("container")));

        assert_eq!(
            judge(
                Some(RAM_MB_MAXIMUM),
                Some(CORES_MAXIMUM),
                Some(DISK_MB_MAXIMUM),
                &big_card(),
                Limits::Known
            )
            .tier,
            Tier::Maximum,
            "a container whose share was read is not capped for being one"
        );
    }

    // AYEAYE-60
    #[test]
    fn a_machine_at_its_ceiling_has_nothing_to_explain() {
        let ceiling = judge(
            Some(RAM_MB_MAXIMUM),
            Some(CORES_MAXIMUM),
            Some(DISK_MB_MAXIMUM),
            &big_card(),
            Limits::None,
        );
        assert_eq!(ceiling.tier, Tier::Maximum);
        assert_eq!(
            ceiling.reason, None,
            "a reason is what is said when the answer is lower than the numbers suggest"
        );
        assert_eq!(ceiling.cause, None);
    }

    // AYEAYE-60 — the report renders "Why not more than that: <reason>." A
    // reason that wraps is a reason that looks like an error message, and the
    // wrap depends on the length of a string nobody looks at again once it is
    // written.
    #[test]
    fn every_reason_fits_on_one_line_beside_the_words_that_introduce_it() {
        const LEAD: usize = "Why not more than that: ".len();
        let cards = [
            card(Acceleration::Cpu, None),
            card(Acceleration::Metal, None),
            card(Acceleration::Cuda, None),
            card(Acceleration::Cuda, Some(0)),
            card(Acceleration::Cuda, Some(VRAM_MB_ACCELERATED)),
            card(Acceleration::Cuda, Some(VRAM_MB_MAXIMUM)),
            card(Acceleration::Rocm, None),
            card(Acceleration::Rocm, Some(VRAM_MB_MAXIMUM)),
        ];
        let mut seen = 0;
        for ram in [None, Some(RAM_MB_LIGHTWEIGHT - 1), Some(RAM_MB_MAXIMUM)] {
            for cores in [None, Some(CORES_LIGHTWEIGHT - 1), Some(CORES_MAXIMUM)] {
                for disk in [None, Some(DISK_MB_LIGHTWEIGHT - 1), Some(DISK_MB_MAXIMUM)] {
                    for gpu in &cards {
                        for limits in [Limits::None, Limits::Known, Limits::Unknown] {
                            let verdict = judge(ram, cores, disk, gpu, limits);
                            assert_eq!(
                                verdict.cause.is_some(),
                                verdict.reason.is_some(),
                                "a cause and a reason arrive together: {verdict:?}"
                            );
                            let Some(reason) = verdict.reason else {
                                continue;
                            };
                            seen += 1;
                            assert!(!reason.trim().is_empty(), "{verdict:?}");
                            // The lead, the reason, and the full stop.
                            let rendered = LEAD + reason.len() + ".".len();
                            assert!(
                                rendered <= 72,
                                "too long for one line: \"Why not more than that: {reason}.\""
                            );
                        }
                    }
                }
            }
        }
        assert!(
            seen > 500,
            "the cross-product produced almost nothing: {seen}"
        );
    }

    // AYEAYE-60 — model selection asks "did this machine reach that tier",
    // which is what the ordering is for.
    #[test]
    fn the_ladder_is_ordered_lowest_to_highest() {
        assert!(Tier::TextOnly < Tier::Lightweight);
        assert!(Tier::Lightweight < Tier::Recommended);
        assert!(Tier::Recommended < Tier::Maximum);
        assert!(Tier::Recommended >= Tier::Lightweight);
        assert!(Tier::Recommended < Tier::Maximum);
        assert_eq!(
            [
                Tier::TextOnly,
                Tier::Lightweight,
                Tier::Recommended,
                Tier::Maximum,
            ]
            .map(Tier::as_str),
            ["text-only", "lightweight", "recommended", "maximum"]
        );
    }

    // AYEAYE-60 — the cause words are the state-file contract.
    #[test]
    fn the_causes_are_spelled_the_way_the_shell_writes_them() {
        assert_eq!(
            [
                Cause::Ram,
                Cause::Disk,
                Cause::Cores,
                Cause::RamUnknown,
                Cause::DiskUnknown,
                Cause::CoresUnknown,
                Cause::GraphicsNone,
                Cause::GraphicsSmall,
                Cause::GraphicsUnknown,
                Cause::GraphicsUnusable,
                Cause::Container,
            ]
            .map(Cause::as_str),
            [
                "ram",
                "disk",
                "cores",
                "ram-unknown",
                "disk-unknown",
                "cores-unknown",
                "graphics-none",
                "graphics-small",
                "graphics-unknown",
                "graphics-unusable",
                "container",
            ]
        );
        assert!(Cause::RamUnknown.is_ignorance());
        assert!(Cause::Container.is_ignorance());
        assert!(
            !Cause::Ram.is_ignorance(),
            "a measured shortage is not the same as not having looked"
        );
        assert!(
            !Cause::GraphicsUnusable.is_ignorance(),
            "the card was read; ayeaye simply cannot use it"
        );
    }
}
