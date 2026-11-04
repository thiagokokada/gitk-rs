# gitk-rs implementation plan

## Goal and agreed defaults

Port gitk-go to Rust/GPUI with full feature parity on Linux/NixOS. Preserve the
existing pane layout and keyboard interactions; theme changes are acceptable.
Syntax highlighting is required for v1. Keep OS integration separable for later
macOS support; macOS packaging and validation are outside v1.

The first increment is this plan and AGENTS.md. Implement subsequent milestones
progressively, leaving a runnable increment and recording evidence at each step.

- V1 prioritizes functionality and Go workflow parity. Performance numbers are
  aspirational guidance, not release or milestone gates; optimize measured
  bottlenecks after the functionality works.
- Aim for a first usable viewport around 500 ms, smooth scrolling and modest RAM.
  Keep the earlier 512 MiB aspiration for context, not a hard retention limit.
- Stream batches from Git and retain loaded rows in memory like gitk-go. Do not
  introduce SQLite, a persistent history database or parsing cache without measured
  need and a future agreed plan change.
- Default batch maximum: 1,000 commits; publish a smaller initial viewport sooner.
- Default graph display cap: 200 lanes, with explicit overflow continuation markers.
- Avoid eager all-history loading, startup counts, blocking Git/highlighting on the
  UI thread, duplicated large data, and creating widgets for every loaded row.
- Bundle broad offline syntax coverage, not a six-language allowlist. Nix, Rust,
  Go, shell, JSON and TOML are smoke cases, not the extent of supported languages.

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
a thin binary entrypoint. Separate Git, in-memory history, pure model, diff/syntax,
and GPUI view/controller modules. Do not expose GPUI types from the Git/model APIs.

The history service accepts repository identity, generation, range and query
requests. It returns nearby display rows, loaded counts, completion state and errors.
Use stable commit IDs and row ordinals rather than owning commit copies in every
UI state structure. Diff requests identify a commit or local-change kind; results
identify their generation and expose a line-addressable document plus file ranges.
All asynchronous requests support cancellation and discard obsolete results.

### Git and in-memory history

- Invoke system Git with argument arrays. Use a cancellable streaming log process
  with bounded records/stderr and pending batches; drain, terminate and reap children.
- Resolve repository root, Git/common directories and worktree identity. Support
  linked worktrees, detached HEAD, empty/unborn repositories and shallow history.
- Follow the original streaming batch model. Keep loaded rows and compact graph
  geometry in memory, publish the first viewport promptly, and load more near the
  end of the list. Pause consumption when browsing demand is satisfied.
- Retain commit data once. Avoid duplicate full messages, normalized search-text
  copies and per-widget metadata collections. Repeated author identities may be
  shared where straightforward; do not build elaborate packing/indexing machinery
  before measurements establish a need.
- Preserve trimmed, case-insensitive substring filtering over hash, author
  name/email and full message. Filter loaded rows on a worker. Like Go, continue
  streaming when there are no matches or the filtered viewport needs more results.
  Show filtered/loaded counts and distinguish a partial history from EOF.
- Keep display work virtualized. Small nearby display caches and compact match
  positions are fine; there is no database or persistent parsing cache. Loaded
  history intentionally grows with browsing demand, so record RSS at comparable
  loaded counts when evaluating Rust against Go.
- Reload/repository switching cancels obsolete work and releases old history.
  Refresh ref labels independently, preserve selection by commit ID where possible,
  and keep a stable scroll anchor as batches arrive. End means last loaded match.

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
- Use syntect on background workers with a broad compatible grammar collection,
  such as the full two-face bundle plus available default grammars. Support as many
  bundled languages as practical; no exact Chroma parity is required.
- Include filename/extension detection, known extensionless filenames and shebangs.
  Provide an actual language/alias inventory through docs and a CLI listing. Bundle
  grammars and licenses offline and load them on a worker, away from initial UI work.
- Keep Nix, Rust, Go, shell, JSON and TOML as required smoke cases. Also exercise
  representative families such as Python, JavaScript/TypeScript, C/C++, Java,
  HTML/CSS, YAML, SQL, Markdown, Ruby and other included grammars.
