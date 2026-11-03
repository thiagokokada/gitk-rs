# gitk-rs implementation plan

## Goal and agreed defaults

Port gitk-go to Rust/GPUI with full feature parity on Linux/NixOS. Preserve the
existing pane layout and keyboard interactions; theme changes are acceptable.
Syntax highlighting is required for v1. Keep OS integration separable for later
macOS support; macOS packaging and validation are outside v1.

The first increment is this plan and AGENTS.md. Implement subsequent milestones
progressively, leaving a runnable increment and recording evidence at each step.

- First usable history: p95 <=500 ms with no application history cache.
- History storage: disposable disk index with bounded RAM.
- Indexing: on demand, with nearby prefetch. Filtering may scan through EOF.
- Memory target: application RSS <512 MiB, including during full-history search.
- Disk budget: 4 GiB per repository, including sidecars and temporary history data.
- Default batch maximum: 1,000 commits; publish the first viewport sooner.
- Default graph display cap: 200 lanes with visible overflow continuation markers.
- No startup full-history count and no all-history RAM collection.

## Reference inventory

Inspected source: ../gitk-go at fc472ac7040d6982daa123c3577e684a198ad9b4.
Visual reference: ../gitk-go/screenshot.png. Inspect this file before UI changes;
it is a reference outside this repository, not a shipped runtime dependency.
Benchmark repository: ../nixpkgs at a4d1fc4e93babff98f0f660a7bf290d8a4d12840.
Record revisions again for each benchmark because these checkouts may change.

Source areas to consult while porting:

| Source in gitk-go | Behavior |
| --- | --- |
| cmd/cmd.go | Repository argument, CLI defaults, theme environment variable |
| internal/git/backend | Git commands, streamed log protocol, status and refs |
| internal/git/service.go and scan.go | List formatting, search fields, graph state |
| internal/gui/model | Selection, filtering, scroll and local-change state |
| internal/gui/view and widgets | Layout, dialogs, graph, menus, text selection |
| internal/gui/shortcuts.go | Key bindings and focus rules |
| internal/gui/preferences.go and internal/watch | Font persistence and reload |

The Go log reader deliberately avoids date/topological ordering that can postpone
first output on large repositories. Preserve its default HEAD-reachable traversal
order. Port behavior and relevant fixtures rather than translating Tk plumbing.

## Design

### Modules and interfaces

Use one Cargo package initially, with a library for testable application code and
a thin binary entrypoint. Separate Git, history storage, pure model, diff/syntax,
and GPUI view/controller modules. Do not expose GPUI types from the Git/store APIs.

The history service accepts repository identity, generation, range and query
requests. It returns bounded pages, indexed counts, completion state and errors.
Use stable commit IDs and row ordinals rather than owning commit copies in every
UI state structure. Diff requests identify a commit or local-change kind; results
identify their generation and expose a line-addressable document plus file ranges.
All asynchronous requests support cancellation and discard obsolete results.

### Git and storage

- Invoke system Git directly with argument arrays. Use a cancellable streaming log
  process with bounded record/stderr handling; drain, terminate and reap children.
- Resolve repository root, Git/common directories and worktree identity. Support
  linked worktrees, detached HEAD, empty/unborn repositories and shallow history.
- Stream metadata into SQLite using rusqlite, batched transactions and bounded
  channels. Store ordinal, commit data, searchable text and compact graph geometry.
  Store full diff contents separately, never in the history index.
- Page by indexed ordinal ranges. Bound SQLite page caches and application caches
  by bytes; avoid unbounded mmap and memory proportional to history or match count.
- Publish initial rows before cache initialization or ancillary work can delay the
  viewport. Prefetch roughly two viewports around visible rows, then pause reads.
- Preserve trimmed case-insensitive substring search over hash, author name/email
  and full message. Use normalized text and incremental background scans, not FTS
  token matching that would change substring semantics. Persist result positions
  on disk and serve result ordinal ranges. Show partial versus complete results.
- Cache timelines under the platform cache directory, keyed by repository/worktree,
  resolved HEAD, traversal settings and schema version. Refresh ref badges
  independently. Changed/incompatible timelines rebuild lazily; v1 needs no
  complicated history-splicing algorithm or durable cache schema migration.
- Prune inactive timelines first. At the disk budget or on write failure, stop
  extension, preserve available browsing and expose a recoverable error. Never
  silently fall back to unbounded RAM. Exclude disposable data from watchers.
- Serialize conflicting repository operations and cancel old jobs on switch/reload.
  Preserve selection by commit ID where possible; retain a stable scroll anchor
  as pages arrive. End navigates to the last currently indexed matching row.

