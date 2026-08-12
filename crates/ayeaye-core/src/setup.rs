//! What setup would do to this machine, decided before any of it is done.
//!
//! The whole of `ayeaye setup`'s judgement, and none of its effects. A machine,
//! what is already here, and what was asked for go in; an ordered list of steps
//! comes out. That shape is what makes two of the ticket's acceptance criteria
//! checkable at all — *re-runnable and does not damage an existing install* is a
//! property of this list, and *records consent for anything with a privacy or
//! security consequence* is a property of which steps may be on it.
//!
//! # What setup asks
//!
//! Almost nothing, and this module is where that decision lives. The wizard's
//! conversation is not ported: the milestone says a conversational assistant
//! walks somebody through a judgement call better than a branching wizard does,
//! and the binary's job is to do the mechanical part and verify the result. So
//! setup asks a person exactly where the shell asks — before an act with a
//! consequence — and there are two of those:
//!
//! - **fetching a model**, which is bytes over the internet onto this disk;
//! - **enabling the service**, which is a program that runs whenever you log in.
//!
//! Minting the key, writing the settings file and *writing* the service
//! definition are not gated. All three land under directories the user already
//! owns, all three are idempotent, and AYEAYE-61 already settled that the
//! definition is the one thing setup does to the machine.
//!
//! Everything the old wizard configured and this does not — network exposure,
//! reverse proxies, mesh networking, agent command-line tools, the terminal
//! multiplexer — is detected and verified by [`crate::health`] and never
//! configured. That is not an omission; it is the milestone's decision, and the
//! reason those are health checks rather than steps.

use crate::machine::Machine;

/// Why a step needs saying yes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Consequence {
    /// It fetches something from the internet onto this disk.
    Network,
    /// It leaves a program running whenever this person logs in.
    RunsAtLogin,
}

impl Consequence {
    /// The question a person is actually being asked.
    pub fn question(self) -> &'static str {
        match self {
            Consequence::Network => "download it now?",
            Consequence::RunsAtLogin => "enable and start it now?",
        }
    }
}

/// One thing setup would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Generate the key that locks the page. Only when there is none: a key that
    /// already exists is kept, so a bookmark already on somebody's phone still
    /// works.
    MintKey,
    /// Write the settings file setup owns, merging rather than replacing.
    WriteSettings,
    /// Discover, smoke-test, and select speech and optional cleanup models.
    ChooseModels,
    /// Write the service definition.
    InstallService,
    /// Make the service start at login, and start it now.
    EnableService,
}

impl Step {
    /// Why this needs consent, or `None` when it does not.
    pub fn consequence(&self) -> Option<Consequence> {
        match self {
            Step::ChooseModels => Some(Consequence::Network),
            Step::EnableService => Some(Consequence::RunsAtLogin),
            Step::MintKey | Step::WriteSettings | Step::InstallService => None,
        }
    }

    /// What this step would do, said to a person before it is done.
    pub fn describe(&self) -> String {
        match self {
            Step::MintKey => "generate the key that locks the page".to_string(),
            Step::WriteSettings => "write the settings file".to_string(),
            Step::ChooseModels => "discover and choose models from the model hub".to_string(),
            Step::InstallService => "write the service definition".to_string(),
            Step::EnableService => "start ayeaye now, and whenever you log in".to_string(),
        }
    }

    /// The command that would take this step on its own.
    ///
    /// What somebody is told when they did not consent, so declining is never a
    /// dead end. `None` for the steps that need no consent, which are never
    /// declined.
    pub fn by_hand(&self) -> Option<String> {
        match self {
            Step::ChooseModels => Some("ayeaye model choose".to_string()),
            Step::EnableService => Some("ayeaye service enable".to_string()),
            _ => None,
        }
    }
}

/// What is already on this machine.
///
/// Everything here is a fact the shell has to go and look up, and every one of
/// them exists to keep a second run from undoing the first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Existing {
    /// A key is already here.
    pub key: bool,
}

