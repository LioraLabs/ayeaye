//! Did what was set up actually work?
//!
//! A port of `lib/steps/80-health.sh`, which is the last thing setup does before
//! it tells somebody they are finished. Every check here is a pure function from
//! *evidence* to a [`Verdict`]: an HTTP status, a command's exit status, a name
//! on `PATH`. Making the request is the shell's; deciding what it means is this.
//!
//! # Four marks, not three
//!
//! The ticket asks for three outcomes — passed, failed, and could-not-run — and
//! this delivers four, because the shell has four and a port is never less
//! honest than what it replaces:
//!
//! ```text
//!   ok        it was checked and it works
//!   FAILED    it was checked and it does not
//!   skipped   it is not part of what you asked for, so there was nothing to check
//!   unknown   it is part of what you asked for and setup could not tell
//! ```
//!
//! They are never collapsed into fewer. A check that was skipped because nobody
//! asked for it and a check that passed are different facts, and a closing
//! screen that renders them the same way is how somebody ends up believing their
//! phone can reach their agents when nothing was ever set up. `unknown` is the
//! fourth because "setup has no curl" is not a passing grade either, and calling
//! it one would be the worst answer available.
//!
//! # The one assertion
//!
//! Every check here measures something except one, which *asserts*: a request
//! carrying no key must be refused. A 200 there means anybody who can reach the
//! address can drive the coding agents on this computer, and it is the single
//! verdict in the whole step that fails it outright rather than leaving it
//! unfinished.

use crate::machine::{Acceleration, Machine, Usability};

/// What became of one check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Checked, and it works.
    Passed,
    /// Checked, and it does not.
    Failed,
    /// Not part of what was asked for, so there was nothing to check.
    Skipped,
    /// Part of what was asked for, and setup could not tell.
    Unknown,
}

impl Verdict {
    /// The mark that goes in front of the line, as the shell prints it.
    pub fn mark(self) -> &'static str {
        match self {
            Verdict::Passed => "ok",
            Verdict::Failed => "FAILED",
            Verdict::Skipped => "skipped",
            Verdict::Unknown => "unknown",
        }
    }

    /// The trailing half-sentence that makes a mark mean something on its own.
    ///
    /// Two of the four say nothing: "ok" and "FAILED" are complete. The other
    /// two are the ones that get mistaken for each other, so neither is ever
    /// printed bare.
    pub fn because(self) -> Option<&'static str> {
        match self {
            Verdict::Passed | Verdict::Failed => None,
            Verdict::Skipped => Some("you did not ask for this"),
            Verdict::Unknown => Some("setup could not check this"),
        }
    }
}

/// One check, its verdict, and what it was about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    /// The short name it is remembered under.
    pub name: &'static str,
    /// What was being checked, in words somebody reads once.
    pub claim: String,
    /// What became of it.
    pub verdict: Verdict,
    /// The evidence, for whoever has to fix it. Never shown as the verdict.
    pub detail: Option<String>,
}

impl Check {
    /// One line: the mark, the claim, and the qualification a mark needs.
    pub fn line(&self) -> String {
        match self.verdict.because() {
            Some(because) => format!("{:>7}  {} — {because}", self.verdict.mark(), self.claim),
            None => format!("{:>7}  {}", self.verdict.mark(), self.claim),
        }
    }
}

/// How the whole step came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Everything asked for was checked and works.
    Done,
    /// Something did not work, or could not be checked. Not a reason to stop:
    /// the shell answers PENDING here, and a pending step never fails a run.
    Unfinished,
    /// The lock is off. The one outcome that fails outright, and the reason this
    /// step is not optional.
    Insecure,
}

/// Everything that was checked, and what it came to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// In the order they were checked.
    pub checks: Vec<Check>,
    /// Whether an unauthenticated request was answered in full.
    pub insecure: bool,
}

impl Report {
    /// Record one check.
    pub fn record(&mut self, check: Check) {
        self.checks.push(check);
    }

