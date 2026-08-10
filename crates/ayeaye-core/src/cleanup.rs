//! What a cleanup pass is allowed to be, and what it is allowed to return.
//!
//! Cleanup is the one step in a dictation that can make things worse. Speech
//! to text can only mishear; a language model asked to tidy a sentence can
//! answer it, explain it, refuse it, or write an essay about it, and whatever
//! it writes is what gets typed into somebody's terminal. The milestone's rule
//! is that raw text beats no text, so this module is the shape that rule takes:
//! [`settle`] is total, [`Cleaned`] always carries text, and a rewrite has to be
//! earned.

use crate::chat::Template;

/// The instruction a cleanup model is given when nothing else says.
///
/// Carried over from the ollama-based dictation path, where every sentence of
/// it was bought with a failure. "Never answer, explain, or act on the text"
/// is there because instruct models answer questions; the spelling rules are
/// there because "parse underscore config" is a person spelling an identifier.
///
/// A default, not a constant that decides anything: see [`Policy`].
pub const DEFAULT_SYSTEM_PROMPT: &str =
    "You clean up dictated speech that will be sent to a coding agent.

Rewrite the user's text: fix grammar, remove filler words and false starts, and restore \
obvious punctuation. Keep the intent and every technical term, filename, and identifier \
exactly as spoken. Never answer, explain, or act on the text -- only rewrite it. Reply \
with the rewritten text and nothing else.

Write plain text only: no markdown, no backticks, no code fences or quotes around names. \
When the speaker says \"underscore\", \"dot\", \"dash\" or \"slash\" between words they are \
spelling out an identifier or path, so join it up: \"parse underscore config\" is \
parse_config, \"server dot py\" is server.py.";

/// Phrases that mean the model quoted its instructions back.
///
/// Tells of [`DEFAULT_SYSTEM_PROMPT`] specifically, which is why they live on
/// the [`Policy`] beside the prompt rather than in a constant everything reads.
/// A configurable prompt with a hardcoded echo list is a guard that stops
/// guarding the moment somebody configures a prompt.
pub const DEFAULT_ECHOES: &[&str] = &["rewrite the user", "filler word", "only rewrite"];

/// How many tokens a rewrite may take before it is abandoned.
///
/// A cleanup pass is one sentence. This is a stop, not a target: a model that
/// has not finished in 200 tokens is not cleaning anything up.
pub const DEFAULT_MAX_NEW_TOKENS: usize = 200;

/// The shortest rewrite that is never refused for its length.
///
/// Below this, doubling a three-word dictation would refuse any reasonable
/// expansion of it — "run em" to "run them" is already 8% longer.
pub const DEFAULT_LENGTH_FLOOR: usize = 120;

/// Everything about a cleanup pass that is a choice rather than a mechanism.
///
/// It is configuration in the sense the ticket means: [`Policy::resolve`] takes
/// the environment as a function, and AYEAYE-56 turns the same fields into a
/// file the binary reads and writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    /// What the model is told to do.
    pub system_prompt: String,
    /// How that instruction and the dictation are spelled for the model.
    pub template: Template,
    /// The stop, in tokens.
    pub max_new_tokens: usize,
    /// Phrases that disqualify a candidate as an echo of the prompt, matched
    /// case-insensitively. Empty is legitimate: a prompt whose tells nobody has
    /// written down is better guarded by nothing than by another prompt's.
    pub echoes: Vec<String>,
    /// The length below which a rewrite is never refused for growing.
    pub length_floor: usize,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
            template: Template::default(),
            max_new_tokens: DEFAULT_MAX_NEW_TOKENS,
            echoes: DEFAULT_ECHOES.iter().map(|e| (*e).to_string()).collect(),
            length_floor: DEFAULT_LENGTH_FLOOR,
        }
    }
}

/// Why a policy could not be read from the environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyError {
    /// A template family nobody implements. Refused rather than ignored, for
    /// the same reason an unknown `--flag` is: silently falling back to ChatML
    /// would leave somebody's Llama-3 model quietly answering their dictations.
    UnknownTemplate(String),
    /// A number that is not one.
    NotANumber {
        /// The setting that was being read.
        setting: String,
        /// What was found instead.
        value: String,
    },
}

