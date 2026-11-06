# Contributor instructions

Report blocked checks and unfinished behavior explicitly; never invent benchmark
results. Commit each validated milestone, and continue with the next authorized
milestone without waiting for confirmation. Stop only when user input is actually required.

## Architecture

- Use Rust and GPUI. Support Linux/NixOS and macOS. Keep OS integration
  isolated so Git, history, models, and most UI logic remain portable.
- Keep Git process management and history data independent of GPUI. Keep state
  transitions and display preparation pure where possible. Avoid abstraction-only
  wrappers and speculative backend/plugin systems.
- Use typed IDs and generation tokens for asynchronous work. Reject stale results
  after selection, query, repository, HEAD, or theme changes.
- Invoke Git with argument arrays, never shell interpolation. Bound stderr capture,
  drain pipes without deadlocks, and terminate and reap children on cancellation.
- Branch switching must preserve Git's ordinary protections. Never automatically
  force checkout, stash, reset, or discard changes.

## Performance invariants

- Never run blocking Git, filesystem scans, or highlighting in render paths
  or on the UI thread. Never wait for a total history count before showing rows.
- Virtualize history and diff lines. Paint graph geometry only for visible rows.
- Match Go's streaming batch model: retain loaded history in memory, avoid eager
  whole-history loading and duplicate large data, and keep pending batches bounded.
  Virtualization limits drawing work; it does not bound total retained history.
- Do not introduce SQLite or a persistent history/parsing cache without measured
  need and an agreed plan change. Continue one Git stream, avoiding repeated --skip
  scans. Pause prefetch when browsing demand is satisfied.
- Bundle broad offline language coverage, not a small allowlist. Preserve grammar
  licenses and detect filenames/shebangs.
- Functionality and workflow parity take priority in v1. Timing/RSS numbers are
  aspirations, not release gates. Avoid obvious performance mistakes, record
  meaningful bottlenecks, and defer elaborate optimization until features work.
- Avoid whole-worktree recursive watcher expansion on nixpkgs. Coalesce events and
  reconcile status periodically; watch failure must leave manual reload usable.
- When optimizing, compare equivalent workloads and loaded counts with Go. Report
  application and child-process memory separately; never invent measurements.

## Dependencies, verification, and documentation

- Keep README.md focused on user-facing features, usage, and setup. Put
  implementation details, dependency versions, architecture, and internal
  workarounds in contributor documentation instead.
- Pin GPUI and use documentation matching that release. Commit Cargo.lock and
  flake.lock. Use the Rust toolchain from pinned nixpkgs for builds and checks.
- Keep native dependencies conditional on platform. Preserve license notices for
  ported code and bundled grammars, fonts, icons, and other assets.
- Once scaffolding exists, use these reproducible commands:
  - nix develop --command cargo fmt --all -- --check
  - nix develop --command cargo clippy --all-targets -- -D warnings
  - nix develop --command cargo test --all-targets --locked
  - nix build
  - nix flake check
- Protect behavior fixes with regression tests at the useful boundaries: pure
  unit tests for algorithms/parsing, tests/ workflows for real Git and worker
  integration, and headless GPUI tests for view, input and focus interactions.
- cargo test --all-targets must run all Rust tests, including GPUI, headlessly.
  Use deterministic temporary Git fixtures; keep native desktop smoke and million-
  commit benchmarks opt-in. Smoke-test real Wayland and X11 separately.
- Run checks relevant to the increment. Documentation-only changes need link,
  checklist, consistency, and ASCII checks, not application tests.
- Keep AGENTS.md ASCII-only and concise. Document observed sandbox, network, build,
  and display limitations with the command and actual error. Do not weaken checks
  or claim completion to work around an unavailable environment.
- Do not rewrite existing Git history without an explicit user request.

## Local environment limitations

- Sandboxed `nix eval --raw --inputs-from . crane#outPath` failed with
  `executing SQLite statement 'pragma synchronous = off': unable to open database file`
  for the Nix fetcher cache. Run Nix validation with approved cache access.
