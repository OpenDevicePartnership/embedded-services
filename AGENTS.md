# AGENTS.md — embedded-services

Guidance for AI coding agents (Copilot CLI, Claude, Cursor, etc.) working in
the `OpenDevicePartnership/embedded-services` workspace. Human contributors
may also find the conventions and command reference useful, but this file is
written for autonomous agents that need a single, self-contained briefing
before touching the tree.

> **Companion documents.** Keep these open while you work:
>
> - [`.github/copilot-instructions.md`](.github/copilot-instructions.md) —
>   the authoritative behavioural contract; it is loaded into the GitHub
>   Copilot context automatically. This file mirrors and extends it.
> - [`README.md`](README.md) — narrative overview of the EC services
>   architecture, transports, and the service catalog.
> - [`docs/api-guidelines.md`](docs/api-guidelines.md) — detailed rationale
>   for the `Service<'hw>` trait pattern, lifetime conventions, and the
>   "no `'static` references" rule.
> - [`docs/power-policy.md`](docs/power-policy.md) — power policy service
>   design.
> - [`CONTRIBUTING.md`](CONTRIBUTING.md) and
>   [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md).

---

## 1. Project snapshot

- **Purpose.** A Cargo workspace of `no_std` Rust crates implementing the
  business logic for an embedded controller (EC). Services glue together
  vendor HALs (via `embedded-hal` / `embedded-hal-async` traits), peripheral
  drivers, and EC subsystem abstractions (sensors, batteries, fans, USB-PD,
  thermal, eSPI, …) on top of the Embassy async runtime.
- **Edition.** Rust 2024, MSRV `1.90` (`rust-toolchain.toml` pins channel
  `1.93` for development).
- **License.** MIT (`LICENSE`).
- **Default branch.** `main`. Some service patterns are still being migrated;
  the `v0.2.0` branch shows the target patterns for new development (see the
  note in `.github/copilot-instructions.md`).
- **Targets.**
  - `x86_64-unknown-linux-gnu` — host target used for tests and std-only
    crates such as `fw-update-interface-mocks`.
  - `thumbv8m.main-none-eabihf` — primary embedded target (ARM Cortex‑M33).
