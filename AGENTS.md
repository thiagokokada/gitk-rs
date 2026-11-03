# Contributor instructions

Read PLAN.md before changing this repository. Implement milestones in order and
update their checklist and progress log with evidence after each increment.
Do not mark a milestone complete until its acceptance checks pass. Report blocked
checks and unfinished behavior explicitly; never invent benchmark results.
Commit each validated milestone, and continue with the next authorized milestone
without waiting for confirmation. Stop only when user input is actually required.

## Architecture

- Use Rust and GPUI. Linux/NixOS is the first supported platform. Isolate OS
  integration so Git, storage, models, and most UI logic remain portable to macOS.
- Keep Git process management and disk storage independent of GPUI. Keep state
  transitions and display preparation pure where possible. Avoid abstraction-only
  wrappers and speculative backend/plugin systems.
- Use typed IDs and generation tokens for asynchronous work. Reject stale results
  after selection, query, repository, HEAD, or theme changes.
- Invoke Git with argument arrays, never shell interpolation. Bound stderr capture,
  drain pipes without deadlocks, and terminate and reap children on cancellation.
- Branch switching must preserve Git's ordinary protections. Never automatically
  force checkout, stash, reset, or discard changes.

## Performance invariants

- Never run blocking Git, SQLite, filesystem scans, or highlighting in render paths
  or on the UI thread. Never wait for a total history count before showing rows.
- Virtualize history and diff lines. Paint graph geometry only for visible rows.
- Bound queues, caches, pending work, and retained text by bytes. Do not accumulate
  all commits, graph rows, search matches, or diff spans in application memory.
- Use disk-backed ordinal paging, not deep SQL OFFSET queries or repeated Git
  --skip scans. Stop prefetching when browsing demand is satisfied.
- Avoid whole-worktree recursive watcher expansion on nixpkgs. Coalesce events and
  reconcile status periodically; watch failure must leave manual reload usable.
- Record before/after measurements for performance changes. Test both cold app
  indexes and warm reopen. Report application and child-process memory separately.

## Dependencies, verification, and documentation

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
- Add meaningful tests for parsing, paging, state transitions, cancellation, and
  failure handling. Use small deterministic Git fixtures in normal checks; keep
  million-commit benchmarks opt-in. Smoke-test real Wayland and X11 separately.
- Run checks relevant to the increment. Documentation-only changes need link,
  checklist, consistency, and ASCII checks, not application tests.
- Keep AGENTS.md ASCII-only and concise. Document observed sandbox, network, build,
  and display limitations with the command and actual error. Do not weaken checks
  or claim completion to work around an unavailable environment.
- Do not rewrite existing Git history without an explicit user request.
