//! How long a model stays in memory, and what happens when the choice changes.
//!
//! `SpeechSlot` already refuses to load on demand, deliberately: a slot that
//! quietly loads on first use takes out a lifetime nobody wrote down. What it
//! has never had is somebody to decide. This is that somebody, and it is pure —
//! the deciding is arithmetic over a duration and two names, and only the
//! carrying-out touches memory.
//!
//! The clock is an argument. `std::time::Instant` is outside the core's effect
//! budget, and taking the elapsed time as a parameter is what makes every
//! branch here reachable from a test without one waiting for anything.

use std::time::Duration;

use super::id::ModelId;

/// When a resident model is let go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    /// How long it may sit with nothing to do. `None` keeps it until something
    /// else releases it — a reconfiguration, or the process ending.
    pub idle: Option<Duration>,
}

/// What to do with the slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// Leave it exactly as it is.
    Keep,
    /// Load the wanted model into a slot that is empty.
    Load,
    /// Let go of what is resident, and load nothing.
    Release,
    /// Let go of what is resident, **then** load the wanted one.
    ///
    /// The order is in the name because it is the whole point. Loading first
    /// and releasing afterwards would hold two models at once, so a
    /// reconfiguration would momentarily need double the memory of the thing it
    /// was reconfiguring — on the machines this runs on, that is the difference
    /// between a reconfiguration and an out-of-memory kill.
    Reload,
}

/// What to do when somebody wants to transcribe.
///
/// Loading happens here, on demand, and nowhere else. `wanted` is an `Option`
/// because no model configured is an ordinary state: that machine runs
/// text-only, and it should not be a special case at every call site.
pub fn on_demand(loaded: Option<&ModelId>, wanted: Option<&ModelId>) -> Plan {
    match (loaded, wanted) {
        (None, None) => Plan::Keep,
        (None, Some(_)) => Plan::Load,
        // Configured back to nothing: the resident model is not what anybody
        // asked for any more, and holding it would be memory nothing can
        // account for.
        (Some(_), None) => Plan::Release,
        (Some(here), Some(want)) if here == want => Plan::Keep,
        (Some(_), Some(_)) => Plan::Reload,
    }
}

/// What to do when nothing has wanted the model for a while.
///
/// Only ever releases. A sweep that could load would mean a model appearing in
/// memory because time passed, which is nobody's intention.
pub fn on_idle(loaded: Option<&ModelId>, idle_for: Duration, policy: &Policy) -> Plan {
    match (loaded, policy.idle) {
        (Some(_), Some(limit)) if idle_for >= limit => Plan::Release,
        _ => Plan::Keep,
    }
}

#[cfg(test)]
mod tests {
    use super::{Plan, Policy, on_demand, on_idle};
    use crate::model::ModelId;
    use std::time::Duration;

    fn id(spelled: &str) -> ModelId {
        ModelId::parse(spelled).expect("a well-formed id")
    }

    // AYEAYE-56 — the acceptance criterion's last clause: never leaked across
    // reconfiguration. Changing the chosen model releases the old one *before*
    // the new one is loaded, so the two are never resident at once. The
    // opposite order needs double the memory at the moment of the change,
    // which on these machines is the difference between a reconfiguration and
    // an out-of-memory kill.
    #[test]
    fn changing_the_model_releases_the_old_one_before_loading_the_new() {
        let old = id("openai/whisper-small.en");
        let new = id("openai/whisper-tiny.en");
        assert_eq!(on_demand(Some(&old), Some(&new)), Plan::Reload);

        // The same model is not a reload. Reloading on every request would be
        // hundreds of megabytes of work to arrive at what was already there.
        assert_eq!(on_demand(Some(&old), Some(&old)), Plan::Keep);

        // A revision is part of the identity, so the same repository at a
        // different revision is a different model.
        let pinned = id("openai/whisper-small.en@a1b2c3d");
        assert_eq!(on_demand(Some(&old), Some(&pinned)), Plan::Reload);
    }

    // AYEAYE-56 — loaded on demand: an empty slot loads when something wants
    // it, and configuring the model away releases what is resident rather than
    // leaving memory nothing can account for.
    #[test]
    fn a_model_is_loaded_on_demand_and_released_when_it_is_configured_away() {
        let model = id("openai/whisper-small.en");
        assert_eq!(on_demand(None, Some(&model)), Plan::Load);
        assert_eq!(on_demand(Some(&model), None), Plan::Release);
        // No model chosen and none resident is an ordinary state, not an error:
        // that machine runs text-only.
        assert_eq!(on_demand(None, None), Plan::Keep);
    }

    // AYEAYE-56 — released on a policy. At the limit as well as past it, since
    // "five minutes idle" that keeps it at exactly five minutes is a policy
    // nobody described.
    #[test]
    fn an_idle_model_is_released_once_it_reaches_the_limit() {
        let model = id("openai/whisper-small.en");
        let after = Policy {
            idle: Some(Duration::from_secs(300)),
        };

        assert_eq!(
            on_idle(Some(&model), Duration::from_secs(299), &after),
            Plan::Keep
        );
        assert_eq!(
            on_idle(Some(&model), Duration::from_secs(300), &after),
            Plan::Release
        );
        assert_eq!(
            on_idle(Some(&model), Duration::from_secs(3_000), &after),
            Plan::Release
        );
    }

    // AYEAYE-56 — a sweep only ever releases. One that could load would mean a
    // model appearing in memory because time passed.
    #[test]
    fn a_sweep_never_loads_anything() {
        let never = Policy { idle: None };
        let model = id("openai/whisper-small.en");

        // Nothing resident: nothing to do, however long it has been.
        assert_eq!(
            on_idle(None, Duration::from_secs(9_999), &never),
            Plan::Keep
        );
        let eager = Policy {
            idle: Some(Duration::from_secs(1)),
        };
        assert_eq!(
            on_idle(None, Duration::from_secs(9_999), &eager),
            Plan::Keep
        );

        // And a policy of `None` keeps it however long it sits, which is what
        // AYEAYE_MODEL_IDLE=0 asks for.
        assert_eq!(
            on_idle(Some(&model), Duration::from_secs(86_400), &never),
            Plan::Keep
        );
    }
}