### GPUI, diffs and platform integration

- Pin published GPUI 0.2.2 and consult its matching docs. Do not mix APIs from the
  current Zed development branch into the published release.
- Preserve the toolbar, upper graph/commit/author/date table, lower resizable
  diff/file-list split, scrollbars and bottom status bar. Virtualize both text
  and history; draw only visible graph geometry and ref badges.
- Keep metadata, refs, status, watchers, initial diff loading and syntax setup off
  the critical path to selectable history. Use bounded workers and generation IDs.
- Commit details include author/committer, timestamps and full message. Root commits
  show additions; merges show first-parent diffs. Retain per-file jump navigation.
- Parse and spool large diffs to temporary disk files with line offsets. Fetch
  nearby text and spans only. Selection and copying must span unloaded lines.
- Use syntect in background workers. Bundle licensed grammars including Nix, Rust,
  Go, shell, JSON and TOML. Syntax foreground colors coexist with diff backgrounds.
  Track old/new parser state separately and reset at discontinuous hunks. Unknown
  languages retain diff colors; extremely long lines may skip syntax with notice.
- Keep ordinary local branch switching protections. Never auto-stash or force a
  checkout. Display actionable Git errors and leave the current view recoverable.
- Use notify with debouncing for Git metadata and relevant worktree locations;
  avoid recursively registering nixpkgs directories. Periodically reconcile local
  status and handle fsmonitor, watcher failures and event bursts gracefully.
- Persist UI/monospace font preferences in gitk-rs configuration, separate from Go.
  Support light/dark themes and isolate platform paths, dialogs and modifier keys.

### Nix and command line

- Pin nixpkgs in flake.lock and use its Rust toolchain consistently. Commit
  Cargo.lock. Supply development shells, package/app outputs and flake checks for
  x86_64-linux and aarch64-linux; validate the host architecture first.
- Package with buildRustPackage and the committed Cargo lockfile. Expose required
  font, Wayland/X11 and Vulkan libraries in both development and installed runtime
  environments. Do not hardcode a vendor GPU driver into the package.
- Preserve equivalent CLI controls for repository path, batch limit, graph columns,
  text graph, theme, syntax, watcher, verbose logging and version. Use Rust-style
  flag spelling and document it; Go flag/env/config compatibility is not required.
- Keep native dependencies conditional for eventual macOS. Preserve original MIT
  notices when porting code and include licenses for bundled assets/grammars.

## Milestones and parity checklist

### 1. Documentation

- [x] Create PLAN.md and AGENTS.md.
- [x] Inspect Go source, screenshot, shortcuts and Git traversal behavior.
- [x] Record defaults, source revisions, acceptance criteria and environment limits.

Acceptance: documents are ASCII, coherent, and leave subsequent work tracked.

### 2. Runnable foundation

- [x] Cargo package, pinned GPUI and Cargo.lock.
- [x] Nix flake, lockfile, shell, package, app and checks.
- [ ] Real window opens with readable fonts on NixOS Wayland and X11.
- [ ] Instrument launch, first painted history, selection and frame timings.
- [x] Preserve licenses and add usage/development README.

Acceptance: nix build and checks pass; installed binary launches outside the shell.

### 3. Streamed, bounded history

- [ ] Cancellable Git stream and repository discovery.
- [ ] SQLite timeline, ordinal paging, disk budget and bounded caches.
- [ ] Virtualized columns, graph/ref badges and incremental batch publication.
- [ ] On-demand prefetch, progressive filtering and disk-backed match positions.
- [ ] Selection/scroll preservation, partial/EOF/error status and text graph mode.

Acceptance: deep browsing and full search do not grow RAM with total commit count;
startup target is measured before moving expensive work into the startup path.

### 4. Highlighted, selectable diffs

- [ ] Metadata, root/first-parent diffs, file list and per-file jumps.
- [ ] Virtualized line rendering, disk spool and bounded span/text caches.
- [ ] Required language grammars and asynchronous highlighting.
- [ ] Cross-line selection and copy with/without +/- markers.
- [ ] Binary, rename, unusual-path, oversized-diff and long-line handling.

Acceptance: Nix/Rust/Go highlighting works, stale results cannot replace the current
selection, and large diffs remain navigable without unbounded RAM.

### 5. Complete existing workflows

- [ ] Staged/unstaged/untracked rows, including unmerged status.
- [ ] Open repository, local branch dialog and protected branch switching.
- [ ] Automatic reload toggle, F5/manual reload and status reconciliation.
- [ ] Light/dark themes, UI/fixed-font preferences, menus, about and F1 help.
- [ ] Copy commit reference, file path, selection and marker-free selection.
- [ ] CLI behavior, errors and version output.
- [ ] Keyboard navigation and focus behavior listed below.