/// A key, from random bytes, in the alphabet a URL can carry.
///
/// The same shape as the key `install.sh` mints with
/// `secrets.token_urlsafe(32)`, because the phone opens the page with the key in
/// a query string and a `+` or a `/` there is a different key by the time it
/// arrives. There is no padding for the same reason.
///
/// Pure, and the randomness is the caller's: where the bytes come from is a
/// decision about entropy sources and belongs in the crate that is allowed to
/// open `/dev/urandom`.
pub fn urlsafe(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for group in bytes.chunks(3) {
        let mut packed = 0u32;
        for (at, byte) in group.iter().enumerate() {
            packed |= u32::from(*byte) << (16 - 8 * at);
        }
        // One output character per 6 bits that any input byte contributed to,
        // and no padding: a key is a string, not a decodable payload.
        for at in 0..=group.len() {
            let index = (packed >> (18 - 6 * at)) & 0x3f;
            out.push(char::from(ALPHABET[index as usize]));
        }
    }
    out
}

/// What was agreed to.
///
/// Two answers and not one, because the shell asks two questions and they are
/// genuinely different decisions: somebody on a metered connection may well want
/// the service and not the download, and somebody on a shared machine the
/// reverse. Collapsing them into a single `--yes` would make the cautious answer
/// to either the cautious answer to both.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Consent {
    /// Fetching a model may go to the internet.
    pub network: bool,
    /// The service may be enabled and started.
    pub run_at_login: bool,
}

impl Consent {
    /// Yes to everything — what `--yes` means.
    pub fn all() -> Self {
        Consent {
            network: true,
            run_at_login: true,
        }
    }

    /// Whether this covers that consequence.
    pub fn allows(&self, consequence: Consequence) -> bool {
        match consequence {
            Consequence::Network => self.network,
            Consequence::RunsAtLogin => self.run_at_login,
        }
    }
}

/// What was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choices {
    /// Which consequential acts were agreed to.
    pub consent: Consent,
    /// Whether to install a service at all.
    pub service: bool,
    /// Whether to choose models.
    pub models: bool,
}

impl Default for Choices {
    fn default() -> Self {
        Choices {
            consent: Consent::default(),
            service: true,
            models: true,
        }
    }
}

/// What setup will do, and what it will not do without being asked again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// In the order they happen.
    pub steps: Vec<Step>,
    /// The consequential steps that were *not* consented to. Not an error and
    /// not a failure: they are printed with the command that would take each of
    /// them, so declining is never a dead end.
    pub declined: Vec<Step>,
}

impl Plan {
    /// Whether anything on this plan has a consequence.
    pub fn consequential(&self) -> bool {
        self.steps.iter().any(|step| step.consequence().is_some())
    }
}

/// Decide what setup does, from the machine, what is here, and what was asked.
///
/// The whole of "re-runnable and does not damage an existing install" is this
/// function: a key that exists is kept, a model already in the store is not
/// fetched again, and the service definition is written every time because
/// `service::plan_install` compares it and leaves an identical one alone — which
/// is the difference between repairing a service and disturbing one.
///
/// `has_manager` is [`crate::service::Session::for_manager`] having found one.
/// Where there is none, neither service step is planned and neither is declined:
/// running ayeaye by hand is a supported way to use it, and offering to enable a
/// service on a machine that has nowhere to put one would be an offer nobody can
/// take up.
pub fn plan(_machine: &Machine, has_manager: bool, existing: &Existing, choices: &Choices) -> Plan {
    let mut plan = Plan::default();

    if !existing.key {
        plan.steps.push(Step::MintKey);
    }
    plan.steps.push(Step::WriteSettings);

    if choices.models {
        gate(&mut plan, Step::ChooseModels, &choices.consent);
    }

    if has_manager && choices.service {
        plan.steps.push(Step::InstallService);
        gate(&mut plan, Step::EnableService, &choices.consent);
    }

    plan
}

/// Put a consequential step on the plan, or on the list of what was declined.
fn gate(plan: &mut Plan, step: Step, consent: &Consent) {
    let consequence = step
        .consequence()
        .expect("only a consequential step is gated");
    if consent.allows(consequence) {
        plan.steps.push(step);
    } else {
        plan.declined.push(step);
    }
}

#[cfg(test)]
mod tests {
    use super::{Choices, Consent, Consequence, Existing, Step, plan};
    use crate::machine::{Machine, Probes};

    /// The captured probe output, reaching this crate the only way it may.
    macro_rules! fixture {
        ($path:literal) => {
            include_str!(concat!("../../../tests/fixtures/", $path))
        };
    }

