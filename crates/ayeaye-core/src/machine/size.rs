//! How big this machine is: cores, memory, and room for the models.
//!
//! Three numbers, each read from whichever source answered, in the order
//! `lib/steps/20-hardware.sh` asks them. The order is judgement and not
//! plumbing — `nproc` answers what this process may actually be scheduled on,
//! which `--cpuset-cpus`, `taskset` and a kubernetes cpuset each change, while
//! `lscpu` answers what the kernel can see, which inside a pinned container is
//! the whole node — so it lives here rather than in the shell that captures the
//! text.
//!
//! Every answer is `Some(n)` or `None`. `None` is "nobody could tell", never
//! zero: zero cores and zero bytes of memory are not machines, they are a probe
//! that answered something it did not mean, and every unknown lowers the
//! recommendation rather than raising it.

use super::Probes;
use super::text::{field, first_word, positive, whole};

/// How many processors this machine has.
pub fn cores(probes: &Probes<'_>) -> Option<u64> {
    None.or_else(|| probes.nproc.and_then(positive))
        .or_else(|| probes.lscpu.and_then(cores_from_lscpu))
        .or_else(|| probes.cpuinfo.and_then(cores_from_cpuinfo))
        .or_else(|| probes.sysctl_ncpu.and_then(positive))
        .or_else(|| {
            probes
                .system_profiler
                .and_then(|text| hardware_overview(text, "Total Number of Cores"))
                .and_then(|value| positive(first_word(&value)))
        })
}

/// How much memory this machine has, in whole megabytes.
pub fn ram_mb(probes: &Probes<'_>) -> Option<u64> {
    None.or_else(|| probes.meminfo.and_then(ram_from_meminfo))
        .or_else(|| probes.free_m.and_then(ram_from_free))
        .or_else(|| {
            probes
                .sysctl_memsize
                .and_then(positive)
                .map(|bytes| bytes / 1_048_576)
        })
        .or_else(|| {
            probes
                .system_profiler
                .and_then(|text| hardware_overview(text, "Memory"))
                .and_then(|value| ram_from_words(&value))
        })
}

/// How much free space there is where the models would land, in whole megabytes.
///
/// `df -Pk` and not `df -h`: POSIX output never wraps a long device name onto a
/// second line, and kilobytes are a number rather than `1.4G`.
pub fn disk_mb(probes: &Probes<'_>) -> Option<u64> {
    let last = probes
        .df_pk?
        .lines()
        .rfind(|line| !line.trim().is_empty())?;
    field(last, 4).and_then(whole).map(|blocks| blocks / 1024)
}

/// Every directory `df` could usefully be asked about, nearest first.
///
/// On a first run nothing has created the model directory yet, and `df` has
/// nothing to say about a path that is not there. The shell walks up to the
/// first ancestor that exists; this is that walk with the existence test left
/// out, so the impure half is one `is_dir` per element of a list it did not have
/// to compute. The last element is always `/`, which is the floor: there is
/// always a filesystem to ask about.
pub fn model_dir_ancestors(model_dir: &str) -> Vec<String> {
    let mut walk = Vec::new();
    let mut here = model_dir;
    while !here.is_empty() && here != "/" {
        walk.push(here.to_string());
        here = match here.rsplit_once('/') {
            Some((head, _)) => head,
            None => "",
        };
    }
    walk.push("/".to_string());
    walk
}

// ------------------------------------------------------------------ readers

/// `lscpu`'s count, at the left margin only.
///
/// `lscpu` prints `CPU(s):` three times — once as the count and twice indented,
/// for the on-line list and for the NUMA node — and an unanchored match picks up
/// whichever comes last.
fn cores_from_lscpu(text: &str) -> Option<u64> {
    text.lines()
        .find_map(|line| line.strip_prefix("CPU(s):"))
        .and_then(positive)
}

