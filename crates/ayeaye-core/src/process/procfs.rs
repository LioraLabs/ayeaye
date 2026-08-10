//! What Linux publishes about a process, and what can be read out of it.
//!
//! Every function here takes the text of one file. The shell opens them; the
//! clock, the boot time and the tick rate all arrive as arguments, because a
//! function that reached for any of the three could not be tested without the
//! machine it was written on.

/// When a process began, in seconds since the epoch.
///
/// `stat` is `/proc/<pid>/stat` and `uptime` is `/proc/<pid>/../uptime`; `now`
/// is the wall clock the caller read, and `clk_tck` is `SC_CLK_TCK`.
///
/// The process name sits in parentheses in the middle of `stat` and may contain
/// one itself, so everything up to the *last* `)` is discarded and what remains
/// starts at field 3 — which puts `starttime`, field 22, at index 19. It counts
/// clock ticks since boot, and boot is now minus uptime.
pub fn start_time(stat: &str, uptime: &str, clk_tck: u64, now: f64) -> Option<f64> {
    if clk_tck == 0 {
        return None;
    }
    let (_, after_name) = stat.rsplit_once(')')?;
    let ticks: f64 = after_name.split_whitespace().nth(19)?.parse().ok()?;
    let seconds_up: f64 = uptime.split_whitespace().next()?.parse().ok()?;
    Some((now - seconds_up) + ticks / clk_tck as f64)
}

/// The pids the kernel lists in `/proc/<pid>/task/<tid>/children`.
///
/// One line, space separated, and empty for a process with none. Anything that
/// is not a number is skipped rather than ending the read: this file is written
/// while the processes it names are starting and stopping.
///
/// It is one file *per thread*, and a process's children are spread across all
/// of them — whichever thread called `fork` is the one that lists the child. A
/// caller that reads only the main thread's file loses every child of every
/// other thread, silently, so the answers for a process's tasks are unioned.
pub fn children(text: &str) -> Vec<u32> {
    text.split_whitespace()
        .filter_map(|word| word.parse().ok())
        .collect()
}

/// Who this process's parent is, from `/proc/<pid>/stat` field 4.
///
/// Same split as [`start_time`], for the same reason: the name is in
/// parentheses and may contain one, so what is left of the line starts at field
/// 3 and the parent is the one after it.
///
/// This is the answer of last resort. A kernel built without `CONFIG_PROC_CHILDREN`
/// publishes no children file, and the only way left to find a process's
/// children is to ask every process who its parent is — which is what `pgrep`
/// does, and what the targeted read exists to avoid.
pub fn parent(stat: &str) -> Option<u32> {
    let (_, after_name) = stat.rsplit_once(')')?;
    after_name.split_whitespace().nth(1)?.parse().ok()
}

/// The address a process was reached from, out of `/proc/<pid>/environ`.
///
/// `None` means the process is sitting at the machine, or that nothing could be
/// learned — the two are the same answer to the only caller, which is choosing
/// between clients of its own.
///
/// The block is what the kernel copied in at exec: NUL-separated, which is what
/// makes it safe to read a value containing spaces, and `SSH_CONNECTION` always
/// contains three of them. Entries are matched whole, so `OLD_SSH_CONNECTION`
/// does not answer for it. Bytes are decoded lossily: an environment nobody can
/// decode is not a reason to fail a request.
///
/// An entry that is set but empty is skipped rather than ending the search, the
/// same way [`super::ps::ssh_peer_in_command`] does — this is the one question
/// the two platforms must answer identically, and two implementations of it
/// answering differently is the failure the module they replace was extracted
/// to stop.
pub fn ssh_peer(environ: &[u8]) -> Option<String> {
    const NAME: &[u8] = b"SSH_CONNECTION=";
    environ
        .split(|byte| *byte == 0)
        .filter(|entry| entry.starts_with(NAME))
        .find_map(|entry| {
            let value = String::from_utf8_lossy(&entry[NAME.len()..]);
            Some(value.split_whitespace().next()?.to_string())
        })
}

