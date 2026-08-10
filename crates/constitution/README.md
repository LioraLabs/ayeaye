# The constitution

This crate is the workspace's rules in a form that can refuse a build. It is
normative: where this file and somebody's intention disagree, this file wins,
because it is the one the suite reads.

Four rules are enforced today, the fourth in two halves. They are **tier 1** —
the rules that keep the pure core pure, the strata apart, and the build free of
a C toolchain. The
duplication tiers, and the waiver ratchet that makes them landable, arrive with
AYEAYE-64.

Every rule takes its input as **data**: source text, a list of dependency
names, a graph of crates, the text of a lockfile. Only `corpus.rs` touches a
disk. That separation is
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
  (`EFFECTFUL_METHODS`), for the receiver that arrived as an argument. A name
  earns its place by meaning the effect on its own: `read`, `write`, `open`,
  `create` and `flush` are all deliberately **absent**, because a pure type
  could answer to any of them and there is no per-violation waiver to let one
  off with.

**Aliases do not launder a reach.** `use std as s`, `use std::time as t`,
`extern crate std as s`, and `use r#std::fs` all resolve to the reach they
really name. An alias is only opaque when the *module* is imported, so every
module above a forbidden reach is refused outright — import the item, not the
module. That is a property of the table rather than of the entries somebody
remembered: a test walks every entry's ancestors and fails if one of them is
unrefused, so a new deep entry cannot land without its cover.

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

## Rule 4 — the pure Rust graph

> Nothing the portable build needs may require a C, C++ or CUDA compiler.

Two halves, because one input cannot answer it. `toolchain::check` reads
`Cargo.lock` and proves the *graph* compiles no C; `toolchain::gated` reads the
*manifests* and proves the cost `cc` cannot see stays out of the default build.

`toolchain::check(lockfile, forbidden)` reads the text of `Cargo.lock` and
refuses the package names in `FORBIDDEN`:

| Package | Why it is refused |
|---|---|
| `cc` | how a build script compiles C or C++; the general case |
| `cmake` | runs CMake at build time |
| `bindgen` | needs libclang at build time |
| `onig`, `onig_sys` | oniguruma, a C library — how this nearly arrived |

**`cc` is the one that makes this cheap to trust.** Whatever vendors native
source *for a C compiler to build*, `cc` is what builds it, so `cc` absent from
the lockfile is a mechanical proof that no such crate is in the graph —
including crates nobody has read. The rest are named for the other two shapes
that cost takes, and for the specific near-miss below. It is not proof that
nothing anywhere compiles C: a build script that drives its own compiler needs
no `cc`, which is exactly what the CUDA path does. See below.

**It reads the lockfile, not the manifests, and that is the point.** The
manifests say what *we* asked for. This cost arrives through somebody else's
dependency: `candle-core` 0.10 names `tokenizers` with the `onig` feature
itself, and no amount of `default-features = false` on our own line has any
effect on that. `ayeaye-infer` is held at candle 0.9 for exactly this reason —
the note in its manifest is the long version.

**What it cannot catch, and it is worse than it looked.** A lockfile is
feature-blind in **both** directions, and AYEAYE-57 measured both:

- A name being present is not proof anything is compiled. The lockfile lists
  every optional dependency whether or not a feature enables it — `bindgen_cuda`
  sits in it today with `cuda` off. AYEAYE-56 measured that half.
- A name being absent is not proof that nothing is. **`cc` is absent under
  `--features cuda` too.** `candle-kernels`' build script drives **nvcc**
  directly over its `.cu` sources, compiles `src/moe/*.cu` into a static
  `libmoe.a`, and emits `cargo:rustc-link-lib=stdc++` and
  `cargo:rustc-link-lib=dylib=cudart`; `bindgen_cuda` depends on `glob`,
  `num_cpus` and `rayon`, and on nothing that compiles C.

That is why the paragraph above is worded as narrowly as it is. `cc` absent
proves no crate in the graph vendors C *for a C compiler to build*; it proves
nothing about a build script that drives its own compiler. The first half
remains the right rule for the cost it was written for. It simply cannot answer
"does the default build need a toolchain" at all, in either direction, because
the lockfile does not record what is on.

