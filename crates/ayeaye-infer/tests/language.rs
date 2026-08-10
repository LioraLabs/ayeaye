//! In-process transcript cleanup, observed only through its public API.

mod gguf;

use ayeaye_core::cleanup::{Kept, Policy};
use ayeaye_infer::language::{LanguageError, LanguageModel, LanguageSlot, SUPPORTED};
use gguf::{
    WINDOW, tiny_model, tiny_model_missing_a_tensor, tiny_model_named, tiny_model_without_a_window,
};

// AYEAYE-55
//
// "The model did not load" is the error message that costs an evening. Each
// file gets its own test because each is read at a different point, and a
// loader that named the first file whatever went missing would pass one of
// these and not the other.
#[test]
fn absent_weights_are_an_error_naming_the_weights() {
    let dir = tiny_model("no-weights");
    dir.remove("model.gguf");

    let error = LanguageModel::load(dir.path()).expect_err("a model with no weights cannot load");

    assert!(
        matches!(error, LanguageError::Missing { .. }),
        "expected a Missing error, got {error:?}"
    );
    assert!(
        error.to_string().contains("model.gguf"),
        "the error should name the file it wanted: {error}"
    );
}

// AYEAYE-55
#[test]
fn an_absent_tokenizer_is_an_error_naming_the_tokenizer() {
    let dir = tiny_model("no-tokenizer");
    dir.remove("tokenizer.json");

    let error = LanguageModel::load(dir.path()).expect_err("a model with no tokenizer cannot load");

    assert!(
        matches!(error, LanguageError::Missing { .. }),
        "expected a Missing error, got {error:?}"
    );
    assert!(
        error.to_string().contains("tokenizer.json"),
        "the error should name the file it wanted: {error}"
    );
}

// AYEAYE-55
#[test]
fn weights_that_are_not_a_gguf_are_a_malformed_error_naming_them() {
    let dir = tiny_model("corrupt-weights");
    dir.corrupt("model.gguf");

    let error = LanguageModel::load(dir.path()).expect_err("a file that is not a GGUF cannot load");

    assert!(
        matches!(error, LanguageError::Malformed { .. }),
        "expected a Malformed error, got {error:?}"
    );
    assert!(error.to_string().contains("model.gguf"));
}

// AYEAYE-55
#[test]
fn a_tokenizer_that_is_not_json_is_a_malformed_error_naming_it() {
    let dir = tiny_model("corrupt-tokenizer");
    dir.corrupt("tokenizer.json");

    let error =
        LanguageModel::load(dir.path()).expect_err("a tokenizer that is not JSON cannot load");

    assert!(
        matches!(error, LanguageError::Malformed { .. }),
        "expected a Malformed error, got {error:?}"
    );
    assert!(error.to_string().contains("tokenizer.json"));
}

// AYEAYE-55
//
// The architecture allowlist, at load time. `candle-transformers` implements
// architectures one at a time, so an unsupported one is a fact about the file
// rather than about the machine — knowable from the header, before a gigabyte
// of tensors is read. AYEAYE-56 asks the same question at pull time, against
// the same list.
#[test]
fn an_architecture_this_build_cannot_run_is_refused_by_name() {
    let dir = tiny_model_named("unsupported", "mamba");

    let error =
        LanguageModel::load(dir.path()).expect_err("an unimplemented architecture cannot load");

    assert!(
        matches!(error, LanguageError::UnsupportedArchitecture { .. }),
        "expected an UnsupportedArchitecture error, got {error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains("mamba"),
        "the error should name what it found: {message}"
    );
    assert!(
        message.contains("llama"),
        "and what it can run instead: {message}"
    );
}

