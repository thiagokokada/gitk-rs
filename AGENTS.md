# Contributor instructions

## Architecture

- Use Rust and GPUI. Support Linux/NixOS and macOS. Keep OS integration isolated
  so Git, history, models, and most UI logic remain portable.
- Keep Git process management and history data independent of GPUI. Keep state
  transitions and display preparation pure where possible. Avoid
  abstraction-only wrappers and speculative backend/plugin systems.
- Use typed IDs and generation tokens for asynchronous work. Reject stale
  results after selection, query, repository, HEAD, or theme changes.
- Invoke Git with argument arrays, never shell interpolation. Bound stderr
  capture, drain pipes without deadlocks, and terminate and reap children on
  cancellation.
- Branch switching must preserve Git's ordinary protections. Never automatically
  force checkout, stash, reset, or discard changes.

## Performance invariants

- Never run blocking Git, filesystem scans, or highlighting in render paths or
  on the UI thread. Never wait for a total history count before showing rows.
- Virtualize history and diff lines. Paint graph geometry only for visible rows.
- Retain loaded history in memory, stream incrementally, avoid duplicate large
  data, and keep pending batches bounded. Stream to EOF by default.
  Virtualization limits drawing work; it does not bound total retained history.
- Do not introduce SQLite or a persistent history/parsing cache without measured
  need and an agreed plan change.
- Avoid whole-worktree recursive watcher expansion. Coalesce events and
  reconcile status periodically; watch failure must leave manual reload usable.
- Report application and child-process memory separately; never invent
  measurements.

## Dependencies, verification, and documentation

- Keep README.md concise and factual, focused on user-facing features, usage,
  and setup. Put implementation details, architecture, and internal workarounds
  in the `docs/` directory. Avoid promotional or generic filler, exhaustive
  shortcut lists, repeated caveats, and examples that imply stronger limits or
  guarantees than the implementation provides. Prefer linking to `--help` for
  complete option details; explain only behavior needed to use a feature safely.
- Do not duplicate configurable default values in documentation. Refer to
  generated help or the implementation; keep historical measurement settings
  explicitly tied to the measurement.
- Pin GPUI Kit and use documentation matching its release and GPUI backend.
  Commit Cargo.lock and flake.lock. Use the Rust toolchain from pinned nixpkgs
  for reproducible builds and checks.
- Keep native dependencies conditional on platform.
- Use these reproducible commands as relevant to the change:
  - nix develop --command cargo fmt --all -- --check
  - nix develop --command cargo clippy --all-targets --locked -- -D warnings
  - nix develop --command cargo test --all-targets --locked
  - nix build
  - nix flake check
- Protect behavior fixes with regression tests at the useful boundaries: pure
  unit tests for algorithms/parsing, tests/ workflows for real Git and worker
  integration, and headless GPUI tests for view, input and focus interactions.
- Test supported behavior. When functionality changes or is removed, update
  existing coverage and remove obsolete checks. Do not add tests or assertions
  solely to prove removed functionality is absent; keep negative checks that
  verify current requirements, such as error handling and data protection.
- cargo test --all-targets must run all non-ignored Rust tests for the current
  platform, including GPUI, headlessly. Use deterministic temporary Git
  fixtures; keep native desktop smoke and large-history benchmarks opt-in. The
  automated [GUI smoke test](scripts/README.md) covers X11; validate Wayland
  separately.
- Keep AGENTS.md ASCII-only and concise. Do not weaken checks or claim
  completion to work around an unavailable environment.
- Use plain imperative commit subjects matching repository history, such as "Add
  branch browsing". Do not use Conventional Commit prefixes.
- Do not rewrite existing Git history without an explicit user request.
