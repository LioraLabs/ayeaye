//! What a request is turned away with when it is the wrong shape.
//!
//! These are the refusals that belong to *reading a request* rather than to any
//! one endpoint, which is why they are here and not beside a handler. The
//! daemon spells each of them the same way at every endpoint that can produce
//! it — `"bad json"` six times, `"no pane"` five — and the failure worth
//! preventing is the next endpoint spelling them a seventh way.
//!
//! Endpoint-specific refusals stay with their endpoint: `agent::refused` has
//! the ones about agents, directories and panes that could not be killed.

/// The body was not JSON this daemon could read. `bin/ayeaye:2688`, `:2699`.
///
/// One divergence, in the safe direction: the daemon parses the body and then
/// calls `.get` on whatever came back, so a body that is valid JSON but not an
/// object — `[1,2,3]`, or `"hello"` — reaches an `AttributeError` rather than
/// this sentence. Answering the same "bad json" for both is what the daemon
/// plainly meant, and it is what `/api/files/resolve` does explicitly at
/// `:2676` with its own wording.
pub const BAD_JSON: &str = "bad json";

/// The body named no pane at all. `bin/ayeaye:2691`.
///
/// Told apart from a pane that is not one of ours, which is
/// `agent::refused::NO_SUCH_PANE`: this one is a request that forgot to say
/// which pane, and the person holding the phone can do nothing about it.
pub const NO_PANE: &str = "no pane";

#[cfg(test)]
mod tests {
    use super::{BAD_JSON, NO_PANE};

    // AYEAYE-51 — transcribed from `bin/ayeaye`, where every endpoint that can
    // produce one of these produces exactly this text. `share/app.html` puts it
    // on screen unchanged, so the wording is the product.
    #[test]
    fn a_badly_shaped_request_is_refused_the_way_the_daemon_refuses_it() {
        assert_eq!(BAD_JSON, "bad json");
        assert_eq!(NO_PANE, "no pane");
    }
}