// AYEAYE-55
//
// The list is public so AYEAYE-56 can refuse a download rather than restating
// it. Every name on it is loaded by a test below — a list is only honest if
// something proves each entry, and "supported on paper" is exactly the failure
// an allowlist is supposed to prevent.
#[test]
fn every_published_architecture_really_loads() {
    assert_eq!(SUPPORTED, &["llama", "qwen2"]);
    for architecture in SUPPORTED {
        let dir = tiny_model_named(architecture, architecture);
        let model = LanguageModel::load(dir.path())
            .unwrap_or_else(|e| panic!("{architecture} is on the list and did not load: {e}"));
        assert_eq!(&model.architecture(), architecture);
    }
}

// AYEAYE-55
//
// The window is the loader's rotary table, not the file's claim about itself,
// and the two architectures answer differently: `quantized_llama` builds a
// fixed 4 096 whatever the file says, `quantized_qwen2` builds exactly the
// context length it reads. A position past the table indexes out of bounds
// rather than degrading, so getting this from the wrong place is a panic in the
// middle of somebody's dictation.
#[test]
fn each_architecture_reports_the_window_its_own_loader_built() {
    let llama =
        LanguageModel::load(tiny_model_named("window-llama", "llama").path()).expect("llama loads");
    let qwen =
        LanguageModel::load(tiny_model_named("window-qwen2", "qwen2").path()).expect("qwen2 loads");

    assert_eq!(qwen.window(), WINDOW);
    assert!(
        llama.window() > WINDOW,
        "llama ignores the file's context length: {}",
        llama.window()
    );
}

// AYEAYE-55
//
// Absent and zero are different facts, and the message has to assert the one
// that is true: telling somebody their context length is 0 sends them looking
// for a number that is not in their file at all.
#[test]
fn a_file_that_never_says_how_long_its_context_is_says_so_in_those_words() {
    let dir = tiny_model_without_a_window("no-window");

    let error = LanguageModel::load(dir.path()).expect_err("a model with no window cannot load");

    let message = error.to_string();
    assert!(
        matches!(error, LanguageError::Malformed { .. }),
        "expected a Malformed error, got {error:?}"
    );
    assert!(message.contains("qwen2.context_length"), "{message}");
    assert!(
        !message.contains(" 0,"),
        "an absent key must not be reported as a zero: {message}"
    );
}

// AYEAYE-55
//
// The other side of the window: an ordinary dictation has to fit, with the
// default system prompt in front of it. The fixture's window and
// `DEFAULT_SYSTEM_PROMPT` live in different crates with nothing linking them,
// so without this the first person to add a sentence to the prompt gets a
// window error from an unrelated test and no clue why.
#[test]
fn an_ordinary_dictation_fits_the_window_with_the_default_prompt_in_front_of_it() {
    let dir = tiny_model_named("headroom", "qwen2");
    let mut slot = LanguageSlot::empty();
    slot.load(dir.path()).expect("a complete model loads");

    let raw = "tell me what broke in the parser and then run the tests again please ".repeat(3);
    slot.rewrite(&raw, &Policy::default())
        .expect("a dictation of ordinary length has to fit");
}

// AYEAYE-55
//
// Refused with the two numbers in it, rather than met as an out-of-bounds index
// part way through a dictation. And refusal is not loss: `clean` turns it back
// into the words the speaker said.
#[test]
fn a_prompt_past_the_window_is_refused_by_the_numbers_and_still_returns_the_dictation() {
    let dir = tiny_model_named("too-long", "qwen2");
    let mut slot = LanguageSlot::empty();
    slot.load(dir.path()).expect("a complete model loads");
    let raw = "run the tests ".repeat(WINDOW);

    let error = slot
        .rewrite(&raw, &Policy::default())
        .expect_err("a prompt past the window cannot be run");
    let message = error.to_string();
    assert!(message.contains(&WINDOW.to_string()), "{message}");

    assert_eq!(slot.clean(&raw, &Policy::default()).text(), raw);
}