Shortcut inventory: arrows/p/k and n/j move commits; Home/End navigate indexed rows;
Ctrl+PageUp/PageDown scroll history; Delete/Backspace/b/B and Space page the diff;
u/U and d/D scroll 18 lines; Ctrl+Shift+C copies without markers; / focuses filter;
Escape leaves filter or closes dialog; F5 reloads; F1 shows help; Ctrl+O opens a
repository; Ctrl+B opens branch selection; Ctrl+Q quits. Filter editing preserves
Ctrl+A/E/B/F/W/U/K/D behavior. Global navigation must not intercept text editing.
Later macOS bindings use Command where the original does.

Acceptance: parity checklist is manually exercised against the original; branch
switch failures preserve worktree contents and an actionable current view.

### 6. Release acceptance

- [ ] Screenshot/layout comparison at matching dimensions and scale.
- [ ] Routine checks and real Wayland/X11 smoke tests pass.
- [ ] nixpkgs startup, scrolling, selection, full search and memory measured.
- [ ] Compare with Go on identical hardware/revision/workload and record results.
- [ ] Document remaining limitations; do not call v1 complete with missing parity
  or failed acceptance targets.

## Validation protocol

Use release builds. On the reference machine measure 20 process launches, each
without the Rust history index. Target p95 <=500 ms from process launch to the
first painted, selectable history viewport with keyboard navigation working.
Measure warm reopening and cold OS filesystem conditions separately; do not drop
system caches during routine automation. Exclude build time, include GUI startup.
Initial diffs, exact totals, refs and worktree status need not finish within 500 ms.

At 60 Hz target steady-scroll p95 frame time <=16.7 ms and selection feedback
<=100 ms. Report diff/highlight completion separately. Target app RSS <512 MiB
through full-history search and deep browsing. Record Git child RSS, aggregate
peak process memory, cache/spool disk use and workload duration separately.
These are acceptance targets, not claims that performance has been achieved.

Store benchmark commands and results with hardware, OS/display/GPU, repository and
source revisions, toolchain, viewport size, cache state and sample count. Include
Go comparisons. Track startup phases rather than using first Git output as a
proxy for an interactive window. Keep million-commit benchmarks opt-in.

Routine fixtures must cover graph merges/page boundaries; interrupted indexing;
partial searches/cancellation; stale results; schema/HEAD changes; full/read-only
disks; watcher bursts/failures; protected branch-switch failures; worktrees;
detached/unborn/shallow history; Unicode/non-UTF8 paths; binary/renamed files;
cross-page copying; huge diffs and representative multiline syntax.

Once the toolchain exists, run the commands in AGENTS.md. Do not add expensive
GUI/repository benchmarks to every unit-test run.

## Progress log

### 2026-11-03: documentation milestone

Inspected the Go source and screenshot and recorded the accepted design. The
workspace initially contained no application files or usable Git metadata.

A read-only local Git probe read the first 100 complete commit records from
nixpkgs in 79.9 ms on the first run and 3.2-3.5 ms on four subsequent runs. This
excludes GUI startup and is only feasibility evidence, not an acceptance result.

Environment blockers for milestone 2:

- cargo and rustc are absent from PATH; no installed compiler was located.
- Initial nix eval could not open the default user cache database. Retrying with
  XDG_CACHE_HOME=/tmp/gitk-rs-nix-cache avoided that cache permission problem.
- Fetching pinned nixpkgs revision 9fbb54b33e91ee4ca368e35a78e0613c720600b3
  then failed because github.com could not resolve.
- An attempted local nix-shell fallback failed to connect to
  /nix/var/nix/daemon-socket/socket with Operation not permitted.

Next: establish a usable Rust/Nix environment, then implement milestone 2. No
application, package-build, GUI or performance acceptance checks have passed yet.

### 2026-11-03: foundation scaffold written, verification blocked

Added Cargo metadata pinned to GPUI 0.2.2, separate library/binary entrypoints,
OS-string CLI parsing and tests, light/dark window shell, quit handling, and Linux
flake shell/package/app/check definitions. The shell is independent of the package
to allow lockfile bootstrapping. Reused the original project's pinned nixpkgs lock
and MIT notice. Added README with honest status and bootstrap commands.

The window is a scaffold with explicit pending-history messaging, not a functional
history viewer. Timing instrumentation, actual history, diffs and all later
milestones remain outstanding. Milestone 2 remains unchecked until verified.

