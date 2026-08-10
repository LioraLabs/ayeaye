//! What graphics this machine has, as a fact and then as a size.
//!
//! The two are different questions and this module answers the first. A
//! two-gigabyte NVIDIA card is honestly `cuda` — CUDA is what is there — and a
//! card whose size will not read is still a card. Whether it is big enough to be
//! worth using, and whether ayeaye can run inference on it at all, is the
//! tier's business, and that is where the sizes live.

use super::text::positive;
use super::{Os, Platform, Probes};

/// What would do the arithmetic here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acceleration {
    /// Apple Silicon, whose memory is the machine's memory.
    Metal,
    Cuda,
    Rocm,
    /// No card this project can name, so the processor does the work.
    Cpu,
}

impl Acceleration {
    /// The lower-case word a caller matches on.
    pub fn as_str(self) -> &'static str {
        match self {
            Acceleration::Metal => "metal",
            Acceleration::Cuda => "cuda",
            Acceleration::Rocm => "rocm",
            Acceleration::Cpu => "cpu",
        }
    }
}

/// The card, if there is one, and what is known about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Graphics {
    /// The verdict: a statement of fact, not of size.
    pub acceleration: Acceleration,
    /// The card's own name, for a sentence that has to name it. `None` when
    /// there is nothing to name.
    pub name: Option<String>,
    /// Graphics memory in whole megabytes. `None` on Apple Silicon, where there
    /// is no second pool, and on a card that would not say.
    pub vram_mb: Option<u64>,
}

impl Graphics {
    /// A machine with no card this project can use.
    fn processor_only() -> Graphics {
        Graphics {
            acceleration: Acceleration::Cpu,
            name: None,
            vram_mb: None,
        }
    }
}

/// Work out what would do the listening work here.
///
/// Precedence, and why: Apple Silicon first, because a Mac's answer cannot be
/// anything else and asking two more commands would only find a way to be wrong;
/// then NVIDIA, then AMD, because on a machine with both CUDA is the
/// better-supported of the two for what this project runs.
pub fn classify(platform: &Platform, probes: &Probes<'_>) -> Graphics {
    if platform.os == Os::Macos {
        // Intel Macs are deliberately not Metal. They have a Metal device, and
        // the accelerated builds this project points people at are Apple
        // Silicon builds; promising acceleration that is not there costs more
        // than declining to promise it.
        if platform.arch != "arm64" {
            return Graphics::processor_only();
        }
        return Graphics {
            acceleration: Acceleration::Metal,
            name: chip_name(probes),
            // One pool of memory, shared with the processor. There is no
            // separate figure to report and inventing one would be a lie in
            // either direction.
            vram_mb: None,
        };
    }
    if let Some(card) = probes.nvidia_smi.and_then(nvidia) {
        return Graphics {
            acceleration: Acceleration::Cuda,
            name: card.name,
            vram_mb: card.vram_mb,
        };
    }
    if let Some(card) = probes.rocminfo.and_then(rocm) {
        return Graphics {
            acceleration: Acceleration::Rocm,
            name: card.name,
            vram_mb: card.vram_mb,
        };
    }
    Graphics::processor_only()
}

/// A card and what could be read about it.
struct Card {
    name: Option<String>,
    vram_mb: Option<u64>,
}

/// What Apple Silicon calls itself.
///
/// `sysctl` answers "Apple M3" immediately; `system_profiler` answers the same
/// thing one to three seconds later, and is only worth waiting for when `sysctl`
/// has nothing to say.
fn chip_name(probes: &Probes<'_>) -> Option<String> {
    let brand = probes
        .sysctl_brand_string
        .map(str::trim)
        .filter(|name| !name.is_empty());
    if let Some(name) = brand {
        return Some(name.to_string());
    }
    probes.system_profiler.and_then(|text| {
        text.lines().find_map(|line| {
            let value = line.trim().strip_prefix("Chip:")?.trim();
            (!value.is_empty()).then(|| value.to_string())
        })
    })
}

/// `nvidia-smi --query-gpu=name,memory.total --format=csv,noheader`.
///
/// The biggest card in a machine is the one that counts: "can a card hold the
/// model" is a question about the largest single card, and nvidia-smi lists them
/// in bus order, which puts a display adapter ahead of the one that does the
/// work as often as not.
fn nvidia(text: &str) -> Option<Card> {
    let mut name: Option<String> = None;
    let mut best = 0;
    let mut found = false;

    for line in text.lines() {
        // A driver that is not loaded prints a paragraph of prose. One comma per
        // card is the shape that tells a listing from an apology, and a card
        // whose name contains a comma keeps it: the split is at the last one.
        let Some((label, size)) = line.rsplit_once(',') else {
            continue;
        };
        let label = label.trim();
        if label.is_empty() {
            continue;
        }
        found = true;
        // The first card's name, whatever else is or is not readable about it:
        // a listing that says nothing but [N/A] still has a card in it to name.
        if name.is_none() {
            name = Some(label.to_string());
        }
        // "[N/A]" is what vGPU, MIG and WSL answer. The card is still a card and
        // the verdict is still cuda; it is the size that is unknown, and unknown
        // is what caps the tier.
        let Some(mb) = positive(size.split_whitespace().next().unwrap_or("")) else {
            continue;
        };
        if mb > best {
            best = mb;
            name = Some(label.to_string());
        }
    }

    found.then(|| Card {
        name,
        vram_mb: (best > 0).then_some(best),
    })
}