    /// Record the one check that can fail the whole step, from its evidence.
    ///
    /// The verdict and the alarm come from one call because they are one
    /// decision. Recording the check and setting [`Report::insecure`] as two
    /// steps is a step that can be forgotten, and forgetting it turns "anybody
    /// who can reach this address can run commands on this computer" into
    /// "something is unfinished" — the distinction the module doc calls one exit
    /// code wide.
    pub fn record_auth(&mut self, claim: &str, code: Option<u16>) -> Verdict {
        let (verdict, insecure) = unauthenticated(code);
        self.insecure |= insecure;
        self.record(Check {
            name: "auth",
            claim: claim.to_string(),
            verdict,
            detail: code.map(|code| format!("an unauthenticated request answered {code}")),
        });
        verdict
    }

    /// How many came out that way.
    pub fn count(&self, verdict: Verdict) -> usize {
        self.checks
            .iter()
            .filter(|check| check.verdict == verdict)
            .count()
    }

    /// The one line at the bottom.
    pub fn summary(&self) -> String {
        format!(
            "{} ok, {} failed, {} skipped, {} unknown",
            self.count(Verdict::Passed),
            self.count(Verdict::Failed),
            self.count(Verdict::Skipped),
            self.count(Verdict::Unknown),
        )
    }

    /// What the whole step came to.
    ///
    /// Everything this step can be unhappy about answers [`Outcome::Unfinished`]
    /// — a capability that did not work, a check that could not be made — and
    /// only the lock being off answers [`Outcome::Insecure`]. That distinction is
    /// one exit code wide and it is the whole reason the step exists.
    pub fn outcome(&self) -> Outcome {
        if self.insecure {
            return Outcome::Insecure;
        }
        if self.count(Verdict::Failed) > 0 || self.count(Verdict::Unknown) > 0 {
            return Outcome::Unfinished;
        }
        Outcome::Done
    }
}

/// What to say when an unauthenticated request was answered in full.
///
/// Worth saying at length, and worth saying in the second person. Anybody who
/// can reach that address can run commands on this computer, which is the worst
/// thing this project can do to somebody.
pub fn insecure_warning(url: &str) -> Vec<String> {
    vec![
        "STOP. Something answered a request that carried no key, and".to_string(),
        format!("answered it in full. Whatever is on {url} can be"),
        "driven by anybody who can reach that address, and driving".to_string(),
        "ayeaye means running commands on this computer.".to_string(),
        "Either something else is listening on that port, or the key".to_string(),
        "has been switched off. Do not open this from your phone until".to_string(),
        "you know which.".to_string(),
    ]
}

/// Is the service running?
///
/// Four answers, as `_health_check_service` has four, and they are not three:
///
/// - **no manager** (`has_manager` false) — skipped. A machine with no user
///   service manager is not a machine with a broken service; ayeaye started by
///   hand is a supported way to use it.
/// - **a manager, and no command to run** (`asked` is `None`) — unknown. This is
///   reachable: a Mac whose `id` would not answer has a launchd and no domain to
///   address, so `Session::command` refuses. Something was asked for and setup
///   could not tell, which is not the same as nothing having been asked for.
/// - **the command ran** — passed or failed.
pub fn service(has_manager: bool, asked: Option<bool>) -> Verdict {
    match (has_manager, asked) {
        (false, _) => Verdict::Skipped,
        (true, None) => Verdict::Unknown,
        (true, Some(true)) => Verdict::Passed,
        (true, Some(false)) => Verdict::Failed,
    }
}

/// What a passing service check is actually worth, which differs by platform.
///
/// `systemctl --user status` succeeds only for a unit that is *active*.
/// `launchctl print` succeeds for a job launchd has merely loaded, which
/// includes one that started, crashed and has not been retried. Claiming "and is
/// running now" from that would be claiming more than the command answered;
/// whether ayeaye is really up is [`local`]'s business, and it asks over HTTP.
pub fn service_claim(launchd: bool) -> &'static str {
    if launchd {
        "ayeaye is registered to start when you log in"
    } else {
        "ayeaye starts when you log in, and is running now"
    }
}

/// Does the program answer on this computer at all?
///
/// `None` is no answer of any kind — no curl, or nothing listening — and it is
/// never a pass.
pub fn local(code: Option<u16>) -> Verdict {
    match code {
        None => Verdict::Unknown,
        Some(code) if (200..400).contains(&code) => Verdict::Passed,
        Some(_) => Verdict::Failed,
    }
}