**The first half's mutation tests** plant `onig_sys` and `cc` in a synthetic
lockfile. Against the real tree the proof runs the other way round: a planted
package cannot be used, because cargo rewrites `Cargo.lock` before the test runs
and drops any entry nothing depends on — so the test instead hands the rule a
table naming `libc`, which the graph really does carry, and fails if the rule
finds nothing.

### The second half: a feature that legitimately needs a toolchain

The manifests are the only place that records what is *on*, so that is what the
second half reads. `toolchain::gated(subject, manifest, gated)` judges one
manifest's text against `GATED`:

| Feature | What building it costs | Which artifact stops being static |
|---|---|---|
| `cuda` | nvcc and a host C++ compiler; candle-kernels compiles `.cu` and links `stdc++` and `cudart` | the x86_64 Linux NVIDIA build, glibc-dynamic rather than static musl |

**A name on `GATED` is not permission to be on. It is permission to exist, on
the condition that nothing turns it on by default.** That is the decision this
table records: an optional acceleration feature is allowed to need a toolchain,
because the milestone accepted one non-portable artifact out of five — and the
price is written next to it rather than left in a commit message.

It refuses two shapes, because closing one leaves the other open:

1. **The transitive closure of `[features] default`.** Transitive, because
   `default = ["everything"]` with `everything = ["cuda"]` is the same build,
   and a check that read one level would pass it.
2. **A `features = [...]` array on any dependency edge**, target-conditional
   tables included. This is not hypothetical: it is exactly how `ayeaye-infer`
   turns `metal` on for Apple builds, since a cargo feature nobody passes is
   off. The way past a `default = []` check is to not use `default`.

**`metal` is deliberately not on the table.** `candle-metal-kernels` declares
`build = false`, and every build script in the Apple graph is pure Rust with no
build-dependencies, so Metal costs no toolchain. Gating it would be a rule
nobody could obey, since being on by default in an Apple build is the point.

**A gated feature is rarely reachable only under its own name**, so `Gated`
carries the other spellings that turn the same cost on. candle defines
`cudnn = ["cuda", …]` and `nccl = ["cuda", …]`, and `default =
["candle-core/cudnn"]` pays for nvcc while naming nothing called `cuda`. Both
are on the table; a rule that matched one keyword would be a filter, not a rule.

**The root manifest is judged too.** It is not a workspace member, it has no
sources, and it declares dependencies under `[workspace.dependencies]` — which
is where this workspace keeps candle, and therefore the most likely place a
future acceleration edit lands. A rule over "every member's manifest" would skip
exactly that file.

Both halves are `Rule::PureRustGraph`, because they are one rule. The second
half's mutation tests plant a `default` naming a gated feature, a `default` that
reaches one through another of our own features, a `default` naming an implying
spelling, a target table forcing one on, and a `[workspace.dependencies]` edge
forcing one on — and assert that the same target table forcing `metal` on is
clean. Against the real tree the proof runs the other way round again: there is
no violation to leave planted, so the test hands the rule a table naming
`metal`, which `ayeaye-infer` really does turn on for Apple builds, and fails if
it finds nothing.


---

## Running it

```
cargo test -p constitution          # the rules, and the rules against this tree
cook rust-suite                     # the same, as the build system's cached unit
```

`crates/constitution/tests/constitution.rs` runs all four rules over the real
workspace, rule 4 in both halves — the lockfile against `FORBIDDEN`, and every
member's manifest against `GATED`. It also asserts the corpus walk found a
non-trivial number of files, and that every crate the strata place contributed
at least one — a walk that
finds nothing passes every rule it feeds, silently, which is the failure those
floors exist to make loud.

It asserts one more thing, which is about the build system rather than about
the code: **every workspace member lives under `crates/`.** The Cookfile's
`rust` probe is a glob rooted there, and a member outside it would be read by
this walk and invisible to that probe — a green release gate over source
nothing rebuilt on. Move a member and the probe moves with it, in the same
commit.

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
