//! Choosing a token from what a model scored.
//!
//! Pure, and here rather than beside the model, because it is the one part of
//! decoding that is a decision rather than an effect — and the part that was
//! wrong in a way no end-to-end test noticed.

/// The highest-scoring token the model is allowed to emit.
///
/// `None` when there is nothing legal to say: every token suppressed, an empty
/// vocabulary, or scores so degenerate that nothing compares greater than
/// anything else. A caller that gets `None` should stop decoding.
///
/// The `None` case is not hypothetical tidiness. The obvious version of this
/// function seeds `best = 0` and `best_logit = -inf`, and then returns token
/// **0** whenever nothing beats negative infinity — which is exactly the state
/// suppression creates. That version emits a suppressed token, repeatedly,
/// while looking like it is working.
///
/// `NaN` scores are skipped rather than compared: every comparison against
/// `NaN` is false, so a diverged model that produces them would otherwise fall
/// into the same hole.
pub fn best_allowed(logits: &[f32], suppressed: &[u32]) -> Option<u32> {
    let mut best: Option<(u32, f32)> = None;

    for (id, &logit) in logits.iter().enumerate() {
        // A vocabulary past u32 is not a vocabulary any of this can address;
        // stop rather than discard the best found so far.
        let Ok(id) = u32::try_from(id) else { break };
        if logit.is_nan() || suppressed.contains(&id) {
            continue;
        }
        match best {
            Some((_, best_logit)) if logit <= best_logit => {}
            _ => best = Some((id, logit)),
        }
    }

    best.map(|(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::best_allowed;

    // AYEAYE-54
    #[test]
    fn the_highest_scoring_token_wins() {
        assert_eq!(best_allowed(&[0.1, 0.9, 0.4], &[]), Some(1));
    }

    // AYEAYE-54
    #[test]
    fn a_suppressed_token_is_never_chosen_even_when_it_scores_highest() {
        assert_eq!(best_allowed(&[0.1, 0.9, 0.4], &[1]), Some(2));
    }

    // AYEAYE-54
    //
    // The bug this function exists to make impossible. Suppressing everything
    // used to return token 0 — a suppressed token, emitted because nothing
    // scored above negative infinity.
    #[test]
    fn nothing_legal_to_say_is_none_rather_than_token_zero() {
        assert_eq!(best_allowed(&[0.9, 0.5, 0.4], &[0, 1, 2]), None);
    }

    // AYEAYE-54
    #[test]
    fn an_empty_vocabulary_has_no_answer() {
        assert_eq!(best_allowed(&[], &[]), None);
    }

    // AYEAYE-54
    //
    // Every comparison against NaN is false, so a naive fold silently keeps
    // whatever it started with.
    #[test]
    fn nan_scores_are_skipped_rather_than_compared() {
        assert_eq!(best_allowed(&[f32::NAN, 0.2, f32::NAN], &[]), Some(1));
        assert_eq!(best_allowed(&[f32::NAN, f32::NAN], &[]), None);
    }

    // AYEAYE-54
    //
    // Ties go to the lower id, which is arbitrary but has to be *decided*:
    // an untrained or diverged model produces flat scores, and a decode that
    // depends on iteration order is a decode that changes under a refactor.
    #[test]
    fn a_tie_goes_to_the_lower_token_id() {
        assert_eq!(best_allowed(&[0.5, 0.5, 0.5], &[]), Some(0));
        assert_eq!(best_allowed(&[0.5, 0.5, 0.5], &[0]), Some(1));
    }

    // AYEAYE-54
    #[test]
    fn negative_infinity_still_beats_having_nothing_to_say() {
        assert_eq!(best_allowed(&[f32::NEG_INFINITY], &[]), Some(0));
    }
}
