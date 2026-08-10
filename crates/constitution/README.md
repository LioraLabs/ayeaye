# The constitution

This crate is the workspace's rules in a form that can refuse a build. It is
normative: where this file and somebody's intention disagree, this file wins,
because it is the one the suite reads.

Three rules are enforced today. They are **tier 1** — the rules that keep the
pure core pure and the strata apart. The duplication tiers, and the waiver
ratchet that makes them landable, arrive with AYEAYE-64.

Every rule takes its input as **data**: source text, a list of dependency
names, a graph of crates. Only `corpus.rs` touches a disk. That separation is
not tidiness — it is what lets each rule be handed a planted violation and
asked whether it notices. **A rule with no mutation test is not done.**

---

## Rule 1 — the effect budget

> `ayeaye-core` may not reach outside itself.

`effect_budget::scan(path, source)` is given the text of one file and returns
findings. The reaches it refuses are the `BUDGET` table in `effect_budget.rs`,
each with the reason it is refused; the table is the authority, and the
headlines are the filesystem, streams, the network, subprocesses, the process
environment, threads, the clock, and operating-system specifics.

**It compares reaches, not spellings.** `use std::time::Instant` and
`use std::time::{Duration, Instant}` name the same reach, and the second names
one more that is perfectly fine. So `use` trees are expanded before anything is
compared, brace groups and nesting and `self` and globs included, and the
`Duration` is acquitted while the `Instant` is not.

**It catches what has nothing to import.** Three shapes:

- A fully-qualified path written inline, with no `use` anywhere:
  `std::fs::read_to_string(p)`.
- A prelude macro, which needs no import at all: `println!`, `eprintln!`,
  `dbg!` and their siblings. This is the shape that made an earlier version of
  this rule green and blind at the same time.
- A short list of method names that are effectful whatever the receiver is
  (`EFFECTFUL_METHODS`), for the receiver that arrived as an argument. `read`
  and `write` are deliberately **not** on it: those are `RwLock`'s, and a pure
  crate may hold a lock.

**Aliases do not launder a reach.** `use std as s`, `use std::time as t`,
`extern crate std as s`, and `use r#std::fs` all resolve to the reach they
really name. The budget refuses the bare `std` and bare `std::time` imports
outright for this reason: import the item, not the module.

**Comments and literals are not code.** They are blanked before scanning, or
this file and the budget table itself would convict the crate that holds them.
Prose naming a forbidden reach is prose.

**`env!` is permitted where `std::env` is not.** `env!` reads the environment
of the *compiler*, at compile time, and is a constant by the time the program
runs. `std::env` reads the environment of the running process. They look alike
and are opposites. The same reasoning admits `include_str!`, which is how a
fixture gets into a pure crate's tests: through the compiler, not through
something that can open a file.

**`panic!` is not an effect** under this rule. It ends the program rather than
reaching outside it, and a pure function that refuses an impossible input is
still pure.

**What it scans.** Every `.rs` file under the crate's `src/`, `tests/`,
`benches/` and `examples/`, plus `build.rs` at the crate root. `#[cfg(test)]`
code is included on purpose: a pure crate's tests need no filesystem, and
exempting them would make the rule advisory. A build script runs on whatever
machine is doing the building, so a crate carrying one is exactly as pure as
that script is.

**What it cannot do.** It is a scanner, not a compiler. It does not resolve
types, so it cannot tell you that a trait method on a value you were handed
reaches a socket. It reads the text of one file at a time and knows nothing
about what a macro expands to. Rule 2 exists because of the first limit, and
nothing yet covers the second.

---

## Rule 2 — the core's dependency allowlist

> `ayeaye-core` may not declare a dependency that is not on the list.

`deps::check(crate_name, declared, allowed)` judges the names declared in every
dependency table — `[dependencies]`, `[dev-dependencies]`,
`[build-dependencies]`, and the `[target.'cfg(…)'.…]` forms of all three. A
rename (`local = { package = "tokio" }`) is recorded as the crate it really is.

`ALLOWED` is **empty**, and that is the honest starting state. A name on the
list is permission granted in advance.

**This rule is not redundant with rule 1.** A crate that opens a file inside
itself is invisible to any scan of *our* source. That is the whole point, and
it is why admission is not "does it look harmless": a crate admitted here must
be effect-free in its own non-optional transitive dependencies. It is also why
timestamp and cookie handling are hand-rolled rather than pulled from crates
that carry a clock along with them.

**Development dependencies count.** Rule 1 already scans `#[cfg(test)]` code;
exempting test dependencies would leave the core one `dev-dependencies` line
away from a filesystem. Fixtures arrive by `include_str!`.

---

## Rule 3 — the strata

> A crate may depend only on a strictly lower stratum.

`strata::check(crates, strata)` judges the edges between workspace members.
The table is `STRATA` in `strata.rs`:

| Stratum | Crate | What it is |
|---|---|---|
| 0 | `ayeaye-core` | pure: text and structs in, text and structs out |
| 1 | `ayeaye-infer` | inference: model files, device memory, time |
| 2 | `ayeaye` | the binary: subprocesses, filesystem, sockets, lifetime |
| 3 | `constitution` | tooling: may see everything, may be seen by nothing |

Upwards is the edge that dissolves the split. Sideways is the edge that turns
two crates into one crate with two manifests. Both are findings.

**A crate absent from the table is itself a finding.** Adding a crate is a
deliberate act — placing it is part of adding it, not a formality — and
without this the way past the rule is to not be in it.

---

## Running it

```
cargo test -p constitution          # the rules, and the rules against this tree
cook rust-suite                     # the same, as the build system's cached unit
```

`crates/constitution/tests/constitution.rs` runs all three rules over the real
workspace. It also asserts the corpus walk found a non-trivial number of files,
and that every crate the strata place contributed at least one — a walk that
finds nothing passes every rule it feeds, silently, which is the failure those
floors exist to make loud.

## Amending a rule

The rules are meant to be changed; they are not meant to be edged around.

1. **Change the data, not the scanner.** A new forbidden reach is a line in
   `BUDGET`, with the reason written where the finding will quote it. A new
   crate is a line in `STRATA`. A newly admitted dependency is a line in
   `ALLOWED` — with a note saying why its non-optional transitive dependencies
   are effect-free, because that is the claim being made.
2. **A new rule ships with its mutation test.** A synthetic input carrying a
   deliberate violation, which must produce a finding. This is the acceptance
   criterion, not a nicety: a rule nobody has watched fire is a rule that may
   already be blind.
3. **Keep the rule's input synthetic-able.** If a rule can only be run against
   the real tree, point 2 is impossible. Where the real data has no example of
   the violation — `STRATA` has no two crates at the same height, `ALLOWED` is
   empty — the table is passed in as an argument so a test can supply one that
   does.
4. **Loosening is a decision, and it is written down.** Removing an entry means
   saying, here, why the thing it refused is now acceptable. Until AYEAYE-64
   lands the waiver ratchet there is no per-violation exemption: a rule either
   holds for the whole crate or it does not hold.