/// `rocminfo`.
///
/// One block per HSA agent, and the processor is an agent too, so nothing is
/// believed until an agent whose name is a gfx target has been seen. The size is
/// best effort: the pool sizes are the only figure there is, the largest of them
/// is the card's memory, and a layout this code does not recognise answers
/// nothing rather than a number — which costs the machine a tier and never gains
/// it one.
fn rocm(text: &str) -> Option<Card> {
    let mut in_gpu = false;
    let mut found = false;
    let mut name: Option<String> = None;
    let mut best = 0;
    let mut processor_best = 0;

    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with("Agent ") {
            in_gpu = false;
        } else if let Some(value) = line.strip_prefix("Marketing Name:") {
            if in_gpu && name.is_none() {
                let value = value.trim();
                if !value.is_empty() {
                    name = Some(value.to_string());
                }
            }
        } else if let Some(value) = line.strip_prefix("Name:") {
            if value.trim().starts_with("gfx") {
                in_gpu = true;
                found = true;
            }
        } else if let Some(value) = line.strip_prefix("Size:") {
            // "25149440(0x17fc000) KB" — the parenthesis is commentary.
            let digits = value.split('(').next().unwrap_or("");
            let Some(kb) = positive(digits.split_whitespace().next().unwrap_or("")) else {
                continue;
            };
            if in_gpu {
                best = best.max(kb);
            } else {
                // The processor's own pool, kept so the card's can be compared
                // against it below.
                processor_best = processor_best.max(kb);
            }
        }
    }

    if !found {
        return None;
    }
    // Graphics built into the processor is an HSA agent like any other, and the
    // pool it reports is the machine's own memory — the same gigabytes the
    // memory line has already counted. Reporting them again as graphics memory
    // would let one pool satisfy two independent checks, which is exactly the
    // double-count the size check exists to prevent. Within a tenth of the
    // processor's pool means shared, and shared means there is no separate
    // figure.
    if best > 0 && processor_best > 0 && best + best / 10 >= processor_best {
        best = 0;
    }
    Some(Card {
        name,
        vram_mb: (best > 0).then_some(best / 1024),
    })
}

#[cfg(test)]
mod tests {
    use super::{Acceleration, Graphics, classify};
    use crate::machine::fixture;
    use crate::machine::{Probes, identify};

    /// An ordinary Linux machine, so only the graphics probes vary.
    fn on_linux(probes: Probes<'_>) -> Graphics {
        classify(
            &identify(None, Some("Linux"), Some("x86_64"), None),
            &probes,
        )
    }

    // AYEAYE-60 — the captured nvidia-smi listings, at the verdict, name and
    // size tests/cases/hardware_probe_test.sh pins for each.
    #[test]
    fn an_nvidia_listing_names_its_biggest_card_and_reads_its_size() {
        let one = on_linux(Probes {
            nvidia_smi: Some(fixture!("nvidia-smi/rtx-4090")),
            ..Probes::default()
        });
        assert_eq!(one.acceleration, Acceleration::Cuda);
        assert_eq!(one.vram_mb, Some(24564));
        assert_eq!(one.name.as_deref(), Some("NVIDIA GeForce RTX 4090"));

        // nvidia-smi lists cards in bus order, which puts a display adapter
        // ahead of the one that does the work as often as not.
        let two = on_linux(Probes {
            nvidia_smi: Some(fixture!("nvidia-smi/two-cards")),
            ..Probes::default()
        });
        assert_eq!(two.vram_mb, Some(24564));
        assert_eq!(two.name.as_deref(), Some("NVIDIA GeForce RTX 4090"));
    }

    // AYEAYE-60 — "[N/A]" is what a MIG slice, a vGPU and WSL answer. The
    // verdict is a statement of fact and the fact is that CUDA is there.
    #[test]
    fn a_card_that_will_not_say_its_size_is_still_a_card() {
        let mig = on_linux(Probes {
            nvidia_smi: Some(fixture!("nvidia-smi/no-size")),
            ..Probes::default()
        });
        assert_eq!(mig.acceleration, Acceleration::Cuda);
        assert_eq!(mig.vram_mb, None, "it is the size that is unknown");
        assert!(mig.name.is_some(), "there is still a card in it to name");
    }