    fn a_machine(tier_maker: Probes<'_>) -> Machine {
        Machine::read(&tier_maker)
    }

    fn roomy() -> Machine {
        a_machine(Probes {
            os_release: Some(fixture!("os-release/ubuntu-24.04")),
            uname_s: Some("Linux"),
            uname_m: Some("x86_64"),
            meminfo: Some(fixture!("meminfo/64gb")),
            lscpu: Some(fixture!("lscpu/x86_64-8core")),
            df_pk: Some(fixture!("df/roomy")),
            nvidia_smi: Some(fixture!("nvidia-smi/rtx-4090")),
            ..Probes::default()
        })
    }

    // AYEAYE-62 — the two acts with a consequence, and the three without. This
    // is the answer to the milestone's load-bearing unknown, and it is asserted
    // rather than left in a comment.
    #[test]
    fn exactly_two_steps_have_a_consequence_worth_asking_about() {
        assert_eq!(Step::ChooseModels.consequence(), Some(Consequence::Network),);
        assert_eq!(
            Step::EnableService.consequence(),
            Some(Consequence::RunsAtLogin)
        );
        for harmless in [Step::MintKey, Step::WriteSettings, Step::InstallService] {
            assert_eq!(
                harmless.consequence(),
                None,
                "{harmless:?} lands under a directory the user already owns and is idempotent"
            );
        }
    }

    // AYEAYE-62 — with nothing here and consent given, setup does the lot, in an
    // order that puts the key before the settings that name it and the
    // definition before the enable that starts it.
    #[test]
    fn a_fresh_machine_with_consent_gets_the_whole_plan_in_order() {
        let made = plan(
            &roomy(),
            true,
            &Existing::default(),
            &Choices {
                consent: Consent::all(),
                ..Choices::default()
            },
        );
        assert_eq!(
            made.steps,
            vec![
                Step::MintKey,
                Step::WriteSettings,
                Step::ChooseModels,
                Step::InstallService,
                Step::EnableService,
            ]
        );
        assert!(made.declined.is_empty());
        assert!(made.consequential());
    }

    // AYEAYE-62 — without consent the two consequential steps do not happen, and
    // are not silently dropped either: each is reported with the command that
    // takes it, so declining is never a dead end. This is what makes setup safe
    // for AYEAYE-63's downloader to hand off to with no terminal attached.
    #[test]
    fn without_consent_nothing_consequential_happens_and_it_says_what_would() {
        let made = plan(&roomy(), true, &Existing::default(), &Choices::default());
        assert_eq!(
            made.steps,
            vec![Step::MintKey, Step::WriteSettings, Step::InstallService]
        );
        assert!(!made.consequential());
        assert_eq!(made.declined, vec![Step::ChooseModels, Step::EnableService]);
        for step in &made.declined {
            let by_hand = step.by_hand().expect("a way to take it later");
            assert!(by_hand.starts_with("ayeaye "), "{by_hand}");
        }
    }

    // AYEAYE-62 — the acceptance criterion: re-runnable, and does not damage an
    // existing install. A key already here is kept, because a bookmark already on
    // somebody's phone is logged in with it.
    #[test]
    fn a_second_run_keeps_the_key() {
        let already = Existing { key: true };
        let made = plan(
            &roomy(),
            true,
            &already,
            &Choices {
                consent: Consent::all(),
                ..Choices::default()
            },
        );
        assert!(!made.steps.contains(&Step::MintKey), "the key is kept");
        assert!(made.declined.is_empty(), "and nothing is owed either");
        // The definition is still written every time: service::plan_install
        // compares it and leaves an identical one alone, which is the difference
        // between repairing a service and disturbing one.
        // And the enable is planned again, because enabling a service that is
        // already enabled is idempotent — unlike its inverse, which is what
        // AYEAYE-61 did to this machine by hand.
        assert_eq!(
            made.steps,
            vec![
                Step::WriteSettings,
                Step::ChooseModels,
                Step::InstallService,
                Step::EnableService
            ]
        );
    }