- **Runtime.** [Embassy](https://embassy.dev/) async executor and
  synchronisation primitives (`embassy-executor`, `embassy-sync`,
  `embassy-time`, `embassy-futures`).

## 2. Workspace layout

The workspace root `Cargo.toml` declares ~30 member crates and a centralised
`[workspace.dependencies]` table. The `examples/` directory contains
**separate** workspaces (`examples/rt633`, `examples/rt685s-evk`,
`examples/std`, `examples/pico-de-gallo`) and is excluded from the root.

| Crate | Role |
|---|---|
| `embedded-service` | Core utilities: intrusive list, IPC primitives (`ipc::deferred`, `broadcaster`, `relay`), `GlobalRawMutex`, `SyncCell`, `fmt` macros. Re-exported as `embedded_services`. |
| `odp-service-common` | Shared traits including `runnable_service::Service<'hw>` and `ServiceRunner`, plus the `spawn_service!` macro. |
| `battery-service`, `battery-service-interface`, `battery-service-relay` | Battery subsystem (service + trait crate + relay). |
| `thermal-service`, `thermal-service-interface`, `thermal-service-relay` | Thermal subsystem. |
| `time-alarm-service`, `time-alarm-service-interface`, `time-alarm-service-relay` | RTC / alarm subsystem. |
| `power-policy-service`, `power-policy-interface` | Power policy (see `docs/power-policy.md`). |
| `type-c-service`, `type-c-interface` | USB Type‑C / USB‑PD orchestration (uses `embedded-usb-pd`, `tps6699x`). |
| `fw-update-interface`, `fw-update-interface-mocks` | Firmware update traits (mocks crate is std-only). |
| `cfu-service` | Component Firmware Update over the `embedded-cfu` protocol. |
| `espi-service` | eSPI transport / memory map. |
| `uart-service` | UART transport. |
| `hid-service`, `keyboard-service` | HID over I²C and keyboard composition. |
| `power-button-service` | Power button handling. |
| `debug-service`, `debug-service-messages` | Debug message bus. |
| `platform-service` | Cross-platform helpers (e.g., reset). |
| `mctp-rs` | MCTP transport bindings used by relays. |
| `partition-manager/{generation,macros,partition-manager}` | Partition table tooling (proc-macro + runtime). |

Always check the root `Cargo.toml` for the canonical list. A new service
crate **must** be added to `[workspace.members]` and its path dependency
registered in `[workspace.dependencies]`.

## 3. Build, test, lint, and doc commands

These commands match what CI runs in `.github/workflows/check.yml`. Run them
from the workspace root unless otherwise noted.

```shell
# Format check (CI: fmt job)
cargo fmt --check

# Apply formatting locally
cargo fmt

# Workspace tests (host target). CI sets -C instrument-coverage; you do not
# need to.
cargo test --locked

# Single crate / single test
cargo test --locked -p partition-manager
cargo test --locked -p partition-manager test_name

# Clippy on test code (CI: test job, second step)
cargo clippy --locked --tests

# Feature-powerset clippy for both targets (CI: hack-clippy)
cargo hack --feature-powerset \
  --mutually-exclusive-features=log,defmt,defmt-timestamp-uptime \
  clippy --locked --target x86_64-unknown-linux-gnu

cargo hack --feature-powerset \
  --mutually-exclusive-features=log,defmt,defmt-timestamp-uptime \
  clippy --exclude fw-update-interface-mocks --locked \
  --target thumbv8m.main-none-eabihf

# Docs (CI: doc job, runs on nightly with RUSTDOCFLAGS=--cfg docsrs)
cargo doc --no-deps -F log --locked
cargo doc --no-deps -F defmt --locked

# MSRV check (CI: msrv job, toolchain 1.90)
cargo +1.90 check -F log    --locked --target x86_64-unknown-linux-gnu
cargo +1.90 check -F defmt  --locked --target x86_64-unknown-linux-gnu
cargo +1.90 check -F log    --locked --workspace \
  --exclude fw-update-interface-mocks --target thumbv8m.main-none-eabihf
cargo +1.90 check -F defmt  --locked --workspace \
  --exclude fw-update-interface-mocks --target thumbv8m.main-none-eabihf

# Dependency hygiene (CI: deny, machete jobs)
cargo deny check --all-features --locked
cargo machete

# Supply chain (manual + cargo-vet workflow)
cargo vet --locked
```

### Example workspaces

Each example directory is its own workspace and must be built from inside
its directory:

```shell
cd examples/rt685s-evk    && cargo clippy --target thumbv8m.main-none-eabihf --locked
cd examples/rt633         && cargo clippy --target thumbv8m.main-none-eabihf --locked
cd examples/std           && cargo clippy --locked
cd examples/pico-de-gallo && cargo clippy --locked
```

### Tooling installation

The required helpers (`cargo-hack`, `cargo-deny`, `cargo-machete`,
`cargo-vet`, `grcov`) are installed in CI via `taiki-e/install-action` and
`EmbarkStudios/cargo-deny-action`. Locally:

```shell
cargo install cargo-hack cargo-deny cargo-machete cargo-vet
```

## 4. Architecture conventions

### 4.1 The `Service<'hw>` pattern

New services and refactors of existing services follow the
`odp_service_common::runnable_service::Service<'hw>` trait. The pattern
enforces a uniform shape:

1. **`Resources`** — caller-allocated state stored in a `StaticCell` by the
   `spawn_service!` macro. **No internal `OnceLock` singletons.**
2. **`new(resources, params) -> (Self, Runner)`** — returns a control
   handle and a `Runner`.
3. **`Runner`** — implements `ServiceRunner` with a single
   `async fn run(self) -> !` that drives the service event loop.
4. **`spawn_service!`** — macro that allocates `Resources` in a
   `StaticCell`, calls `new()`, and spawns the `Runner` on an Embassy
   executor.

```rust
#[derive(Default)]
pub struct Resources<'hw> {
    inner: Option<ServiceInner<'hw>>,
}

pub struct MyService<'hw> { /* control handle */ }
pub struct Runner<'hw>    { /* references into Resources */ }

impl<'hw> Service<'hw> for MyService<'hw> {
    type Resources  = Resources<'hw>;
    type Runner     = Runner<'hw>;
    type InitParams = MyInitParams<'hw>;
    type ErrorType  = MyError;

    async fn new(
        resources: &'hw mut Self::Resources,
        params:    Self::InitParams,
    ) -> Result<(Self, Self::Runner), Self::ErrorType> {
        // …
    }
}

// At composition root:
let my_service = spawn_service!(spawner, MyService, my_init_params)?;
```

Core principles (see `docs/api-guidelines.md` for the full rationale):

- **No `'static` references in public APIs.** Use a generic `'hw` lifetime
  so the service is testable on a host.
- **External memory allocation.** Callers own `Resources`; services never
  reach for `static OnceLock`/`OnceCell` singletons.
- **Trait-based public APIs.** Runtime interfaces live in standalone
  `-interface` crates (e.g., `battery-service-interface`) so consumers can
  mock or substitute implementations.

> Some crates on `main` still use older patterns (`comms::Endpoint`,
> `MailboxDelegate`, `OnceLock` singletons). Do **not** propagate those
> patterns into new code. The `v0.2.0` branch shows the migration target.

### 4.2 IPC mechanisms

Pick the smallest mechanism that fits the data flow:

- `embassy_sync::channel::Channel` — bounded async MPSC for command /
  response streams.
- `embassy_sync::signal::Signal` — single-value notification with
  "latest wins" semantics.
- `embedded_services::ipc::deferred` — request/response where the caller
  awaits a reply (typed channels with a deferred completion).
- `embedded_services::broadcaster` — publish/subscribe fan-out for events.
- `embedded_services::relay` — MCTP-style request/response dispatch with
  direct async calls; used by the `-relay` crates.

### 4.3 Core utilities (`embedded-service` crate)

- `GlobalRawMutex` — alias that resolves to `ThreadModeRawMutex` on ARM
  bare-metal and `CriticalSectionRawMutex` on RISC‑V bare-metal as well as
  std/test builds. Use this for any cross-task mutex inside a service.
- `SyncCell<T>` — `ThreadModeCell` on ARM, `CriticalSectionCell` elsewhere;
  interior mutability for embedded.
- `fmt` macros — `trace!`, `debug!`, `info!`, `warn!`, `error!` dispatch to
  the active logging backend (`defmt` or `log`).

### 4.4 Composition

An EC binary is a top-level Embassy application that wires services
together. Subsystems compose multiple services that communicate through
the transport layer (intrusive endpoints). See `README.md` and the
`examples/` workspaces for end-to-end wiring patterns.

## 5. Coding conventions

### 5.1 `no_std` and feature flags

- All service crates are `#![no_std]`.
- Logging is feature-gated and the `log` / `defmt` /
  `defmt-timestamp-uptime` features are **mutually exclusive** (enforced
  via `cargo hack --mutually-exclusive-features`). Never enable more than
  one logging backend simultaneously in production code.
- Use the unified macros from `embedded_services::fmt` rather than calling
  `defmt::*` or `log::*` directly so crates stay backend-agnostic.

### 5.2 Lints (defined in root `Cargo.toml`)

```toml
[workspace.lints.rust]
warnings = "deny"

[workspace.lints.clippy]
correctness         = "deny"
expect_used         = "deny"
indexing_slicing    = "deny"
panic               = "deny"
panic_in_result_fn  = "deny"
perf                = "deny"
suspicious          = "deny"
style               = "deny"
todo                = "deny"
unimplemented       = "deny"
unreachable         = "deny"
unwrap_used         = "deny"
```

Consequences for agents:

- Do not call `.unwrap()` / `.expect()` in production code. Return a
  `Result` or use a `match` with explicit handling.
- Replace `slice[i]` with `slice.get(i)` (or `.get_mut(i)`) and propagate
  the `Option`.
- No `panic!`, `todo!`, `unimplemented!`, or `unreachable!` in production
  code paths. Tests may relax these with `#[allow(clippy::panic)]`,
  `#[allow(clippy::unwrap_used)]`, etc.

### 5.3 Error handling

- Prefer per-module custom `enum` error types — `thiserror` is unavailable
  (it requires std). Lightweight struct error types are acceptable when
  the variant set is naturally a single value.
- Derive `Debug, Clone, Copy, PartialEq, Eq` on error enums when
  practical; some errors only derive a subset (commonly `Debug`/`Copy`).
- Add `defmt` support behind a cfg:
  `#[cfg_attr(feature = "defmt", derive(defmt::Format))]`.
- Use per-module `Result` aliases, e.g.
  `pub type BatteryResponse = Result<ContextResponse, ContextError>`.

### 5.4 Dependencies

- Centralise dependency versions in the root
  `[workspace.dependencies]` and reference them from member crates with
  `dep.workspace = true`.
- Several upstreams come from the OpenDevicePartnership GitHub org as git
  dependencies: `embassy-imxrt`, `embedded-usb-pd`, `embedded-cfu`,
  `tps6699x`. Re-pin via the root table, not in member crates.
- Feature-gated optional deps (`log`, `defmt`) should be listed under
  `[package.metadata.cargo-machete] ignored` to silence false positives
  from `cargo machete`.
- Every new dependency must satisfy `cargo deny check` (license,
  advisories, sources, bans) and `cargo vet` (supply chain). See
  `deny.toml` and `supply-chain/`.

### 5.5 Testing

- Async unit tests in Embassy-focused `no_std` crates use
  `embassy_futures::block_on(async { … })` so they remain
  runtime-agnostic.
- Host-only async tests in crates that depend on `tokio` may use
  `#[tokio::test]` (`tokio` features: `rt`, `macros`, `time`).
- Dev-dependencies typically enable `std`/`tokio` features:
  `embassy-sync/std`, `embassy-time/std`, `critical-section/std`,
  `tokio = { version = "1", features = ["rt", "macros", "time"] }`.
- Tests live in `#[cfg(test)] mod tests` blocks or dedicated `tests/`
  directories.
- After fixing or writing tests, also run
  `cargo clippy --locked --tests` (CI runs it as a separate step).

### 5.6 Formatting

- `rustfmt.toml` sets `max_width = 120`.
- Run `cargo fmt` before committing. CI gates on `cargo fmt --check`.
- Line endings are LF; the repository expects `core.autocrlf=false` on
  Windows. Do not let an editor convert files to CRLF.

## 6. Workflow and contribution rules

### 6.1 Branching

- Default branch is `main`. Open PRs against `main` unless the work is
  explicitly targeting `v0.2.0` (in which case follow the patterns in
  `.github/copilot-instructions.md`).
- Keep feature branches focused; small, reviewable PRs are preferred.

### 6.2 Commit messages

Follow the
[standard Git commit message conventions](https://tbaggery.com/2008/04/19/a-note-about-git-commit-messages.html):

- Subject capitalised, ≤ 50 characters, imperative mood ("Fix bug", not
  "Fixed bug").
- Blank line between subject and body.
- Body wrapped at 72 columns; explain the *what* and *why*, not the *how*.

### 6.3 AI attribution (mandatory)

Every commit that contains AI-generated or AI-assisted work **must**
include an `Assisted-by` trailer:

```
Assisted-by: AGENT_NAME:MODEL_VERSION [TOOL1] [TOOL2]
```

- `AGENT_NAME` — e.g., `GitHub Copilot`.
- `MODEL_VERSION` — the specific model version actually used, e.g.,
  `claude-opus-4.7`. **Verify your own identity before composing the
  trailer; do not reuse a model name from a previous session.**
- Optional `[TOOL]` entries name specialised analysis tools used
  (`coccinelle`, `sparse`, `clang-tidy`, …). Editors, `git`, and `cargo`
  do not count.

AI agents **MUST NOT** add `Signed-off-by` trailers. Only humans can
certify the Developer Certificate of Origin.

### 6.4 Pull requests and review

- CI workflows (see `.github/workflows/`):
  - `check.yml` — fmt, doc, hack-clippy (stable + beta × two targets),
    deny, test (+ coverage), msrv, example workspaces, machete.
  - `rolling.yml` — periodic refresh against newer toolchains.
  - `cargo-vet.yml`, `cargo-vet-pr-comment.yml` — supply-chain auditing.
- The PR review skill (`.github/skills/code-review/SKILL.md`) lists the
  shape of an AI-assisted review.
- When reviewing or self-reviewing, pay particular attention to:
  - Async selection APIs (`select`, `select_array`, `select_slice`) — they
    drop the futures that do not complete. Make sure no work-in-flight or
    state is silently lost.
  - Code marked with "panic safety" or "drop safety" comments.
  - New `unsafe` blocks — justify them in a comment and keep them narrow.

### 6.5 What an agent must not do

- Do **not** introduce `'static` references, `OnceLock` singletons, or
  global mutable state into new code.
- Do **not** add `unwrap`/`expect`/`panic`/`todo`/`unimplemented` to
  production code paths.
- Do **not** enable `log` and `defmt` simultaneously, or bypass the
  `embedded_services::fmt` façade for logging.
- Do **not** rewrite shared git history, force-push to other people's
  branches, or push directly to upstream `main`.
- Do **not** add `Signed-off-by` trailers from an AI agent.
- Do **not** commit secrets, tokens, or vendor-confidential material.
- Do **not** edit `Cargo.lock` files by hand; let `cargo` regenerate them.

## 7. Quick agent checklist

Use this list before declaring a change "done":

1. `cargo fmt --check` is clean.
2. `cargo clippy --locked --tests` is clean.
3. `cargo test --locked` passes (host target).
4. For changes that touch features or new crates, re-run the
   feature-powerset clippy commands from §3 on **both** targets.
5. `cargo doc --no-deps -F log --locked` and `-F defmt --locked` both
   succeed.
6. `cargo machete` reports no unused dependencies.
7. If dependencies changed: `cargo deny check --all-features --locked`
   and `cargo vet --locked` pass.
8. Commit message follows §6.2 and includes the `Assisted-by` trailer
   from §6.3.
9. No CRLF line endings, no stray `.unwrap()`/`.expect()`/`panic!()` in
   production code, no new `'static` references in service APIs.

## 8. Where to look for more detail

- **Design rationale:** `docs/api-guidelines.md`,
  `docs/power-policy.md`, `README.md` (mermaid diagrams of subsystem
  topologies).
- **Behavioural contract for Copilot:**
  `.github/copilot-instructions.md` (extends and is extended by this
  file).
- **Reusable agent skills:** `.github/skills/` —
  `address-review/SKILL.md`, `cargo-vet-audit/SKILL.md`,
  `code-review/SKILL.md`. Reuse these instead of reinventing review or
  audit prompts.
- **Custom agents:** `.github/agents/cargo-vet-auditor.agent.md`.
- **CI definitions:** `.github/workflows/`.
- **Supply chain config:** `deny.toml`, `supply-chain/`.

When in doubt, prefer the patterns demonstrated in the most recently
modified service crate that already implements `Service<'hw>` (see git
log of `odp-service-common` and any of the `*-service` crates).

## Model selection & cost discipline

Premium models (Opus, GPT-5 family, "high"/"xhigh" reasoning variants)
cost an order of magnitude more than standard models (Sonnet, Haiku,
mini). Most steps in a typical task do not need premium reasoning,
and over-using premium models wastes credits without improving
outcomes. The rules below apply to *all* model selection: your own
session, sub-agents launched via the `task` tool, and parallel work
launched via `/fleet`.

### Default posture

- **Default to the cheapest model that can do the job.** Reach for a
  premium model only when one of the escalation triggers below is hit.
- **Plan with premium, execute with cheap.** Spend at most one or two
  premium turns on design / planning, then downshift to a cheaper
  model for mechanical execution of the plan.
- **Never bump the model "just in case."** If you cannot articulate
  *why* a cheaper model would fail, use the cheaper model.

### Escalation triggers (use a premium model)

Reach for a premium model when *any* of these are true:

- Cross-module refactor, architectural design, or API design from
  scratch.
- Subtle correctness reasoning: concurrency, lifetimes, `unsafe`,
  FFI ABI, cryptography, safety-critical control paths.
- Debugging a failure that survived one prior cheap-model attempt.
- Reviewing code on a safety-, security-, or money-critical path.
- The diff cannot be predicted in advance — i.e. there is genuine
  creative or design work to do, not just typing.

### De-escalation triggers (use a cheap model)

Use the cheapest available model when *any* of these are true:

- Searching, reading, summarising files or docs.
- Single-file mechanical edits: rename, format, lint fix, dependency
  bump, boilerplate, scaffolding from a known template.
- Generating tests for code that already works.
- Running builds, tests, linters, or other commands where the model
  only needs to report success/failure.
- Routine commits, PR descriptions, changelog entries.
- The diff is essentially predictable before generation.

### Sub-agent routing (the `task` tool)

When delegating with the `task` tool, set `model:` explicitly. Do not
let sub-agents inherit a premium default for cheap work.

| Sub-agent type    | Default model             | Override to                                     |
|-------------------|---------------------------|-------------------------------------------------|
| `explore`         | cheap                     | keep cheap (`claude-haiku-4.5` or `gpt-5-mini`) |
| `task` (run cmd)  | cheap                     | keep cheap                                      |
| `research`        | cheap for breadth         | premium only for the final synthesis            |
| `general-purpose` | match task                | cheap for mechanical work; premium for design   |
| `rubber-duck`     | premium                   | keep premium — this is where reasoning pays off |
| `code-review`     | premium on critical paths | cheap on cosmetic / mechanical diffs            |

### `/fleet` (parallel sub-agents) rules

- Fleet mode multiplies cost by the fleet width. Apply the rules
  above *per worker*, not in aggregate.
- Split a fleet job along complexity lines: route the cheap,
  parallelisable workers (file edits, test runs, doc updates) to a
  cheap model; reserve premium models for the small number of
  workers that need real reasoning.
- If every worker in a fleet would need a premium model, the work is
  probably not a good fit for fleet mode — reconsider the
  decomposition before paying N× premium.

### Session hygiene

- Keep sessions short and focused. Long premium sessions are the
  single largest source of waste because every turn re-processes the
  full history.
- Use `/compact` when the conversation grows long, and `/new` for
  unrelated work.
- Prefer `/ask` for one-off side questions so they don't extend the
  main session.

### When in doubt

Ask: *"If a cheaper model produced the wrong answer here, would I
catch it in seconds (compiler, tests, my own review) or in
weeks (production incident)?"* If the former, use the cheap model
and let the feedback loop do its job.