impl core::fmt::Display for PolicyError {
    fn fmt(&self, out: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownTemplate(name) => write!(
                out,
                "{name:?} is not a chat template this build knows; it has chatml and llama3"
            ),
            Self::NotANumber { setting, value } => {
                write!(out, "{setting} is {value:?}, which is not a number")
            }
        }
    }
}

impl core::error::Error for PolicyError {}

impl Policy {
    /// Read a policy out of the environment, falling back to the defaults.
    ///
    /// `env` is a lookup rather than `std::env`, which is what makes "the
    /// system prompt is configuration" a decision a test can drive, and what
    /// keeps this crate pure. It is handed the *bare* name — `CLEANUP_PROMPT` —
    /// and the caller decides what to prefix, exactly as `Settings::resolve`
    /// does.
    ///
    /// An empty value is not a value. `CLEANUP_PROMPT=` in a shell that means
    /// to unset it must not blank the prompt, because a model with no
    /// instruction does not clean anything up — it continues the sentence.
    ///
    /// **Replacing the prompt drops the echo phrases with it**, unless
    /// `CLEANUP_ECHOES` names new ones as a comma-separated list. That is the
    /// coupling [`DEFAULT_ECHOES`] argues for, enforced at the one place that
    /// can break it: the default tells belong to the default prompt, and
    /// carrying them onto somebody else's is wrong in both directions — they
    /// catch nothing real, and they refuse a legitimate rewrite that happens to
    /// contain the words.
    pub fn resolve(env: impl Fn(&str) -> Option<String>) -> Result<Self, PolicyError> {
        let value = |name: &str| env(name).filter(|v| !v.trim().is_empty());
        let mut policy = Self::default();

        if let Some(prompt) = value("CLEANUP_PROMPT") {
            policy.system_prompt = prompt;
            policy.echoes = Vec::new();
        }
        if let Some(echoes) = value("CLEANUP_ECHOES") {
            policy.echoes = echoes
                .split(',')
                .map(str::trim)
                .filter(|echo| !echo.is_empty())
                .map(str::to_string)
                .collect();
        }
        if let Some(name) = value("CLEANUP_TEMPLATE") {
            policy.template = match name.trim().to_ascii_lowercase().as_str() {
                "chatml" => Template::chatml(),
                "llama3" => Template::llama3(),
                _ => return Err(PolicyError::UnknownTemplate(name.trim().to_string())),
            };
        }
        if let Some(tokens) = value("CLEANUP_MAX_TOKENS") {
            let not_a_number = || PolicyError::NotANumber {
                setting: "CLEANUP_MAX_TOKENS".to_string(),
                value: tokens.trim().to_string(),
            };
            let budget: usize = tokens.trim().parse().map_err(|_| not_a_number())?;
            // Zero is a number and is not a budget: it means the model is
            // loaded, prompted, and never allowed to say anything, which
            // degrades safely and is indistinguishable from a working
            // configuration. Refused rather than obeyed.
            if budget == 0 {
                return Err(not_a_number());
            }
            policy.max_new_tokens = budget;
        }

        Ok(policy)
    }

    /// The prompt for one dictation.
    pub fn prompt(&self, raw: &str) -> String {
        self.template.render(&self.system_prompt, raw)
    }
}

/// Why a dictation came back the way it was spoken.
///
/// Every variant is a rewrite that was refused, and every one of them was a
/// real failure of the ollama path before it was a variant here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
    /// Nothing produced a candidate: no model loaded, or generation failed.
    Unavailable,
    /// The transcription was blank, so there was nothing to rewrite — and a
    /// model handed nothing writes something, which would be a dictation the
    /// speaker never gave.
    NothingSaid,
    /// The candidate tidied away to nothing.
    Empty,
    /// The candidate quoted the instructions back.
    Instructions,
    /// More than one line. A cleanup pass is one line; several means the model
    /// answered the request instead of rewriting it.
    MultipleLines,
    /// The candidate grew past what a rewrite of this dictation could be.
    TooLong,
}