    // AYEAYE-62 — the third answer, reaching this far. On a machine with no
    // service manager neither service step is planned *and neither is declined*:
    // offering to enable a service where there is nowhere to put one would be an
    // offer nobody can take up.
    #[test]
    fn a_machine_with_no_service_manager_is_not_offered_a_service() {
        let made = plan(
            &roomy(),
            false,
            &Existing::default(),
            &Choices {
                consent: Consent::all(),
                ..Choices::default()
            },
        );
        assert!(!made.steps.contains(&Step::InstallService));
        assert!(!made.steps.contains(&Step::EnableService));
        assert!(made.declined.is_empty());
        // Everything else still happens: this is a supported way to run ayeaye.
        assert!(made.steps.contains(&Step::MintKey));
        assert!(made.steps.contains(&Step::WriteSettings));
    }

    // AYEAYE-62 — and asking for no model, or no service, is taken as meant.
    #[test]
    fn asking_for_less_gets_less() {
        let made = plan(
            &roomy(),
            true,
            &Existing::default(),
            &Choices {
                consent: Consent::all(),
                service: false,
                models: false,
            },
        );
        assert_eq!(made.steps, vec![Step::MintKey, Step::WriteSettings]);
        assert!(made.declined.is_empty(), "not declined — never asked for");
    }

    // AYEAYE-62 — the two questions are two, and answering one does not answer
    // the other. Somebody on a metered connection may well want the service and
    // not the download; collapsing them would make the cautious answer to either
    // the cautious answer to both.
    #[test]
    fn the_two_consents_are_independent_of_each_other() {
        let network_only = plan(
            &roomy(),
            true,
            &Existing::default(),
            &Choices {
                consent: Consent {
                    network: true,
                    run_at_login: false,
                },
                ..Choices::default()
            },
        );
        assert!(network_only.steps.contains(&Step::ChooseModels));
        assert_eq!(network_only.declined, vec![Step::EnableService]);

        let service_only = plan(
            &roomy(),
            true,
            &Existing::default(),
            &Choices {
                consent: Consent {
                    network: false,
                    run_at_login: true,
                },
                ..Choices::default()
            },
        );
        assert!(service_only.steps.contains(&Step::EnableService));
        assert_eq!(service_only.declined, vec![Step::ChooseModels]);
    }

    // AYEAYE-62 — the key goes on the end of a URL, so it has to survive being
    // one. The alphabet is url-safe and there is no padding: `+`, `/` and `=` in
    // a query string are all a different key by the time the phone sends it back.
    #[test]
    fn a_minted_key_survives_being_put_in_a_url() {
        use super::urlsafe;
        assert_eq!(urlsafe(&[]), "");
        // The two bytes that produce `+` and `/` in the standard alphabet.
        assert_eq!(urlsafe(&[0xff, 0xef, 0xbf]), "_--_");
        assert_eq!(urlsafe(&[0, 0, 0]), "AAAA");
        // 32 bytes in, 43 characters out — the same as secrets.token_urlsafe(32).
        let key = urlsafe(&(0u8..32).collect::<Vec<u8>>());
        assert_eq!(key.len(), 43);
        for byte in 0u8..=255 {
            let said = urlsafe(&[byte, byte, byte]);
            assert!(
                said.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
                "{byte} produced {said}, which a URL would change"
            );
        }
        // Different bytes are different keys: an encoder that dropped the tail
        // would collide, and every colliding key is a shared secret that is not
        // secret.
        assert_ne!(urlsafe(&[1, 2, 3]), urlsafe(&[1, 2, 4]));
        assert_ne!(urlsafe(&[1, 2]), urlsafe(&[1, 3]));
        assert_ne!(urlsafe(&[1]), urlsafe(&[2]));
    }

    // AYEAYE-62 — every step says what it would do, before it is done. A plan
    // nobody can read is a plan nobody can consent to.
    #[test]
    fn every_step_can_say_what_it_would_do() {
        for step in [
            Step::MintKey,
            Step::WriteSettings,
            Step::ChooseModels,
            Step::InstallService,
            Step::EnableService,
        ] {
            let said = step.describe();
            assert!(!said.is_empty(), "{step:?} says nothing");
            assert!(
                said.chars().next().is_some_and(char::is_lowercase),
                "{said:?} is a clause in a list, not a sentence on its own"
            );
        }
        assert_eq!(
            Step::ChooseModels.by_hand().as_deref(),
            Some("ayeaye model choose")
        );
    }
}