/// Is the lock on?
///
/// The one check that asserts rather than measures, and the only one that can
/// make the whole step fail. The four answers are genuinely four:
///
/// - **401** — refused for the right reason. The lock is on.
/// - **403** — refused for the *other* reason, most likely the host gate. The
///   lock may well be on; this request never got far enough to find out, so the
///   honest answer is that setup could not tell.
/// - **2xx** — answered in full, with no key. The lock is off.
/// - anything else — not a refusal, so not a pass.
pub fn unauthenticated(code: Option<u16>) -> (Verdict, bool) {
    match code {
        None => (Verdict::Unknown, false),
        Some(401) => (Verdict::Passed, false),
        Some(403) => (Verdict::Unknown, false),
        Some(code) if (200..300).contains(&code) => (Verdict::Failed, true),
        Some(_) => (Verdict::Failed, false),
    }
}

/// Host validation, asserted in both directions.
///
/// `configured` is one status per address that was configured, and `stranger` is
/// the status for an address nobody configured. **Only the second half proves
/// anything**: a server that accepts everything passes the first half perfectly.
///
/// `None` anywhere is a request that could not be made, and the whole check
/// becomes unknown — a partial answer to a question about what is refused is not
/// an answer.
pub fn hosts(configured: &[Option<u16>], stranger: Option<u16>) -> Verdict {
    if configured.is_empty() {
        return Verdict::Skipped;
    }
    if configured.iter().any(Option::is_none) || stranger.is_none() {
        return Verdict::Unknown;
    }
    let own_addresses_accepted = configured.iter().flatten().all(|code| *code != 403);
    let stranger_refused = stranger == Some(403);
    if own_addresses_accepted && stranger_refused {
        Verdict::Passed
    } else {
        Verdict::Failed
    }
}

/// The https front end, whichever one is in place.
///
/// Detected and verified, never configured — a reverse proxy, a mesh network and
/// a LAN certificate are judgement calls this binary has no business making, and
/// its job is to say whether the result answers. A refusal counts as an answer:
/// without a key ayeaye is supposed to refuse, and a refusal proves the address
/// reaches it.
pub fn https(asked_for: bool, host: Option<&str>, code: Option<u16>) -> Verdict {
    if !asked_for {
        return Verdict::Skipped;
    }
    // Asked for, and setup cannot tell which address to try. Not "you did not
    // ask for this": somebody configured a front end and this could not check
    // it, which is the floor of the whole step broken from the other side.
    if host.is_none_or(str::is_empty) {
        return Verdict::Unknown;
    }
    match code {
        None => Verdict::Unknown,
        // A refusal counts. Without a key ayeaye is supposed to refuse, and a
        // refusal proves the address reaches it — `_health_is_answer` is
        // `2*|3*|401|403` and this is that list.
        Some(code) if (200..400).contains(&code) || code == 401 || code == 403 => Verdict::Passed,
        Some(_) => Verdict::Failed,
    }
}

/// The coding agents. Only the ones that are here; the others are not missing.
///
/// Detected and verified, never installed. `wanted` is empty when nothing was
/// asked for, which is skipped and not a failure.
pub fn agents(wanted: &[(&str, bool)]) -> Verdict {
    if wanted.is_empty() {
        return Verdict::Skipped;
    }
    if wanted.iter().all(|(_, present)| *present) {
        Verdict::Passed
    } else {
        Verdict::Failed
    }
}

/// The terminal multiplexer, without which nothing works.
///
/// Verified and never installed: the acceptance criterion is that setup prints
/// the exact command for this platform, and `machine::packages` is what produces
/// it. A missing tmux is a real failure and not an unknown — the question was
/// answered, and the answer was no.
pub fn tmux(present: bool) -> Verdict {
    if present {
        Verdict::Passed
    } else {
        Verdict::Failed
    }
}