#[cfg(test)]
mod tests {
    use super::{children, parent, ssh_peer, start_time};

    /// A `/proc/<pid>/stat` line, shaped like the real thing.
    ///
    /// The padding after `starttime` is deliberate: a parser that took the last
    /// field rather than the twentieth would pass without it.
    fn stat(name: &str, start_ticks: u64) -> String {
        let mut fields = vec!["S".to_string(), "1".to_string()];
        fields.extend(std::iter::repeat_n("0".to_string(), 17));
        fields.push(start_ticks.to_string());
        fields.extend(std::iter::repeat_n("7".to_string(), 30));
        format!("42 ({name}) {}\n", fields.join(" "))
    }

    const CLK_TCK: u64 = 100;
    const NOW: f64 = 1_772_000_000.0;

    // AYEAYE-44 — up for 1000s, started 400s after boot, so it began 600s ago.
    #[test]
    fn a_start_time_is_boot_time_plus_the_twenty_second_field() {
        let got = start_time(
            &stat("bash", 400 * CLK_TCK),
            "1000.0 9876.54\n",
            CLK_TCK,
            NOW,
        );
        assert_eq!(got, Some(NOW - 600.0));
    }

    // AYEAYE-44 — the window a session is matched inside is seconds wide, so
    // the fraction is not noise.
    #[test]
    fn a_start_time_keeps_sub_second_resolution() {
        let ticks = 400 * CLK_TCK + CLK_TCK / 4;
        let got = start_time(&stat("bash", ticks), "1000.0 9876.54\n", CLK_TCK, NOW);
        assert_eq!(got, Some(NOW - 599.75));
    }

    // AYEAYE-44 — the name is whatever was exec'd, brackets and all, and a
    // split from the left reads its contents as fields.
    #[test]
    fn a_parenthesis_in_the_process_name_does_not_move_the_field() {
        let got = start_time(&stat("co)de(x", 100 * CLK_TCK), "1000.0\n", CLK_TCK, NOW);
        assert_eq!(got, Some(NOW - 900.0));
    }

    // AYEAYE-44 — a process that went away between the walk and the read.
    #[test]
    fn a_truncated_stat_line_has_no_start_time() {
        assert_eq!(
            start_time("42 (bash) S 1\n", "1000.0\n", CLK_TCK, NOW),
            None
        );
        assert_eq!(start_time("", "1000.0\n", CLK_TCK, NOW), None);
    }

    // AYEAYE-44 — a tick rate of zero is a machine that could not answer
    // `SC_CLK_TCK`; dividing by it would put the process at the end of time
    // rather than saying nothing is known.
    #[test]
    fn a_tick_rate_of_zero_has_no_start_time() {
        let line = stat("bash", 400 * CLK_TCK);
        assert_eq!(start_time(&line, "1000.0\n", 0, NOW), None);
    }

    // AYEAYE-44 — without a boot time the ticks mean nothing at all, and a
    // number measured from zero would put every process in 1970.
    #[test]
    fn an_unreadable_uptime_has_no_start_time() {
        let line = stat("bash", 400 * CLK_TCK);
        assert_eq!(start_time(&line, "", CLK_TCK, NOW), None);
        assert_eq!(start_time(&line, "not a number\n", CLK_TCK, NOW), None);
    }

    // AYEAYE-44 — the kernel's own answer to "who are this process's
    // children", which is what makes the Linux walk cost no scan of the
    // process table and no process of its own.
    #[test]
    fn the_children_file_is_a_line_of_pids() {
        assert_eq!(children("101 102 103 \n"), [101, 102, 103]);
        assert_eq!(children("\n"), []);
        assert_eq!(children(""), []);
    }

    // AYEAYE-44 — the fallback when the kernel publishes no children file:
    // field 4, found by the same split from the right, so a process called
    // `co)de(x` does not make somebody else its parent.
    #[test]
    fn the_parent_is_the_field_after_the_state() {
        assert_eq!(parent(&stat("bash", 0)), Some(1));
        assert_eq!(parent("900 (co)de(x) S 100 0 0\n"), Some(100));
        assert_eq!(parent("42 (bash) S\n"), None);
        assert_eq!(parent(""), None);
    }