    // AYEAYE-60 — the tool being installed is not the same as a card being
    // present.
    #[test]
    fn nvidia_smi_with_nothing_to_list_is_not_cuda() {
        let apology = on_linux(Probes {
            nvidia_smi: Some(fixture!("nvidia-smi/no-devices")),
            ..Probes::default()
        });
        assert_eq!(apology.acceleration, Acceleration::Cpu);
        assert_eq!(apology.vram_mb, None);

        let silent = on_linux(Probes {
            nvidia_smi: Some(""),
            ..Probes::default()
        });
        assert_eq!(silent.acceleration, Acceleration::Cpu);
    }

    // AYEAYE-60
    #[test]
    fn an_amd_card_is_rocm_and_its_pool_is_read() {
        let card = on_linux(Probes {
            rocminfo: Some(fixture!("rocminfo/gfx1100")),
            ..Probes::default()
        });
        assert_eq!(card.acceleration, Acceleration::Rocm);
        assert_eq!(card.vram_mb, Some(24560), "25149440 KB");
        assert_eq!(card.name.as_deref(), Some("AMD Radeon RX 7900 XTX"));

        // The card is real, and the recommendation stays conservative because
        // the size is not known rather than because the card is not there.
        let unsized_card = on_linux(Probes {
            rocminfo: Some(fixture!("rocminfo/gfx-without-a-size")),
            ..Probes::default()
        });
        assert_eq!(unsized_card.acceleration, Acceleration::Rocm);
        assert_eq!(unsized_card.vram_mb, None);

        let processor = on_linux(Probes {
            rocminfo: Some(fixture!("rocminfo/no-gpu")),
            ..Probes::default()
        });
        assert_eq!(processor.acceleration, Acceleration::Cpu);
    }

    // AYEAYE-60 — an integrated Radeon is an HSA agent like any other and the
    // pool it reports is the machine's own memory. Reporting it again as
    // graphics memory would let one pool satisfy two independent checks.
    #[test]
    fn graphics_built_into_the_processor_do_not_count_their_pool_twice() {
        let integrated = on_linux(Probes {
            rocminfo: Some(concat!(
                "==========\nHSA Agents\n==========\n",
                "*******\nAgent 1\n*******\n",
                "  Name:                    AMD Ryzen 7 7840U\n",
                "  Device Type:             CPU\n",
                "  Pool Info:\n    Pool 1\n",
                "      Size:                    32505856(0x1f00000) KB\n",
                "*******\nAgent 2\n*******\n",
                "  Name:                    gfx1103\n",
                "  Marketing Name:          AMD Radeon 780M Graphics\n",
                "  Device Type:             GPU\n",
                "  Pool Info:\n    Pool 1\n",
                "      Size:                    32505856(0x1f00000) KB\n",
            )),
            ..Probes::default()
        });
        assert_eq!(
            integrated.acceleration,
            Acceleration::Rocm,
            "the card is real"
        );
        assert_eq!(
            integrated.vram_mb, None,
            "and it has no memory of its own to offer"
        );
    }

    // AYEAYE-60
    #[test]
    fn a_mac_is_metal_only_on_apple_silicon() {
        let mac = |machine, probes| {
            classify(
                &identify(None, Some("Darwin"), Some(machine), None),
                &probes,
            )
        };
        let apple = mac(
            "arm64",
            Probes {
                sysctl_brand_string: Some("Apple M3\n"),
                ..Probes::default()
            },
        );
        assert_eq!(apple.acceleration, Acceleration::Metal);
        assert_eq!(apple.name.as_deref(), Some("Apple M3"));
        assert_eq!(apple.vram_mb, None, "one pool, already counted as memory");

        let named_by_profiler = mac(
            "arm64",
            Probes {
                system_profiler: Some(fixture!("system_profiler/apple-m3-24gb")),
                ..Probes::default()
            },
        );
        assert_eq!(named_by_profiler.name.as_deref(), Some("Apple M3"));

        let intel = mac(
            "x86_64",
            Probes {
                system_profiler: Some(fixture!("system_profiler/intel-i9-32gb")),
                ..Probes::default()
            },
        );
        assert_eq!(intel.acceleration, Acceleration::Cpu);
        assert_eq!(intel.vram_mb, None);
    }

    // AYEAYE-60
    #[test]
    fn the_precedence_is_metal_then_cuda_then_rocm() {
        let both_cards = Probes {
            nvidia_smi: Some(fixture!("nvidia-smi/rtx-4090")),
            rocminfo: Some(fixture!("rocminfo/gfx1100")),
            ..Probes::default()
        };
        assert_eq!(on_linux(both_cards).acceleration, Acceleration::Cuda);

        let mac_with_a_card = classify(
            &identify(None, Some("Darwin"), Some("arm64"), None),
            &Probes {
                nvidia_smi: Some(fixture!("nvidia-smi/rtx-4090")),
                ..Probes::default()
            },
        );
        assert_eq!(mac_with_a_card.acceleration, Acceleration::Metal);

        let bare = on_linux(Probes::default());
        assert_eq!(bare.acceleration, Acceleration::Cpu);
        assert_eq!(bare.name, None);
        assert_eq!(bare.vram_mb, None);
    }
}