Rechecked environment: nix-shell -p cargo rustc still receives Operation not
permitted connecting to the daemon. The socket is world-writable; ordinary file
mode does not explain denial. The process has an active seccomp filter. curl to
index.crates.io fails DNS lookup, and no Cargo/Rust compiler is available in PATH.
No escalation is available under this session's permission policy.

Validation: nix-instantiate --parse flake.nix passes. Manifest/lock JSON parsing
and consistency checks pass. Rust formatting, compilation, tests, Cargo.lock
generation, packaged runtime, and Wayland/X11 checks have NOT run. Cargo.lock was
not fabricated; the package emits an explicit bootstrap instruction if absent.

Next: restore execution access to the Nix daemon and dependency network; run the
README bootstrap commands, commit Cargo.lock, fix any compiler/native dependency
issues, then finish milestone 2 before proceeding to disk-backed history.

### 2026-11-03: execution exception restores Nix access

The updated session permits reviewed execution exceptions. The same nix develop
command still fails inside the sandbox, but succeeds with require_escalated.
Nix downloaded the pinned toolchain and native libraries; cargo reports 1.97.0.
Using CARGO_HOME=/tmp/gitk-rs-cargo and
XDG_CACHE_HOME=/tmp/gitk-rs-nix-cache keeps tool caches in explicit task paths.

Generated Cargo.lock from the registry and formatted application sources. The
first compilation found xattr 0.2.3 (via GPUI's zed-async-tar dependency) using the
removed Linux libc::ENOATTR alias. Vendored that small crate with upstream license
files and substituted ENODATA on Linux/Android; all other platforms are unchanged.
See vendor/README.md for patch provenance and removal criteria.

Compilation then caught an invalid WindowOptions.title field in the unverified
scaffold. Replaced it with GPUI 0.2.2's window.set_window_title API. Updated
deprecated X11 package names and removed the missing-lockfile bootstrap branch.

Validation so far: cargo fmt --all passes; cargo clippy --all-targets --locked
-- -D warnings passes. Full unit-test build is in progress. Packaged build,
flake checks, real Wayland/X11 launch and instrumentation remain outstanding.
No GUI startup or memory acceptance results are claimed.

### 2026-11-03: package checks and desktop startup verified

The first development-window launch failed in Vulkan initialization. Retrying
with VK_LOADER_DEBUG=error,warn showed that the host Mesa/LLVM driver requires
GLIBC_2.44 but the August nixpkgs pin inherited from gitk-go supplied glibc 2.42.
Updated flake.lock to the host nixpkgs revision
6510408d147d8c5e6d76043e899e9251756f6a75, obtained from the installed NIX_PATH
source's .git-revision. This remains a remote, reproducible pin, with no dependency
on the local nixpkgs checkout. Its Rust toolchain is 1.98.1.

Rechecked with that final pin:

- cargo fmt --all -- --check: passes.
- cargo clippy --all-targets --locked -- -D warnings: passes.
- cargo test --all-targets --locked: all four CLI tests pass.
- cargo build --locked: passes.
- nix flake check path:. --cores 4: passes, including format and package checks.
- nix build path:. --cores 4: passes.
- Packaged --help and --version: pass outside nix develop.
- Development and packaged executables on Wayland and X11: each remained alive
  for eight seconds without startup failure, then the test terminated its own
  process. Installed package tested:
  /nix/store/l19sd9xydjxyknpv8pybrn11ysdbs2mq-gitk-rs-0.1.0.

Normal compilation and the Nix package both use the checked-in xattr patch. Nix
package sources explicitly exclude target and .cache directories. Build logs:
/tmp/gitk-rs-dev-check.log and /tmp/gitk-rs-flake-check.log. Startup smoke logs:
/tmp/gitk-rs-wayland-smoke.log and /tmp/gitk-rs-x11-smoke.log. These are temporary
local artifacts, not dependencies required to build the project.

Only x86_64-linux was built; flake check explicitly omitted aarch64-linux. The
desktop smoke tests establish successful startup, not a visual font/layout review
or a measured first painted viewport. The GPU driver prints a non-conformance
warning, but both backends stayed running. A transitive proc-macro-error2 crate
also produces a compiler future-incompatibility notice; root Clippy still passes.

Milestone 2 remains open for visual inspection and timing instrumentation. The
window is still a scaffold. No history, diff, syntax-highlighting, memory or
500 ms startup acceptance is claimed. The earlier daemon/network blocker is
resolved through reviewed execution exceptions; it is not a remaining blocker.