// AYEAYE-55
//
// A GGUF that parses and is short a tensor the architecture needs is a
// malformed *file*, not a panic and not an unsupported architecture.
#[test]
fn a_gguf_missing_a_tensor_is_malformed_rather_than_a_panic() {
    let dir = tiny_model_missing_a_tensor("short-a-tensor");

    let error = LanguageModel::load(dir.path()).expect_err("an incomplete model cannot load");

    assert!(
        matches!(error, LanguageError::Malformed { .. }),
        "expected a Malformed error, got {error:?}"
    );
    assert!(error.to_string().contains("model.gguf"));
}

// AYEAYE-55
//
// Criterion 1's first half, and criterion 4 by construction: this runs on the
// CPU and nothing else.
#[test]
fn a_complete_quantized_model_loads_from_the_directory_it_was_pointed_at() {
    let dir = tiny_model("loads");

    let model = LanguageModel::load(dir.path()).expect("a complete model loads");

    assert_eq!(model.architecture(), "llama");
    assert_eq!(model.backend().label(), "cpu");
    // The Debug line is what ends up in a log, and it must describe the model
    // rather than print gigabytes of weights.
    let described = format!("{model:?}");
    assert!(described.contains("llama"), "{described}");
    assert!(described.contains("cpu"), "{described}");
}

// AYEAYE-57
//
// The language model got the same wiring as the speech model and needs the same
// hold-down: `load_with` names a selection this machine cannot produce, so the
// state a GPU artifact reaches after a dead card is reachable here. See the
// twin in `tests/speech.rs` for the argument.
#[test]
fn a_model_loaded_after_a_fallback_carries_the_reason_out_with_it() {
    let dir = tiny_model("carries-the-reason");
    let selection = ayeaye_infer::backend::choose(ayeaye_infer::Backend::Metal, |_| {
        Err(candle_core::Error::Msg(
            "no Metal device is present".to_string(),
        ))
    });

    let model = LanguageModel::load_with(dir.path(), selection.clone())
        .expect("a model should load on the device it fell back to");

    assert_eq!(model.backend(), selection.got());
    assert_eq!(model.backend(), ayeaye_infer::Backend::Cpu);
    let why = model
        .fallback()
        .expect("the reason should survive the load");
    assert_eq!(Some(why), selection.fallback());
    assert!(why.contains("metal"), "{why}");
    assert!(why.contains("no Metal device is present"), "{why}");
}

// -------------------------------------------------------------- generation

// AYEAYE-55
//
// Criterion 3, at its bluntest. No model at all, and the dictation still
// arrives — with no `Result` in the caller's way to unwrap the wrong direction.
#[test]
fn cleaning_with_an_empty_slot_returns_the_dictation_itself() {
    let mut slot = LanguageSlot::empty();

    let cleaned = slot.clean("um so run the tests", &Policy::default());

    assert_eq!(cleaned.text(), "um so run the tests");
    assert!(!cleaned.was_rewritten());
    assert_eq!(cleaned.kept(), Some(Kept::Unavailable));
}

// AYEAYE-55
//
// The diagnostic half of the pair says what happened, so a log can. It does not
// load a model: a slot that loads on first use has taken out a lifetime nobody
// wrote down.
#[test]
fn rewriting_with_an_empty_slot_is_an_error_rather_than_a_load() {
    let mut slot = LanguageSlot::empty();

    let error = slot
        .rewrite("run the tests", &Policy::default())
        .expect_err("an empty slot cannot rewrite");

    assert!(
        matches!(error, LanguageError::NotLoaded),
        "expected NotLoaded, got {error:?}"
    );
    assert!(!slot.is_loaded());
}

// AYEAYE-55
#[test]
fn a_slot_loads_unloads_and_reloads() {
    let dir = tiny_model("residency");
    let mut slot = LanguageSlot::empty();

    assert!(!slot.is_loaded());
    assert!(!slot.unload(), "there was nothing to unload");

    slot.load(dir.path()).expect("a complete model loads");
    assert!(slot.is_loaded());
    assert!(slot.unload());
    assert!(!slot.is_loaded());

    slot.load(dir.path())
        .expect("and loads again after unloading");
    assert!(slot.is_loaded());
}