impl Kept {
    /// One line, for a log a person will read at two in the morning.
    pub fn why(self) -> &'static str {
        match self {
            Self::Unavailable => "no cleanup model answered",
            Self::NothingSaid => "the transcription was blank",
            Self::Empty => "the rewrite was empty",
            Self::Instructions => "the rewrite quoted the instructions back",
            Self::MultipleLines => "the rewrite was more than one line",
            Self::TooLong => "the rewrite was longer than the dictation could justify",
        }
    }
}

/// A dictation, cleaned up or deliberately not.
///
/// There is no constructor that produces one without text, and no accessor that
/// can return nothing. That is the acceptance criterion — "cleanup degrades to
/// the raw transcription rather than losing the dictation" — expressed as a
/// type instead of as a `try`/`except` somebody has to remember to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cleaned {
    text: String,
    kept: Option<Kept>,
}

impl Cleaned {
    /// The text to type. Always something, never the empty case of a failure.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The text to type, owned.
    pub fn into_text(self) -> String {
        self.text
    }

    /// Whether the model's rewrite was the thing accepted.
    pub fn was_rewritten(&self) -> bool {
        self.kept.is_none()
    }

    /// Why the raw transcription was kept, where it was.
    pub fn kept(&self) -> Option<Kept> {
        self.kept
    }
}

/// Whether a transcription is worth spending a model on.
///
/// [`settle`] asks this first and answers [`Kept::NothingSaid`] when it is
/// false, so a caller that runs a model *before* asking has bought seconds of
/// inference to be told something that was knowable for free — and has handed a
/// model a blank prompt, which it will fill.
pub fn worth_cleaning(raw: &str) -> bool {
    !raw.trim().is_empty()
}

/// Decide between a transcription and a model's rewrite of it.
///
/// Total, and that is the point. `candidate` is `None` when there was no model
/// or generation failed, which collapses "the slot was empty", "the weights are
/// corrupt", "the decode diverged" and "the model answered the question" into
/// one place with one answer: the words the speaker said.
pub fn settle(policy: &Policy, raw: &str, candidate: Option<&str>) -> Cleaned {
    let keep = |kept: Kept| Cleaned {
        text: raw.to_string(),
        kept: Some(kept),
    };

    if !worth_cleaning(raw) {
        return keep(Kept::NothingSaid);
    }
    let Some(candidate) = candidate else {
        return keep(Kept::Unavailable);
    };

    let candidate = tidy(candidate);
    if candidate.is_empty() {
        return keep(Kept::Empty);
    }

    let lowered = candidate.to_lowercase();
    if policy
        .echoes
        .iter()
        .any(|echo| lowered.contains(&echo.to_lowercase()))
    {
        return keep(Kept::Instructions);
    }
    if candidate.contains('\n') {
        return keep(Kept::MultipleLines);
    }
    // Characters rather than bytes: an accented dictation is not a longer one.
    if candidate.chars().count() > policy.length_floor.max(raw.chars().count() * 2) {
        return keep(Kept::TooLong);
    }

    Cleaned {
        text: candidate,
        kept: None,
    }
}