- Syntax foreground colors coexist with diff backgrounds. Track old/new parsing
  state separately and reset at discontinuous hunks. Unknown languages retain diff
  colors; extremely long lines may skip syntax with notice.
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
- [x] Real window opens with readable fonts on NixOS Wayland and X11.
- [x] Instrument launch/window painting and bounded CPU frame timings. Connect
  history/selection paint markers when those views exist in milestones 3 and 4.
- [x] Preserve licenses and add usage/development README.

Acceptance: nix build and checks pass; installed binary launches outside the shell.

### 3. Streamed in-memory history

- [x] Cancellable Git stream and repository discovery.
- [x] In-memory loaded rows, bounded batches and no duplicated commit collections.
- [x] Virtualized columns, graph/ref badges and incremental batch publication.
- [x] On-demand loading, progressive filtering and compact match positions.
- [x] Selection/scroll preservation, partial/EOF/error status and text graph mode.

Acceptance: streamed history, graph, navigation and filtering work like Go without
creating a history database/cache. Include a nixpkgs responsiveness smoke check;
performance measurements guide follow-up work and do not block the milestone.

### 4. Highlighted, selectable diffs

- [x] Metadata, root/first-parent diffs, file list and per-file jumps.
- [x] Virtualized line rendering, disk spool and bounded span/text caches.
- [x] Broad bundled grammars, filename/shebang detection, language inventory and
  asynchronous highlighting, tested across representative language families.
- [x] Cross-line selection and copy with/without +/- markers.
- [x] Binary, rename, unusual-path, oversized-diff and long-line handling.

Acceptance: Nix/Rust/Go highlighting works, stale results cannot replace the current
selection, and large diffs remain navigable without unbounded RAM.

### 5. Complete existing workflows

- [x] Staged/unstaged/untracked rows, including unmerged status.
- [x] Open repository, local branch dialog and protected branch switching.
- [x] Automatic reload toggle, F5/manual reload and status reconciliation.
- [x] Light/dark themes, UI/fixed-font preferences, menus, about and F1 help.
- [x] Copy commit reference, file path, selection and marker-free selection.
- [x] CLI behavior, errors and version output.
- [x] Keyboard navigation and focus behavior listed below.

Shortcut inventory: arrows/p/k and n/j move commits; Home/End navigate loaded rows;
Ctrl+PageUp/PageDown scroll history; Delete/Backspace/b/B and Space page the diff;
u/U and d/D scroll 18 lines; Ctrl+Shift+C copies without markers; / focuses filter;
Escape leaves filter or closes dialog; F5 reloads; F1 shows help; Ctrl+O opens a
repository; Ctrl+B opens branch selection; Ctrl+Q quits. Filter editing preserves
Ctrl+A/E/B/F/W/U/K/D behavior. Global navigation must not intercept text editing.
Later macOS bindings use Command where the original does.

Acceptance: parity checklist is manually exercised against the original; branch
switch failures preserve worktree contents and an actionable current view.

### 6. Release acceptance

- [x] Screenshot/layout comparison at matching dimensions and scale.
- [x] Routine checks and real Wayland/X11 smoke tests pass.
- [x] nixpkgs responsiveness smoke check; record readily available startup and
  memory observations, and track measured bottlenecks for later optimization.
- [x] Compare with Go on identical hardware/revision/workload and record results.
- [x] Document remaining functional limitations and optimization opportunities;
  missing functional parity blocks v1, aspirational timing/RSS misses do not.

## Validation protocol

Prioritize functional tests, GUI correctness and completing the workflows. Protect
fixes with a mix of pure unit tests, public-service integration workflows under
tests/ using real temporary Git repositories, and headless GPUI view/input tests
for focus/controller behavior. Standard cargo test --all-targets runs all Rust
targets without a display; native desktop smoke checks stay explicitly opt-in. Run formatting, Clippy, relevant tests, Nix checks and Wayland/X11 smoke
checks at milestones. Use small deterministic repositories for routine coverage and a local
nixpkgs smoke check to catch obvious responsiveness or memory mistakes.