/// What this build can use of what this machine has.
///
/// A CPU build on a machine with a usable card is the case this exists for: it
/// works perfectly, and the only other symptom is inference being mysteriously
/// slow, which nobody diagnoses. So it is said out loud, and it is said as
/// [`Verdict::Failed`] — the machine has a capability the running binary is not
/// using, which is a thing to act on.
///
/// An unusable card is not this machine's fault and not a failure of it. AMD is
/// the case that matters: `machine::tier` detects and names the card and reports
/// [`Usability::Unsupported`] because candle has no ROCm backend, and this
/// repeats that verdict in its own words rather than inventing a cheerier one.
pub fn acceleration(build: Acceleration, machine: &Machine) -> Check {
    let (verdict, claim, detail) = match (machine.usability(), machine.acceleration()) {
        // Usable acceleration this build was not compiled for. One arm for both
        // kinds, because the fact is the same fact and the card names itself.
        (Usability::Usable, found) if found != build => (
            Verdict::Failed,
            "this build is using the graphics card in this machine".to_string(),
            Some(format!(
                "there is a usable {} here and this is a {} build, so \
                 transcription runs on the processor and will be slow",
                machine.gpu_name().unwrap_or(found.as_str()),
                build.as_str()
            )),
        ),
        (Usability::Usable, _) => (
            Verdict::Passed,
            format!("transcription runs on {}", describe(machine)),
            None,
        ),
        // Named, and declined, with the reason said plainly. Not a failure of
        // this machine: nothing here is broken and nothing can be done about it.
        (Usability::Unsupported(why), _) => (
            Verdict::Skipped,
            format!("transcription runs on the processor: {why}"),
            machine
                .gpu_name()
                .map(|name| format!("the card is a {name}")),
        ),
        // A card that would not say how big it is. Not a kind of TooSmall —
        // telling somebody their card is too small when what happened is that a
        // command did not answer is a lie about their hardware — and the one
        // place in this module where "setup could not tell" is about the machine
        // rather than about a request.
        (Usability::Unsized, _) => (
            Verdict::Unknown,
            "which processor transcription will run on".to_string(),
            Some(format!(
                "there is a {} here and it would not say how much memory it has",
                machine.gpu_name().unwrap_or("graphics card")
            )),
        ),
        (Usability::TooSmall, _) => (
            Verdict::Skipped,
            "transcription runs on the processor: the graphics card is too small to be any use"
                .to_string(),
            machine
                .gpu_name()
                .map(|name| format!("the card is a {name}")),
        ),
        (Usability::None, _) => (
            Verdict::Passed,
            "transcription runs on the processor".to_string(),
            None,
        ),
    };
    Check {
        name: "acceleration",
        claim,
        verdict,
        detail,
    }
}