/// Strip the packaging a chat model puts round an answer.
///
/// A fence and a quote pair, then whitespace, and in that order: a model that
/// was told not to use markdown does it anyway, and the fence is on the outside
/// of the quotes when it does both. Applied once each rather than until it
/// stops changing, so that a dictation which genuinely *is* a quoted phrase
/// keeps its inner quotes.
///
/// Private: it hands back model output nobody has judged, and the only caller
/// entitled to that is [`settle`], on its way to a verdict.
fn tidy(candidate: &str) -> String {
    let text = candidate.trim();

    let text = match text.strip_prefix("```") {
        Some(rest) => {
            let rest = rest.trim_end().strip_suffix("```").unwrap_or(rest);
            // The opening fence may carry a language tag, which runs to the end
            // of that line and is not part of the answer. It may equally be a
            // one-line fence with no newline at all — ```like this``` — where
            // everything after the backticks *is* the answer. Splitting on the
            // newline first threw that whole case away as empty.
            match rest.split_once('\n') {
                Some((_tag, body)) => body,
                None => rest,
            }
            .trim()
        }
        None => text,
    };

    let text = match (text.chars().next(), text.chars().next_back()) {
        (Some(open @ ('"' | '\'')), Some(close)) if open == close && text.chars().count() > 1 => {
            &text[open.len_utf8()..text.len() - close.len_utf8()]
        }
        _ => text,
    };

    text.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::{Cleaned, Kept, Policy, PolicyError, settle, tidy, worth_cleaning};
    use crate::chat::Template;

    fn policy() -> Policy {
        Policy::default()
    }

    fn assert_kept(cleaned: &Cleaned, raw: &str, why: Kept) {
        assert_eq!(cleaned.text(), raw, "the dictation must survive");
        assert_eq!(cleaned.kept(), Some(why));
        assert!(!cleaned.was_rewritten());
    }

    // AYEAYE-55
    #[test]
    fn a_good_rewrite_is_accepted_and_reported_as_one() {
        let cleaned = settle(&policy(), "um so run the tests", Some("Run the tests."));

        assert_eq!(cleaned.text(), "Run the tests.");
        assert!(cleaned.was_rewritten());
        assert_eq!(cleaned.kept(), None);
    }

    // AYEAYE-55
    //
    // The whole acceptance criterion in one line: no model, and the dictation
    // still arrives.
    #[test]
    fn no_candidate_keeps_the_dictation_and_says_why() {
        let cleaned = settle(&policy(), "um so run the tests", None);
        assert_kept(&cleaned, "um so run the tests", Kept::Unavailable);
    }

    // AYEAYE-55
    #[test]
    fn a_rewrite_that_tidies_away_to_nothing_keeps_the_dictation() {
        assert_kept(
            &settle(&policy(), "run the tests", Some("   \n  ")),
            "run the tests",
            Kept::Empty,
        );
    }

    // AYEAYE-55
    //
    // The failure that put this guard in the Python: an instruct model handed a
    // short sentence sometimes restates the instructions instead of obeying
    // them, and the restatement gets typed into the pane.
    #[test]
    fn a_rewrite_that_quotes_the_instructions_keeps_the_dictation() {
        assert_kept(
            &settle(
                &policy(),
                "run the tests",
                Some("I will only rewrite what you say."),
            ),
            "run the tests",
            Kept::Instructions,
        );
    }

    // AYEAYE-55
    #[test]
    fn a_multi_line_rewrite_keeps_the_dictation() {
        assert_kept(
            &settle(&policy(), "run the tests", Some("Sure.\nHere you go:")),
            "run the tests",
            Kept::MultipleLines,
        );
    }

    // AYEAYE-55
    //
    // Growth past twice the dictation, floored so that a three-word clip is not
    // held to six words.
    #[test]
    fn a_rewrite_that_ran_away_keeps_the_dictation() {
        let raw = "run the tests and then tell me what broke in the parser please";
        assert_kept(
            &settle(&policy(), raw, Some(&"very ".repeat(60))),
            raw,
            Kept::TooLong,
        );
    }

    // AYEAYE-55
    #[test]
    fn a_short_dictation_is_not_held_to_twice_its_length() {
        let cleaned = settle(&policy(), "run em", Some("Run them."));
        assert_eq!(cleaned.text(), "Run them.");
    }

    // AYEAYE-55
    //
    // A blank transcription has nothing to clean, and a model handed nothing
    // invents something. That would be a dictation the speaker never gave,
    // which is the opposite failure to the one this module is mostly about and
    // just as bad.
    #[test]
    fn a_blank_transcription_is_never_rewritten_into_words() {
        assert_kept(
            &settle(&policy(), "  ", Some("Hello there.")),
            "  ",
            Kept::NothingSaid,
        );
    }

    // AYEAYE-55
    #[test]
    fn tidying_strips_a_wrapping_quote_pair() {
        assert_eq!(tidy("  \"Run the tests.\"  "), "Run the tests.");
        assert_eq!(tidy("'Run the tests.'"), "Run the tests.");
        // Not a pair, so not packaging.
        assert_eq!(tidy("\"quoted\" and more"), "\"quoted\" and more");
        // A lone quote is not a wrapping pair either.
        assert_eq!(tidy("\""), "\"");
    }

    // AYEAYE-55
    #[test]
    fn tidying_strips_a_markdown_fence_and_its_language_tag() {
        assert_eq!(tidy("```\nRun the tests.\n```"), "Run the tests.");
        assert_eq!(tidy("```text\nRun the tests.\n```"), "Run the tests.");
        // Fence on the outside, quotes on the inside: both go.
        assert_eq!(tidy("```\n\"Run the tests.\"\n```"), "Run the tests.");
    }

    // AYEAYE-55
    //
    // A one-line fence has no language tag and no newline, so everything after
    // the backticks is the answer. Splitting on the newline first threw the
    // whole rewrite away as empty — safe, since the dictation survived, and
    // still a good rewrite lost to a model doing what the prompt says it does.
    #[test]
    fn a_one_line_fence_is_not_the_same_as_an_empty_answer() {
        assert_eq!(tidy("```Run the tests.```"), "Run the tests.");
        assert!(
            settle(&policy(), "um run the tests", Some("```Run the tests.```")).was_rewritten()
        );
    }

    // AYEAYE-55
    //
    // Tidying happens before judging, so a fenced answer is judged on what is
    // inside the fence rather than refused for the backticks.
    #[test]
    fn a_fenced_rewrite_is_judged_on_what_is_inside_it() {
        let cleaned = settle(
            &policy(),
            "um run the tests",
            Some("```\nRun the tests.\n```"),
        );
        assert_eq!(cleaned.text(), "Run the tests.");
        assert!(cleaned.was_rewritten());
    }

    // AYEAYE-55
    //
    // The echo phrases are the *prompt's* tells, so they travel with it. A
    // policy that changed the prompt and kept the default list would be
    // guarding against instructions nobody is giving.
    #[test]
    fn the_echo_phrases_belong_to_the_prompt_rather_than_to_the_guard() {
        let policy = Policy {
            system_prompt: "Transcribe verbatim.".to_string(),
            echoes: vec!["transcribe verbatim".to_string()],
            ..Policy::default()
        };

        assert_kept(
            &settle(&policy, "run the tests", Some("I Transcribe Verbatim.")),
            "run the tests",
            Kept::Instructions,
        );
        // And the old default no longer fires, because it is no longer a tell.
        assert!(settle(&policy, "run the tests", Some("I only rewrite.")).was_rewritten());
    }

    // AYEAYE-55
    #[test]
    fn the_default_policy_carries_the_default_prompt() {
        let policy = Policy::default();
        assert!(policy.system_prompt.contains("clean up dictated speech"));
        assert_eq!(policy.template, Template::chatml());
    }

    // AYEAYE-55
    //
    // Criterion 2: the prompt is configuration.
    //
    // And the echoes go with it. The default tells belong to the default
    // prompt; carrying them onto somebody else's is wrong in both directions —
    // they catch nothing real, and they refuse a legitimate rewrite that
    // happens to contain the words. Asserting only `system_prompt` here is a
    // test that passes while the module's central claim is false.
    #[test]
    fn replacing_the_system_prompt_drops_the_echo_phrases_with_it() {
        let policy = Policy::resolve(|name| match name {
            "CLEANUP_PROMPT" => Some("Say it back in French.".to_string()),
            _ => None,
        })
        .expect("a named prompt resolves");

        assert_eq!(policy.system_prompt, "Say it back in French.");
        assert!(policy.prompt("bonjour").contains("Say it back in French."));
        assert!(policy.echoes.is_empty());
        // Which is observable, not bookkeeping: a rewrite carrying the old
        // prompt's tells is now an ordinary rewrite.
        assert!(settle(&policy, "run the tests", Some("I only rewrite.")).was_rewritten());
    }

    // AYEAYE-55
    #[test]
    fn the_environment_can_name_the_new_prompts_own_tells() {
        let policy = Policy::resolve(|name| match name {
            "CLEANUP_PROMPT" => Some("Say it back in French.".to_string()),
            "CLEANUP_ECHOES" => Some(" say it back , en francais ".to_string()),
            _ => None,
        })
        .expect("a prompt and its tells resolve");

        assert_eq!(policy.echoes, vec!["say it back", "en francais"]);
        assert_kept(
            &settle(&policy, "run the tests", Some("I Say It Back in French.")),
            "run the tests",
            Kept::Instructions,
        );
    }

    // AYEAYE-55
    //
    // `CLEANUP_PROMPT=` in a shell means "unset", not "no instructions". A
    // model with no instruction does not clean a sentence up, it continues it.
    #[test]
    fn an_empty_value_does_not_blank_the_prompt() {
        let policy = Policy::resolve(|name| match name {
            "CLEANUP_PROMPT" => Some("   ".to_string()),
            _ => None,
        })
        .expect("an empty value resolves to the default");

        assert_eq!(policy.system_prompt, Policy::default().system_prompt);
    }

    // AYEAYE-55
    #[test]
    fn the_environment_can_name_a_template_and_a_budget() {
        let policy = Policy::resolve(|name| match name {
            "CLEANUP_TEMPLATE" => Some("Llama3".to_string()),
            "CLEANUP_MAX_TOKENS" => Some("64".to_string()),
            _ => None,
        })
        .expect("both settings resolve");

        assert_eq!(policy.template, Template::llama3());
        assert_eq!(policy.max_new_tokens, 64);
    }

    // AYEAYE-55
    //
    // Refused rather than ignored. Falling back to ChatML would leave a
    // Llama-3 model answering dictations instead of rewriting them, and the
    // only symptom would be worse output.
    #[test]
    fn an_unknown_template_is_refused_by_name() {
        let error = Policy::resolve(|name| match name {
            "CLEANUP_TEMPLATE" => Some("alpaca".to_string()),
            _ => None,
        })
        .expect_err("an unimplemented template cannot resolve");

        assert_eq!(error, PolicyError::UnknownTemplate("alpaca".to_string()));
        assert!(error.to_string().contains("alpaca"));
    }

    // AYEAYE-55
    #[test]
    fn a_token_budget_that_is_not_a_number_is_refused() {
        let error = Policy::resolve(|name| match name {
            "CLEANUP_MAX_TOKENS" => Some("lots".to_string()),
            _ => None,
        })
        .expect_err("a budget that is not a number cannot resolve");

        assert!(error.to_string().contains("lots"));
    }

    // AYEAYE-55
    //
    // Zero is a number and is not a budget. It loads a model, prompts it, and
    // never lets it say anything — which degrades safely and is
    // indistinguishable from a configuration that works.
    #[test]
    fn a_zero_token_budget_is_refused_rather_than_obeyed() {
        assert!(
            Policy::resolve(|name| match name {
                "CLEANUP_MAX_TOKENS" => Some("0".to_string()),
                _ => None,
            })
            .is_err()
        );
    }

    // AYEAYE-55
    #[test]
    fn nothing_worth_cleaning_is_the_same_question_settle_asks_first() {
        assert!(!worth_cleaning(""));
        assert!(!worth_cleaning(" \n\t "));
        assert!(worth_cleaning("run the tests"));
    }

    // AYEAYE-55
    //
    // The length gate is `>`, so a rewrite exactly at the limit is allowed. The
    // boundary is asserted rather than reasoned about: an off-by-one here
    // refuses good rewrites and nothing else would notice.
    #[test]
    fn the_length_gate_admits_a_rewrite_exactly_at_the_limit() {
        let policy = Policy {
            length_floor: 0,
            ..Policy::default()
        };
        let raw = "abcde";

        assert!(settle(&policy, raw, Some(&"x".repeat(10))).was_rewritten());
        assert_kept(
            &settle(&policy, raw, Some(&"x".repeat(11))),
            raw,
            Kept::TooLong,
        );
    }

    // AYEAYE-55
    //
    // Every refusal has a line a person can read, and the accepted text can be
    // taken by value without going through a borrow.
    #[test]
    fn a_kept_dictation_can_say_why_and_a_cleaned_one_can_be_taken() {
        let cleaned = settle(&policy(), "run the tests", None);
        assert_eq!(
            cleaned.kept().map(Kept::why),
            Some("no cleanup model answered")
        );
        assert_eq!(cleaned.into_text(), "run the tests");

        assert_eq!(
            settle(&policy(), "um run the tests", Some("Run the tests."))
                .into_text()
                .as_str(),
            "Run the tests."
        );
    }

    // AYEAYE-55
    #[test]
    fn nothing_in_the_environment_is_the_default_policy() {
        assert_eq!(
            Policy::resolve(|_| None).expect("no settings"),
            Policy::default()
        );
    }
}