// AYEAYE-55
//
// Criterion 1's second half, and criterion 4: a loaded quantized model is
// prompted with the policy's system prompt and generates, on the CPU. The toy
// model's weights are noise, so what it says is noise — what is under test is
// that it says a bounded amount of it and stops.
#[test]
fn a_loaded_model_generates_within_the_token_budget_and_stops() {
    let dir = tiny_model("generates");
    let mut slot = LanguageSlot::empty();
    slot.load(dir.path()).expect("a complete model loads");

    let policy = Policy {
        max_new_tokens: 8,
        ..Policy::default()
    };
    let written = slot
        .rewrite("run the tests", &policy)
        .expect("a loaded model rewrites");

    // A word-level vocabulary, so tokens are words: at most the budget, and the
    // point is that it terminated at all rather than running to the context.
    assert!(
        written.split_whitespace().count() <= policy.max_new_tokens,
        "generated past its budget: {written:?}"
    );
}

// AYEAYE-55
//
// The assertion a single-shot test cannot make, and the one holding up the
// ticket's subtlest claim. Both loaders keep a key-value cache inside every
// layer and neither offers a way to clear it; what clears it is starting a
// generation at position zero, which each of them special-cases by dropping the
// cache instead of concatenating onto it. Nothing in candle's API says so, so
// this test is the only thing standing between us and one dictation attending
// to the last — a wrong answer with no symptom but a worse one.
//
// Both architectures, because it is a property of each loader separately.
#[test]
fn two_dictations_through_one_loaded_model_do_not_bleed_into_each_other() {
    let policy = Policy {
        max_new_tokens: 6,
        ..Policy::default()
    };

    for architecture in SUPPORTED {
        let dir = tiny_model_named(&format!("no-bleed-{architecture}"), architecture);
        let mut slot = LanguageSlot::empty();
        slot.load(dir.path()).expect("a complete model loads");

        let first = slot.rewrite("run the tests", &policy).expect("the first");
        let _elsewhere = slot
            .rewrite("tell me what broke in the parser please", &policy)
            .expect("a different dictation in between");
        let again = slot
            .rewrite("run the tests", &policy)
            .expect("the first again");

        assert_eq!(
            first, again,
            "{architecture} gave two answers to one dictation, so state survived between them"
        );
    }
}

// AYEAYE-55
//
// Criterion 3 against a model that really ran. The toy weights are noise, so
// whatever it produces is exactly the kind of thing the guard exists to refuse —
// which makes this the honest version of the degradation test rather than a
// staged one. Either outcome is correct; returning nothing is not.
#[test]
fn cleaning_against_a_real_model_never_loses_the_dictation() {
    let dir = tiny_model("degrades");
    let mut slot = LanguageSlot::empty();
    slot.load(dir.path()).expect("a complete model loads");

    for raw in [
        "um so run the tests",
        "tell me what broke in the parser please",
        "run",
    ] {
        let cleaned = slot.clean(raw, &Policy::default());
        assert!(
            !cleaned.text().is_empty(),
            "a dictation was lost: {raw:?} came back empty"
        );
        assert!(
            cleaned.was_rewritten() || cleaned.text() == raw,
            "{raw:?} came back as neither a rewrite nor itself: {:?}",
            cleaned.text()
        );
    }
}

// AYEAYE-55
//
// A blank dictation is never handed to a model. It would cost seconds of
// inference to be told what the guard already knows, and a model given a blank
// prompt fills it — which would be a dictation the speaker never gave.
#[test]
fn a_blank_dictation_is_not_worth_a_model_and_is_never_rewritten() {
    let dir = tiny_model("blank");
    let mut slot = LanguageSlot::empty();
    slot.load(dir.path()).expect("a complete model loads");

    let cleaned = slot.clean("   ", &Policy::default());

    assert_eq!(cleaned.text(), "   ");
    assert_eq!(cleaned.kept(), Some(Kept::NothingSaid));
}