/// What is doing the work, named.
fn describe(machine: &Machine) -> String {
    match machine.gpu_name() {
        Some(name) => name.to_string(),
        None => match machine.acceleration() {
            Acceleration::Cuda => "the NVIDIA card".to_string(),
            Acceleration::Metal => "this Mac's graphics".to_string(),
            Acceleration::Rocm => "the AMD card".to_string(),
            Acceleration::Cpu => "the processor".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Check, Outcome, Report, Verdict, acceleration, agents, hosts, https, insecure_warning,
        local, service, service_claim, tmux, unauthenticated,
    };
    use crate::machine::{Acceleration, Machine, Probes, Usability};

    /// The captured probe output, reaching this crate the only way it may.
    ///
    /// `machine::fixture!` cannot be reused: the relative path in it resolves
    /// against the file the macro is expanded in, and every module using that
    /// one sits a directory below `src/` while this sits in it.
    macro_rules! fixture {
        ($path:literal) => {
            include_str!(concat!("../../../tests/fixtures/", $path))
        };
    }

    fn check(name: &'static str, verdict: Verdict) -> Check {
        Check {
            name,
            claim: "something".to_string(),
            verdict,
            detail: None,
        }
    }

    // AYEAYE-62 — the four marks, which the whole step exists to keep apart.
    // Three of them are the ticket's three; the fourth is "you did not ask for
    // this", and collapsing it into "setup could not tell" would report a
    // capability nobody wanted as one setup failed to check.
    #[test]
    fn the_four_marks_are_four_and_none_reads_as_another() {
        assert_eq!(Verdict::Passed.mark(), "ok");
        assert_eq!(Verdict::Failed.mark(), "FAILED");
        assert_eq!(Verdict::Skipped.mark(), "skipped");
        assert_eq!(Verdict::Unknown.mark(), "unknown");
        let marks = [
            Verdict::Passed.mark(),
            Verdict::Failed.mark(),
            Verdict::Skipped.mark(),
            Verdict::Unknown.mark(),
        ];
        for (at, mark) in marks.iter().enumerate() {
            assert!(
                !marks[at + 1..].contains(mark),
                "two verdicts print the same mark: {mark}"
            );
        }
        // The two that get mistaken for each other are never printed bare.
        assert_eq!(Verdict::Skipped.because(), Some("you did not ask for this"));
        assert_eq!(
            Verdict::Unknown.because(),
            Some("setup could not check this")
        );
        assert_eq!(Verdict::Passed.because(), None);
        assert!(
            check("x", Verdict::Unknown)
                .line()
                .contains("could not check")
        );
        assert!(check("x", Verdict::Skipped).line().contains("did not ask"));
    }

    // AYEAYE-62 — the acceptance criterion, stated as the property rather than
    // as a case: a check that could not run is never rendered as one that
    // passed, whichever way it is looked at.
    #[test]
    fn a_check_that_could_not_run_is_never_a_check_that_passed() {
        let mut report = Report::default();
        report.record(check("a", Verdict::Unknown));
        report.record(check("b", Verdict::Skipped));
        assert_eq!(report.count(Verdict::Passed), 0);
        assert_eq!(report.outcome(), Outcome::Unfinished);
        assert_eq!(report.summary(), "0 ok, 0 failed, 1 skipped, 1 unknown");
    }

    // AYEAYE-62 — a skipped check is finished business. A run made of nothing
    // but "you did not ask for that" is a run with nothing left to do, and
    // reporting it as unfinished would nag somebody forever about a machine that
    // is fine.
    #[test]
    fn a_run_of_nothing_but_skips_is_finished() {
        let mut report = Report::default();
        report.record(check("a", Verdict::Skipped));
        report.record(check("b", Verdict::Passed));
        assert_eq!(report.outcome(), Outcome::Done);
    }

    // AYEAYE-62 — the one assertion in the whole step, and its four answers.
    // 200 is the worst thing this project can do to somebody: whatever is on
    // that address can be driven by anybody who can reach it.
    #[test]
    fn a_request_with_no_key_must_be_refused_and_a_200_is_the_alarm() {
        assert_eq!(unauthenticated(Some(401)), (Verdict::Passed, false));
        // Refused, but for the other reason: this request never got far enough
        // to find out whether the lock is on.
        assert_eq!(unauthenticated(Some(403)), (Verdict::Unknown, false));
        assert_eq!(unauthenticated(Some(200)), (Verdict::Failed, true));
        assert_eq!(unauthenticated(Some(204)), (Verdict::Failed, true));
        assert_eq!(unauthenticated(Some(500)), (Verdict::Failed, false));
        // A redirect is not "answered it in full". The shell's window is `2*`
        // and this is that window: widening it would raise the loudest alarm
        // this program has over a 302, which is how an alarm stops being
        // believed.
        assert_eq!(unauthenticated(Some(302)), (Verdict::Failed, false));
        assert_eq!(unauthenticated(Some(399)), (Verdict::Failed, false));
        assert_eq!(unauthenticated(Some(299)), (Verdict::Failed, true));
        assert_eq!(unauthenticated(None), (Verdict::Unknown, false));
    }

    // AYEAYE-62 — and it is the one verdict that fails the step outright.
    // Everything else this step can be unhappy about leaves work outstanding;
    // that distinction is one exit code wide.
    #[test]
    fn the_lock_being_off_is_the_only_thing_that_fails_the_step() {
        let mut unfinished = Report::default();
        unfinished.record(check("a", Verdict::Failed));
        unfinished.record(check("b", Verdict::Unknown));
        assert_eq!(unfinished.outcome(), Outcome::Unfinished);

        // Through the one door, so the verdict and the alarm cannot come apart.
        let mut insecure = Report::default();
        assert_eq!(
            insecure.record_auth("the lock is on", Some(200)),
            Verdict::Failed
        );
        assert_eq!(insecure.outcome(), Outcome::Insecure);
        assert!(
            insecure.checks[0].detail.is_some(),
            "the status is the evidence"
        );

        let mut locked = Report::default();
        assert_eq!(
            locked.record_auth("the lock is on", Some(401)),
            Verdict::Passed
        );
        assert_eq!(locked.outcome(), Outcome::Done);

        let mut unasked = Report::default();
        assert_eq!(
            unasked.record_auth("the lock is on", None),
            Verdict::Unknown
        );
        assert_eq!(unasked.outcome(), Outcome::Unfinished, "and never Done");
        assert!(
            insecure_warning("http://127.0.0.1:8912")
                .join(" ")
                .contains("http://127.0.0.1:8912")
        );
    }

    // AYEAYE-62 — no answer at all is never a pass. "setup has no curl" is not a
    // passing grade, and calling it one would be the worst answer available.
    #[test]
    fn no_answer_at_all_is_unknown_and_not_a_pass() {
        assert_eq!(local(None), Verdict::Unknown);
        assert_eq!(local(Some(200)), Verdict::Passed);
        assert_eq!(local(Some(302)), Verdict::Passed);
        assert_eq!(
            local(Some(401)),
            Verdict::Failed,
            "the door answered, and this check is about the page"
        );
        assert_eq!(local(Some(502)), Verdict::Failed);
    }

    // AYEAYE-62 — only the second half of the host check proves anything. A
    // server that accepts everything passes the first half perfectly, which is
    // exactly the bug the check exists to catch.
    #[test]
    fn a_server_that_accepts_everything_fails_the_host_check() {
        assert_eq!(hosts(&[Some(200)], Some(403)), Verdict::Passed);
        assert_eq!(
            hosts(&[Some(200)], Some(200)),
            Verdict::Failed,
            "an address nobody configured was accepted"
        );
        assert_eq!(
            hosts(&[Some(403)], Some(403)),
            Verdict::Failed,
            "it refused its own configured address"
        );
        assert_eq!(hosts(&[Some(200), Some(403)], Some(403)), Verdict::Failed);
        assert_eq!(hosts(&[], Some(403)), Verdict::Skipped);
    }

    // AYEAYE-62 — a partial answer to a question about what is refused is not an
    // answer. One request that could not be made makes the whole check unknown,
    // rather than a pass over whichever half did answer.
    #[test]
    fn a_host_check_that_could_not_be_finished_is_unknown() {
        assert_eq!(hosts(&[None], Some(403)), Verdict::Unknown);
        assert_eq!(hosts(&[Some(200)], None), Verdict::Unknown);
        assert_eq!(hosts(&[Some(200), None], Some(403)), Verdict::Unknown);
    }

    // AYEAYE-62 — the front end is detected and verified, never configured, and
    // a refusal counts as an answer: without a key ayeaye is supposed to refuse,
    // and a refusal proves the address reaches it.
    #[test]
    fn an_https_front_end_is_verified_and_a_refusal_proves_it_answers() {
        // Nobody asked for a front end. The mode that keeps ayeaye on this
        // computer has none by design.
        assert_eq!(https(false, None, Some(200)), Verdict::Skipped);
        assert_eq!(
            https(false, Some("box.example"), Some(200)),
            Verdict::Skipped
        );
        // Asked for, and there is no address to try. `_health_check_https`
        // answers unknown here, and it matters: somebody *did* ask for this, and
        // rendering it as "you did not ask" is the criterion's floor broken from
        // the other side.
        assert_eq!(https(true, None, Some(200)), Verdict::Unknown);
        assert_eq!(https(true, Some(""), Some(200)), Verdict::Unknown);

        assert_eq!(https(true, Some("box.example"), Some(200)), Verdict::Passed);
        assert_eq!(https(true, Some("box.example"), Some(302)), Verdict::Passed);
        // Both refusals count, and for the same reason: `_health_is_answer` is
        // `2*|3*|401|403`, because a refusal proves the address reaches ayeaye.
        // A reverse proxy answering 403 to an unauthenticated request is a
        // working front end, not a broken one.
        assert_eq!(https(true, Some("box.example"), Some(401)), Verdict::Passed);
        assert_eq!(https(true, Some("box.example"), Some(403)), Verdict::Passed);
        assert_eq!(https(true, Some("box.example"), Some(502)), Verdict::Failed);
        assert_eq!(https(true, Some("box.example"), Some(404)), Verdict::Failed);
        assert_eq!(https(true, Some("box.example"), None), Verdict::Unknown);
    }

    // AYEAYE-62 — a machine with no service manager is skipped and not failed:
    // ayeaye started by hand is a supported way to use it, which is the whole of
    // what AYEAYE-61 left for this ticket.
    #[test]
    fn a_machine_with_no_service_manager_is_skipped_and_not_failed() {
        assert_eq!(service(false, None), Verdict::Skipped);
        assert_eq!(
            service(false, Some(false)),
            Verdict::Skipped,
            "there is no manager, so nothing it might have answered matters"
        );
        assert_eq!(service(true, Some(true)), Verdict::Passed);
        assert_eq!(service(true, Some(false)), Verdict::Failed);
        // A manager, and no command to run against it. Reachable: a Mac whose
        // `id` will not answer has a launchd and no domain to address, so
        // Session::command refuses. Something was asked for and setup could not
        // tell — which is not the same fact as nothing having been asked for.
        assert_eq!(service(true, None), Verdict::Unknown);
    }

    // AYEAYE-62 — and what a pass is worth differs by platform, so the sentence
    // does too. `launchctl print` succeeds for a job that started, crashed and
    // has not been retried.
    #[test]
    fn the_service_claim_says_only_what_the_command_answered() {
        assert!(service_claim(false).contains("is running now"));
        assert!(!service_claim(true).contains("is running now"));
        assert!(service_claim(true).contains("registered"));
    }

    // AYEAYE-62 — the agents are detected and verified, never installed, and a
    // machine that was not asked for one has nothing missing.
    #[test]
    fn only_the_agents_that_are_here_are_checked_and_none_is_installed() {
        assert_eq!(agents(&[]), Verdict::Skipped);
        assert_eq!(agents(&[("claude", true)]), Verdict::Passed);
        assert_eq!(
            agents(&[("claude", true), ("codex", false)]),
            Verdict::Failed
        );
    }

    // AYEAYE-62 — tmux is a real answer either way. The question was asked and
    // answered, so a missing one is a failure and not an unknown.
    #[test]
    fn a_missing_multiplexer_is_a_failure_and_not_an_unknown() {
        assert_eq!(tmux(true), Verdict::Passed);
        assert_eq!(tmux(false), Verdict::Failed);
    }

    // AYEAYE-62 — the acceptance criterion in its own words: a CPU build running
    // on a machine with a usable NVIDIA card says so, because the only other
    // symptom is inference being mysteriously slow.
    #[test]
    fn a_cpu_build_on_a_machine_with_a_usable_card_says_so() {
        let machine = Machine::read(&Probes {
            os_release: Some(fixture!("os-release/ubuntu-24.04")),
            uname_s: Some("Linux"),
            uname_m: Some("x86_64"),
            meminfo: Some(fixture!("meminfo/64gb")),
            lscpu: Some(fixture!("lscpu/x86_64-8core")),
            df_pk: Some(fixture!("df/roomy")),
            nvidia_smi: Some(fixture!("nvidia-smi/rtx-4090")),
            ..Probes::default()
        });
        assert_eq!(machine.usability(), Usability::Usable);

        let said = acceleration(Acceleration::Cpu, &machine);
        assert_eq!(said.verdict, Verdict::Failed);
        let detail = said.detail.expect("it must name the card");
        assert!(detail.contains("NVIDIA GeForce RTX 4090"), "{detail}");
        assert!(detail.contains("slow"), "{detail}");

        // The same machine, built for it, has nothing to report.
        let matched = acceleration(Acceleration::Cuda, &machine);
        assert_eq!(matched.verdict, Verdict::Passed);
        assert!(matched.claim.contains("RTX 4090"), "{}", matched.claim);

        // And a build compiled for the *other* card is just as wrong as a
        // processor build: what matters is that the acceleration this machine
        // has is not the acceleration this binary can use.
        let mismatched = acceleration(Acceleration::Metal, &machine);
        assert_eq!(mismatched.verdict, Verdict::Failed);
        assert!(
            mismatched
                .detail
                .is_some_and(|detail| detail.contains("metal build")),
            "it must name the build, or nobody knows which artifact to replace"
        );
    }

    // AYEAYE-62 — the same criterion the other way round: an Apple machine
    // whose build cannot use its graphics.
    #[test]
    fn a_cpu_build_on_an_apple_machine_says_so_too() {
        let machine = Machine::read(&Probes {
            uname_s: Some("Darwin"),
            uname_m: Some("arm64"),
            sw_vers: Some(fixture!("sw_vers/macos-15.1")),
            system_profiler: Some(fixture!("system_profiler/apple-m3-24gb")),
            df_pk: Some(fixture!("df/roomy")),
            ..Probes::default()
        });
        assert_eq!(machine.acceleration(), Acceleration::Metal);
        assert_eq!(
            acceleration(Acceleration::Cpu, &machine).verdict,
            Verdict::Failed
        );
        assert_eq!(
            acceleration(Acceleration::Metal, &machine).verdict,
            Verdict::Passed
        );
    }

    // AYEAYE-62 — a card that would not say how big it is. Not a kind of "too
    // small": telling somebody their card is too small when what happened is
    // that a command did not answer is a lie about their hardware. This is the
    // one place in the module where "setup could not tell" is about the machine
    // rather than about a request.
    #[test]
    fn a_card_that_would_not_say_its_size_is_unknown_and_not_declined() {
        let machine = Machine::read(&Probes {
            os_release: Some(fixture!("os-release/debian-12")),
            uname_s: Some("Linux"),
            uname_m: Some("x86_64"),
            meminfo: Some(fixture!("meminfo/64gb")),
            lscpu: Some(fixture!("lscpu/x86_64-8core")),
            df_pk: Some(fixture!("df/roomy")),
            nvidia_smi: Some("NVIDIA GeForce RTX 4090, [N/A]\n"),
            ..Probes::default()
        });
        assert_eq!(machine.usability(), Usability::Unsized);
        let said = acceleration(Acceleration::Cpu, &machine);
        assert_eq!(said.verdict, Verdict::Unknown);
        assert!(
            said.detail
                .as_deref()
                .is_some_and(|detail| detail.contains("would not say")),
            "{said:?}"
        );
    }

    // AYEAYE-62 — AMD, which is AYEAYE-60's deliberate departure carried
    // through: the card is real, it is named, and ayeaye cannot use it because
    // candle has no ROCm backend. That is not a failure of this machine and
    // nothing can be done about it, so it is skipped rather than failed — and it
    // is never reported as a card being used.
    #[test]
    fn an_amd_card_is_named_and_declined_in_the_detectors_own_words() {
        let machine = Machine::read(&Probes {
            os_release: Some(fixture!("os-release/arch")),
            uname_s: Some("Linux"),
            uname_m: Some("x86_64"),
            meminfo: Some(fixture!("meminfo/64gb")),
            lscpu: Some(fixture!("lscpu/x86_64-8core")),
            df_pk: Some(fixture!("df/roomy")),
            rocminfo: Some(fixture!("rocminfo/gfx1100")),
            ..Probes::default()
        });
        let said = acceleration(Acceleration::Cpu, &machine);
        assert_eq!(said.verdict, Verdict::Skipped);
        assert!(
            said.claim
                .contains("ayeaye cannot use an AMD graphics card"),
            "the detector's own words, not a cheerier invention: {}",
            said.claim
        );
        assert!(
            said.detail
                .as_deref()
                .is_some_and(|detail| detail.contains("AMD Radeon RX 7900 XTX")),
            "somebody who paid for the card is told it was seen"
        );
    }

    // AYEAYE-62 — a card too small to be any use is named too, for the same
    // reason: saying "there is no graphics card here" to somebody who paid for
    // one is how a tool stops being believed about anything else.
    #[test]
    fn a_card_too_small_to_use_is_still_named() {
        let machine = Machine::read(&Probes {
            os_release: Some(fixture!("os-release/debian-12")),
            uname_s: Some("Linux"),
            uname_m: Some("x86_64"),
            meminfo: Some(fixture!("meminfo/64gb")),
            lscpu: Some(fixture!("lscpu/x86_64-8core")),
            df_pk: Some(fixture!("df/roomy")),
            nvidia_smi: Some(fixture!("nvidia-smi/gtx-1050")),
            ..Probes::default()
        });
        let said = acceleration(Acceleration::Cpu, &machine);
        assert_eq!(said.verdict, Verdict::Skipped);
        assert!(said.claim.contains("too small"), "{}", said.claim);
        assert!(
            said.detail
                .as_deref()
                .is_some_and(|detail| detail.contains("GTX 1050"))
        );
    }

    // AYEAYE-62 — a machine with no card at all has nothing to report and is not
    // nagged about it.
    #[test]
    fn a_machine_with_no_card_is_not_told_it_is_missing_one() {
        let said = acceleration(Acceleration::Cpu, &Machine::read(&Probes::default()));
        assert_eq!(said.verdict, Verdict::Passed);
        assert_eq!(said.detail, None);
    }
}