The earlier numbers (roughly 500 ms first usable history, 16.7 ms steady frame
work, 100 ms selection feedback and 512 MiB RSS) remain useful aspirations only.
Do not delay v1 for profiling, exhaustive benchmarks, memory packing, indexing or
micro-optimizations when the functionality is still incomplete. Record material
bottlenecks and improve them after the basic application is functional.

When comparing performance, use release builds and equivalent loaded commit counts
and workloads in Go/Rust. Distinguish application RSS from Git child RSS, and CPU
paint callbacks from actual presentation. Record hardware, repository/source
revision and cache conditions. Comprehensive 20-launch and million-commit benchmark
runs are optional follow-up work, not v1 release gates.

Routine fixtures should cover graph merges/batch boundaries, interrupted streaming,
filtering/cancellation, stale results, HEAD changes, watcher bursts/failures,
protected branch-switch failures, worktrees, detached/unborn/shallow history,
unusual filenames, binary/renamed files, copying across virtualized lines, large
diffs and representative multiline syntax across a broad language collection.

Temporary files for exceptionally large diffs are distinct from a persistent
history database; clean them up on selection/repository change and shutdown. No
commit history or filter index should persist between runs.

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

### 2026-11-03: foundation instrumentation and visual review

Initial repository commit: b4704ff. Added --timings with a 64-record nonblocking
queue and an off-thread writer retaining at most 512 CPU render-to-paint samples.
The foundation records window creation and CPU paint callbacks; the same timing
API exposes history/selection phases to connect when those views exist. Moved
those connections to their owning milestones rather than requiring nonexistent
history views to complete the foundation.

Inspected an own-window X11 screenshot at /tmp/gitk-rs-foundation.png: readable
system fonts, expected dark panes and status bar. Both native backends had already
passed the startup smoke checks, using the same view/text system. The screenshot
test additionally exposed a shutdown borrow panic in GPUI's X11 backend. App::defer
was insufficient because its effects still run inside the native close callback;
yielding through a foreground task's 1 ms timer before quitting fixes the path.
The test now closes its own window with WM_DELETE_WINDOW and requires exit code 0.

Validation: formatting, Clippy, all four CLI tests and executable build pass.
Graceful X11 close and timing-writer shutdown pass; the latest debug run reported
WindowCreated at 179767 us and WindowPainted at 244099 us. These are CPU callback
timestamps for the scaffold, NOT history startup, GPU presentation or a 20-run
acceptance benchmark. Nix package/flake verification is rerun before committing.
Nix build and flake checks pass. The installed release package also passed the
own-window screenshot/graceful-close test, reporting WindowPainted at 103948 us
in that single warm run. Milestone 2 is complete; v1 acceptance remains open.

### 2026-11-03: revised v1 priorities requested by the user

The current design above supersedes the earlier disk-backed-history and strict
performance-gate decisions in this historical log. Use Go-style in-memory streamed
batches without SQLite or a persistent history cache. Only revisit disk storage
if measurements establish a concrete need and the change is agreed.

Broaden syntax coverage to the full practical bundled grammar set. The six initial
languages are tests, not an allowlist, and exact Chroma coverage is not required.
Document actual support and keep grammars available offline.

Functionality and workflow parity come first. Timing and memory aspirations should
prevent obviously costly designs, not block milestones or require elaborate early
optimization. Keep implementing and commit between completed milestones; stop only
when user input is genuinely required. The in-progress Git backend remains work
for milestone 3; no SQLite implementation was created.

### 2026-11-03: streamed in-memory history implemented

Foundation commit: 240c620. Plan revision commit: 525d116. Implemented the revised
history design with one resumable Git log process, byte-bounded pending batches,
single retained Commit objects shared by Arc, compact filtered positions, and a
single-slot request/result mailbox so scrolling cannot queue unlimited work.
Loaded rows grow in memory like Go; no persistent history cache or SQLite dependency
exists. Filtering does not retain another full normalized message collection.

The GUI now has a real filter, graph/commit/author/date columns, ref badges,
scrollbar, keyboard selection/page navigation, reload, first/last-loaded navigation,
and commit metadata. It uses the compatible GPUI Component 0.5.1 input widget and
keeps rendering virtualized. Filtering preserves a selected visible commit using
its original ordinal; stale request/generation results are discarded.