    // AYEAYE-44 — this file is written while the processes it names are
    // starting and stopping, so a half-written word costs that pid and not
    // the whole answer.
    #[test]
    fn a_word_that_is_not_a_pid_costs_only_itself() {
        assert_eq!(children("101 nonsense 103\n"), [101, 103]);
    }

    /// A NUL-separated, NUL-terminated environment block, as the kernel holds
    /// it. The separator is the whole point: a value may contain spaces, and
    /// `SSH_CONNECTION` always does.
    fn environ(entries: &[&str]) -> Vec<u8> {
        let mut block = Vec::new();
        for entry in entries {
            block.extend_from_slice(entry.as_bytes());
            block.push(0);
        }
        block
    }

    // AYEAYE-44 — the value is "<client ip> <client port> <server ip> <server
    // port>" and only the first field is an address.
    #[test]
    fn the_peer_is_the_first_field_of_ssh_connection() {
        let block = environ(&["SSH_CONNECTION=100.101.102.103 54321 100.64.0.1 22"]);
        assert_eq!(ssh_peer(&block).as_deref(), Some("100.101.102.103"));
    }

    // AYEAYE-44 — a client sitting at the machine.
    #[test]
    fn an_environment_without_it_has_no_peer() {
        assert_eq!(ssh_peer(&environ(&["HOME=/home/someone"])), None);
        assert_eq!(ssh_peer(&[]), None);
    }

    // AYEAYE-44 — a variable whose name merely ends in it is not it, and the
    // wrong one here is an address the caller would go on to dial.
    #[test]
    fn a_variable_whose_name_merely_ends_in_it_is_not_it() {
        let block = environ(&[
            "OLD_SSH_CONNECTION=10.0.0.1 51000 10.0.0.2 22",
            "SSH_CLIENT=10.0.0.3 51000 22",
            "SSH_CONNECTION=100.101.102.103 54321 100.64.0.1 22",
        ]);
        assert_eq!(ssh_peer(&block).as_deref(), Some("100.101.102.103"));
    }

    // AYEAYE-44 — set but empty is not an address, and it is not the end of
    // the search either: this is the one question both platforms must answer
    // identically, and the module exists because two implementations of it is
    // how they stopped doing so.
    #[test]
    fn an_empty_value_does_not_end_the_search() {
        let block = environ(&[
            "SSH_CONNECTION=",
            "SSH_TTY=/dev/pts/4",
            "SSH_CONNECTION=10.0.0.9 1 2 3",
        ]);
        assert_eq!(ssh_peer(&block).as_deref(), Some("10.0.0.9"));
        assert_eq!(
            ssh_peer(&block),
            crate::process::ps::ssh_peer_in_command(
                "SSH_CONNECTION= SSH_TTY=/dev/pts/4 SSH_CONNECTION=10.0.0.9 1 2 3"
            ),
            "the two platforms answer this question with one answer or not at all"
        );
    }

    // AYEAYE-44 — set but empty is not an address.
    #[test]
    fn an_ssh_connection_with_no_fields_has_no_peer() {
        assert_eq!(
            ssh_peer(&environ(&["SSH_CONNECTION=", "SSH_TTY=/dev/pts/4"])),
            None
        );
    }

    // AYEAYE-44 — one undecodable variable somewhere in the block must not
    // cost the address in the next one.
    #[test]
    fn an_undecodable_environment_does_not_cost_the_peer() {
        let mut block = vec![b'O', b'D', b'D', b'=', 0xff, 0xfe, 0];
        block.extend(environ(&[
            "SSH_CONNECTION=100.101.102.103 54321 100.64.0.1 22",
        ]));
        assert_eq!(ssh_peer(&block).as_deref(), Some("100.101.102.103"));
    }
}