fn cores_from_cpuinfo(text: &str) -> Option<u64> {
    let n = text
        .lines()
        .filter(|line| line.starts_with("processor") && line.contains(':'))
        .count() as u64;
    (n > 0).then_some(n)
}

fn ram_from_meminfo(text: &str) -> Option<u64> {
    text.lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))
        .and_then(|value| positive(first_word(value)))
        .map(|kb| kb / 1024)
}

fn ram_from_free(text: &str) -> Option<u64> {
    text.lines()
        .find(|line| line.starts_with("Mem:"))
        .and_then(|line| field(line, 2))
        .and_then(positive)
}

/// `Memory: 24 GB` — the only place a unit has to be read rather than assumed,
/// because `system_profiler` is the only source that prints one.
fn ram_from_words(value: &str) -> Option<u64> {
    let mut words = value.split_whitespace();
    let n = positive(words.next()?)?;
    match words.next()? {
        "GB" | "gb" | "GiB" => n.checked_mul(1024),
        "MB" | "mb" | "MiB" => Some(n),
        "TB" | "tb" | "TiB" => n.checked_mul(1024 * 1024),
        _ => None,
    }
}

/// One labelled line out of `system_profiler SPHardwareDataType`, indented as it
/// arrives.
fn hardware_overview(text: &str, label: &str) -> Option<String> {
    text.lines().find_map(|line| {
        line.trim()
            .strip_prefix(label)?
            .strip_prefix(':')
            .map(|value| value.trim().to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::{cores, disk_mb, model_dir_ancestors, ram_mb};
    use crate::machine::Probes;
    use crate::machine::fixture;

    // AYEAYE-60 — the captured lscpu output, at the figures
    // tests/cases/hardware_probe_test.sh pins. The aarch64 board is the one
    // that matters: its NUMA and on-line lines both say "CPU(s):" too, indented.
    #[test]
    fn lscpu_is_read_at_the_left_margin_and_not_at_the_numa_line() {
        let of = |text| {
            cores(&Probes {
                lscpu: Some(text),
                ..Probes::default()
            })
        };
        assert_eq!(of(fixture!("lscpu/x86_64-8core")), Some(8));
        assert_eq!(of(fixture!("lscpu/aarch64-4core")), Some(4));
    }

    // AYEAYE-60 — nproc first, because it answers what this process may be
    // scheduled on; lscpu answers what the kernel can see.
    #[test]
    fn the_core_count_falls_through_its_sources_in_order() {
        let probes = Probes {
            nproc: Some("6\n"),
            lscpu: Some(fixture!("lscpu/x86_64-8core")),
            ..Probes::default()
        };
        assert_eq!(cores(&probes), Some(6), "nproc is asked first");

        assert_eq!(
            cores(&Probes {
                cpuinfo: Some(
                    "processor\t: 0\nmodel name\t: A\n\nprocessor\t: 1\nmodel name\t: A\n\nprocessor\t: 2\n"
                ),
                ..Probes::default()
            }),
            Some(3)
        );
        assert_eq!(
            cores(&Probes {
                sysctl_ncpu: Some("10\n"),
                ..Probes::default()
            }),
            Some(10)
        );
        assert_eq!(
            cores(&Probes {
                system_profiler: Some(fixture!("system_profiler/apple-m3-24gb")),
                ..Probes::default()
            }),
            Some(8),
            "\"Total Number of Cores: 8 (4 performance and 4 efficiency)\" is eight"
        );
        assert_eq!(cores(&Probes::default()), None);
    }

    // AYEAYE-60 — the megabyte figures the shell suite pins, out of the same
    // captured files.
    #[test]
    fn memory_is_read_in_whole_megabytes_from_whichever_source_answered() {
        let meminfo = |text| {
            ram_mb(&Probes {
                meminfo: Some(text),
                ..Probes::default()
            })
        };
        assert_eq!(
            meminfo(fixture!("meminfo/16gb")),
            Some(15846),
            "16226356 kB is 15846 whole megabytes"
        );
        assert_eq!(meminfo(fixture!("meminfo/2gb")), Some(1849));
        assert_eq!(meminfo(fixture!("meminfo/4gb")), Some(3935));
        assert_eq!(meminfo(fixture!("meminfo/64gb")), Some(64263));
        assert_eq!(
            meminfo(fixture!("meminfo/truncated")),
            None,
            "a meminfo with no total line in it is not a memory figure"
        );

        assert_eq!(
            ram_mb(&Probes {
                free_m: Some(fixture!("free/16gb")),
                ..Probes::default()
            }),
            Some(15845),
            "free -m answers in megabytes already"
        );
        assert_eq!(
            ram_mb(&Probes {
                free_m: Some(fixture!("free/2gb")),
                ..Probes::default()
            }),
            Some(1849)
        );
        assert_eq!(
            ram_mb(&Probes {
                sysctl_memsize: Some("25769803776\n"),
                ..Probes::default()
            }),
            Some(24576)
        );
        assert_eq!(
            ram_mb(&Probes {
                system_profiler: Some(fixture!("system_profiler/apple-m3-24gb")),
                ..Probes::default()
            }),
            Some(24576),
            "\"Memory: 24 GB\""
        );
        assert_eq!(
            ram_mb(&Probes {
                system_profiler: Some(fixture!("system_profiler/intel-i9-32gb")),
                ..Probes::default()
            }),
            Some(32768)
        );
        assert_eq!(ram_mb(&Probes::default()), None);
    }

    // AYEAYE-60
    #[test]
    fn free_space_is_the_available_column_of_the_last_line_df_printed() {
        let of = |text| {
            disk_mb(&Probes {
                df_pk: Some(text),
                ..Probes::default()
            })
        };
        assert_eq!(
            of(fixture!("df/roomy")),
            Some(510580),
            "522834900 blocks of 1 KB, rounded down"
        );
        assert_eq!(of(fixture!("df/tight")), Some(800));
        assert_eq!(of(fixture!("df/eight-gigabytes")), Some(8192));
        assert_eq!(
            of("df: /nope: No such file or directory\n"),
            None,
            "a df that printed nothing useful is unknown and not zero"
        );
        assert_eq!(disk_mb(&Probes::default()), None);
    }

    // AYEAYE-60 — an over-promise here becomes a download the machine cannot
    // run, so anything that is not a whole positive number is thrown away
    // rather than believed.
    #[test]
    fn an_answer_that_is_not_a_number_is_thrown_away_rather_than_believed() {
        let count = |text| {
            cores(&Probes {
                nproc: Some(text),
                ..Probes::default()
            })
        };
        assert_eq!(count("quite a few\n"), None);
        assert_eq!(count("0\n"), None, "zero cores is not a machine");
        assert_eq!(count("\n"), None);
        assert_eq!(count("-1\n"), None);
        assert_eq!(
            ram_mb(&Probes {
                meminfo: Some("MemTotal:       not a lot\n"),
                ..Probes::default()
            }),
            None
        );
    }
    // AYEAYE-60 — asking df about a directory nobody has created answers
    // nothing, so the walk up is part of the question rather than part of the
    // reading.
    #[test]
    fn the_disk_question_walks_up_to_something_that_could_exist() {
        assert_eq!(
            model_dir_ancestors("/home/ada/whisper-models"),
            ["/home/ada/whisper-models", "/home/ada", "/home", "/"]
        );
        assert_eq!(
            model_dir_ancestors("/whisper-models"),
            ["/whisper-models", "/"]
        );
        assert_eq!(
            model_dir_ancestors("/"),
            ["/"],
            "there is always a filesystem to ask about"
        );
        assert_eq!(
            model_dir_ancestors(""),
            ["/"],
            "a caller with no HOME still gets a question df can answer"
        );
    }
}