Git cancellation kills the process group and reaps the process, including helpers
holding stdout/stderr pipes. Tests found Git tformat's final LF after its final
NUL; handling that separator fixes EOF without accepting truncated commit records.
Errors preserve available rows and allow reload. Added graph merge/overflow tests,
Git parser/cancellation/error fixtures, annotated-tag peeling and filter/selection
tests. Fifteen tests, formatting and Clippy pass before final package validation.

Manual own-window screenshot checks on nixpkgs and this repository verified graph,
readable fonts, metadata and counts. Targeted X11 input tests exercised navigation,
filtering to the initial commit, clearing and F5 reload, then graceful close.
Temporary artifacts: /tmp/gitk-rs-history.png, /tmp/gitk-rs-filter.png and
/tmp/gitk-rs-reload.png. The debug headless history_probe loaded 1064 nixpkgs entries
in roughly 495 ms in one run; this is a smoke observation, not a performance gate.

Diffs, changed-file navigation, broad syntax highlighting and the remaining dialogs
are still pending.

### 2026-11-04: history milestone validated

The final clean flake check passes, including the package build and all fifteen
tests. The first clean check exposed Git missing from the build-time test PATH;
adding nativeCheckInputs = [ pkgs.git ] fixed it without weakening tests. Desktop
startup checks passed on Wayland/X11, and targeted X11 screenshots/input verified
filtering, clearing, selection and reload. Milestone 3 is complete. Continue with
the diff viewer and broad bundled syntax coverage; performance tuning is deferred.

### 2026-11-04: highlighted, selectable diff viewer

Implemented root and first-parent commit diffs, metadata, virtualized changed-file
lists and line jumps. Selected diff text lives in an unlinked temporary file with
line offsets; only nearby text/spans enter the UI. Limits are explicit: 256 MiB or
one million lines per preview, 16 KiB displayed per line and 16 MiB per clipboard
selection. Copy reads original text across unloaded lines and preserves tabs and
Unicode. Marker-free copy removes code markers while keeping diff headers intact.

Bundled two-face 0.5.2+bat-0.26.1 with syntect's Rust regex backend: 213 offline
syntaxes, documented in docs/LANGUAGES.md and exposed by --list-languages. Bundled
syntax/theme notices are included in assets/licenses and installed by Nix.
Filename/extension/shebang detection and separate old/new multiline parsing run on
the diff worker. Hunk gaps reset parsing state; unknown/oversized lines remain
readable. --nosyntax disables highlighting. Typed selection IDs, request IDs and
cancellation prevent obsolete diff/copy results from replacing current selection.

Twenty-six tests pass, including root/merge diffs, binary and renamed files,
unusual filenames, long Unicode lines, copying outside a loaded page, cancellation,
and actual colored tokens across seventeen language families. Formatting and
Clippy pass. Own-window X11 screenshots verify readable metadata, Go syntax colors,
selection and scrolling across added/removed lines. A GUI check found and fixed
GPUI's non-strict scrolling behavior for file jumps/pages. Native XTest keys work;
XSendEvent keys were ignored. KWin/Xwayland pointer-warp requests did not move the
pointer, so the automated file-list click check is inconclusive; file range parsing
and strict navigation are checked in code. No successful pointer test is claimed.
The installed package remains alive on Wayland/X11 for eight seconds each and
X11 WM_DELETE_WINDOW exits cleanly. Clean nix flake check of the final scroll fix passes, including the packaged
build and all twenty-six tests. nix build passed before that final positioning
adjustment; its rebuilt package is also produced by the final flake check.
Milestone 4 is complete. The next milestone covers local changes, dialogs,
preferences, menus and automatic reload.

### 2026-11-04: remaining workflows implemented, final checks pending

Milestone 4 commit: 6b286b1. Implemented staged/unstaged/untracked rows, conflict
status and ordinary protected git switch operations. Local diffs include untracked
files and compare conflicted worktree entries against ours; duplicate unmerged
numstat entries are collapsed. The repository worker handles status, refs, branch
listing/switching and a bounded nonrecursive watch set away from rendering.
Events coalesce for 350 ms; five-second reconciliation covers unwatched locations
and watcher failures. F5 remains independent of watching, and --nowatch/--verbose
are available. Ref labels refresh without discarding retained history; reload
preserves nearby selection by commit ID where possible.

Added a native repository picker, searchable local-branch dialog, menus, copy
reference/path/selection actions, F1/About, light/dark switching, resizable panes,
and atomic font preferences in gitk-rs's own config directory. Original Emacs
filter controls are scoped to input focus. Bundled GPUI Component 0.5.1 icons and
included their Apache/Lucide/Feather notices. Split dialog logic into its own module.

Thirty-two tests, formatting, Clippy and the development build pass before the
last layout/focus checks. New fixtures cover staged/unstaged/untracked/renamed and
unmerged status, protected switch failures preserving contents, watcher event
bursts, disabled-watcher manual refresh, repository generation changes and atomic
preferences. A disposable-repository X11 GUI script verifies local Nix/Rust and
staged Go diffs, a rejected switch with unchanged contents followed by a successful
switch, help, isolated preference saving, theme switching, filtering and reload.
Screenshots include /tmp/gitk-rs-local-nix.png, /tmp/gitk-rs-branch-protected.png,
/tmp/gitk-rs-preferences.png and /tmp/gitk-rs-light-theme.png. No user repository or
preferences were modified by these interaction tests. Final clean Nix and native
backend validation are pending; milestone 5 remains open until they pass.


### 2026-11-04: workflow milestone validated

Final formatting, Clippy and all thirty-two tests pass. Clean nix flake check and
nix build pass; installed package /nix/store/jnb6narwwc49nhpggp9867brcdbma4dr-gitk-rs-0.1.0
launches outside the shell on Wayland and X11 and remains alive for eight seconds
on each backend. Its help/version/language inventory commands also pass.

The final X11 fixture test additionally exercises branch filtering and opening an
alternate repository by path, asserting its branch in the verbose state log and
inspecting its actual history/diff. It exposed and fixed a stale repository path
in history requests. Closing a successful branch dialog now restores keyboard
focus. Native file-picker failure offers a path-entry fallback, also available
with Ctrl+Shift+O. Last desktop artifacts include /tmp/gitk-rs-repository-opened.png
and /tmp/gitk-rs-branch-filtered.png. Native picker selection itself has not been
automated; the path-opening workflow and repository transition are verified.

Running target/debug/gitk-rs outside nix develop failed with NoWaylandLib and
missing libvulkan.so.1, as expected for an unwrapped development binary. Repeating
inside the pinned shell passes; the installed wrapper passes outside it. No host
library or permission settings were changed. Milestone 5 is complete. Continue to
release acceptance, same-machine reference observations and remaining limitations.

### 2026-11-04: release acceptance in progress

Workflow milestone commit: dccd6ac. Added checked-in own-window GUI and release
observation scripts, comparison screenshots and docs/VALIDATION.md. The scripts
use isolated temporary repositories/config/cache locations; native input is guarded
by window focus, and the comparison probe verifies XRes client ownership before
resizing/capturing windows. This matters because the Go window lacks _NET_WM_PID.

Release inspection added fixtures for linked worktrees, shallow clones and
conflicted diff navigation, and isolated test hooks/fsmonitor. It also found a
wide-frontier display issue: canvas lanes now fit using shared visible-row spacing;
the bounded text fallback keeps its current node and continuation indicators in
view, respecting the chosen fixed font. Concurrent native picker requests are
bounded to one. Thirty-seven tests, formatting, Clippy and the development build
pass; final clean Nix and installed-runtime checks are in progress.

Two short, matched-size X11 observations on the same nixpkgs HEAD and 192 loaded
commits recorded roughly 76-82 MiB Rust application RSS versus 50-54 MiB Go RSS.
Sampled child-process memory was recorded separately. These are first-view smoke
observations, not full-history memory retention or presentation benchmarks; timing
events differ between applications. See docs/VALIDATION.md and raw records for
methodology, source/store revisions, cache conditions and limits. Performance
numbers remain aspirations, not release gates.

Release regression follow-up: the GUI script now asserts that a selected ancestor
stays selected after switching branches. Removed a duplicate HEAD-change reload
that could replace the pending selection. Horizontal diff extent now accounts for
expanded tabs so tab-heavy line endings remain reachable; its regression brings
the suite to thirty-eight tests. All tests, formatting, Clippy and the executable
build pass. A clean package recheck of these final fixes is running.


Testing follow-up requested by the user: added tests/workflows.rs as a separate
Cargo integration target. Its four public-service workflows cover history demand
and progressive filtering/repository generations, history-to-highlighted-diff and
copy across unloaded lines, protected branch/status operations through the worker,
and repository opening from a subdirectory. The subdirectory test first failed
with "root-level untracked file is missing"; anchoring diff commands at Git's
discovered root fixes it. All 38 library tests and all 4 integration workflows
now pass with DISPLAY and WAYLAND_DISPLAY removed (42 total). The optional GUI
script additionally asserts selected-ancestor preservation after branch switching.
AGENTS.md now requires this layered regression approach for future behavior fixes.


### 2026-11-04: code-driven GPUI regression coverage

Verified the pinned GPUI 0.2.2 test-support API and enabled it only as a development
dependency. Four #[gpui::test] tests now construct the actual Root/HistoryWindow
and DiffPane on TestAppContext/VisualTestContext, sharing production key bindings.
They drive navigation and filter editing, rejected/successful branch switching
with selected-ancestor preservation, repository path input with restored focus,
and a file-list mouse click followed by full-diff clipboard copies across unloaded
lines (including expanded tabs). No Python, xdotool, display server or GPU is
required. Git work runs through real workers against isolated temporary fixtures;
UI timers use GPUI's virtual clock with bounded waits for external worker results.

Formatting, strict Clippy and cargo test --all-targets --locked pass with DISPLAY
and WAYLAND_DISPLAY unset: 38 library, 4 public-service and 4 GPUI tests (46 total).
Python tools remain optional native-backend smoke/measurement tools. AGENTS.md and
validation documentation now require headless GPUI coverage for controller/focus
regressions. Clean Nix package verification is in progress.

### 2026-11-04: release acceptance complete

All milestone 6 acceptance checks pass. Clean nix flake check validates formatting
and the package, with all 46 Rust tests passing inside the Nix build as well as
in the display-free development run. Package build log:
/tmp/gitk-rs-gpui-package.log. The tested installed output is
/nix/store/zfpwsazjbljw8n87h69xq0nx34ps8y40-gitk-rs-0.1.0; nix build of the checked
derivation links it as result. Subsequent source edits only finalize these docs.

The installed wrapper stays alive for eight seconds each on real Wayland and X11,
with no startup failure. The separate native X11 fixture workflow passes and exits
cleanly; its screenshots/log are in /tmp/gitk-rs-gpui-release-native. Version and
213-language inventory commands pass. No native picker automation, native Wayland
input coverage, macOS/aarch64 execution, pixel-level headless rendering assertion
or million-commit retention benchmark is claimed. These verification boundaries
and functional resource caps are documented in docs/VALIDATION.md.

Matched-size Go/Rust screenshots, source/store revisions, same-machine nixpkgs
observations and remaining optimization opportunities are documented. Performance
observations are not release gates. The first Linux release is ready to run with
nix run . -- /path/to/repository; all planned milestones are complete.


### 2026-11-04: correct Nix launch instructions

The previously suggested nix run command omitted the explicit flake installable.
Nix consequently treated the repository argument as the flake to run, even when
invoked from gitk-rs. Corrected it to nix run . -- /path/to/repository and added
README examples for both the checkout and another working directory. This was
a documentation/command error, not evidence of a caller working-directory error.

Validation: local documentation links, command consistency, whitespace and AGENTS
ASCII checks pass. nix eval --raw .#apps.x86_64-linux.default.program resolves to
the gitk-rs executable. The explicit-flake version probe triggers a package build;
no application source changed in this correction.
