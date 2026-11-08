# Incremental history reload

Reload previously replaced the history worker session, including all retained
commits. Retain in-memory history whenever the new repository state can safely
use it. No persistent cache is needed.

## Milestones

- [x] Retain history when the repository and HEAD are unchanged. Refresh branch
  labels and refs, preserve completed searches and an unfinished Git stream, and
  still reject snapshots from obsolete UI generations. Repository changes,
  graph configuration changes, and failed sessions must start fresh.
- [x] Handle a bounded linear extension of HEAD by reading only new commits.
  Verify every parent link back to the previous HEAD before changing retained
  state. Preserve the old stream for partially loaded history, update ordinals,
  adjust the first retained graph row, and reconcile search and restore state.
  Fall back to a full reload for merges, divergence, rewinds, unborn HEAD,
  or incompatible traversal metadata. The initial size cap is removed below.
- [x] Validate manual and automatic reload, selection and viewport bookmarks,
  filtering, partial history continuation, stale snapshots, and fallbacks with
  deterministic real Git fixtures and headless GPUI tests. Record limitations
  and commit each validated implementation milestone.

## Removing the total delta limit

- [x] Replace the total 256-commit / 2 MiB probe budget with a resumable worker
  phase. Read and prepare graph rows in bounded steps, retaining the provisional
  new history once, and verify the entire chain before changing the old suffix.
  Process new requests between steps and discard superseded verification.
- [x] Reconcile large prefixes with bounded metadata/path-search batches. Keep
  completed suffix decisions, retry only its bounded in-flight candidates, and
  avoid a full reload merely because the prefix contains many candidates.
- [x] Cover updates beyond both old budgets, partial-history continuation,
  cancellation and supersession, fallback after several probe steps, and UI
  bookmark preservation. Validate and commit with the pinned toolchain.

The follow-up removes size-based fallbacks; nonlinear histories still fall back
to normal loading. Verification yields after 256 commits or reaching a 2 MiB
accounted-data threshold in each worker step. Individual Git records remain
bounded separately. New commit records are retained once as prospective history,
then moved into the final index after the full chain has been verified.

Prefix filters run in batches of at most 1000 commits with the existing bounded
path-search queues. Their matches are joined with completed suffix matches once,
instead of repeatedly inserting ahead of a large suffix. Only bounded in-flight
suffix path candidates are retried. Snapshots wait for verification and prefix
filtering to finish so restored bookmarks address the complete reconciled list.
A reload superseding unfinished prefix filtering restarts the search over
retained commits; a newer query similarly replaces unfinished search work.

## Safety and cost

Keep Git and retained-history work on the history worker. Snapshot generations
remain independent of the lifetime of a reusable Git stream. Cancel and reap
the old stream when it is replaced or the worker stops. Ordinary streaming uses
bounded worker steps. Reloads of the same repository wait for the current step;
they must not cancel the stream before the worker can decide whether to retain
it.

Only accept linear extensions, where the new commits form an unbroken chain
ending at the old HEAD. This avoids changing the ordering or graph lanes of the
retained suffix. There is no total delta commit, byte, or path-candidate limit
that causes a full reload. Work is divided into bounded steps instead.
Reindexing retained entries still takes O(loaded commits) worker CPU in one pass,
which can delay subsequent worker requests. It does not reparse old commits or
duplicate the entire retained history. Existing UI references may require
copying the small referenced subset of entries during reindexing. Completed
path-search decisions survive; only bounded in-flight candidates are retried.

Changes to replacement refs, grafts, or shallow boundaries can alter traversal
without changing HEAD. Detect these before reuse, or conservatively disable
reuse when the relevant traversal metadata exists. Keep full reload available
for unsupported cases. General merge reconciliation and benchmarks are follow-up
work; no timing or memory improvements are claimed without measurements.

## Validation

Use the pinned Nix toolchain for format, clippy, all Rust tests (including
headless GPUI), build, and flake checks as required by AGENTS.md. Native Wayland
and X11 smoke tests and large-repository memory probes remain opt-in.

The unchanged-HEAD milestone passed all-target tests (151 passed, three opt-in
tests ignored) and clippy with warnings denied. Tests cover retained entry
identity, an unfinished traversal, completed metadata search, ref labels,
replacement refs, grafts, shallow boundaries, graph configuration replacement,
generation rejection, and F5 selection restoration.

The linear-extension milestone passed all-target tests (155 passed, three
opt-in tests ignored) and clippy with warnings denied. Incremental results match
fresh traversal for commit order, ordinals, graph geometry (including an old
merge and lane caps), metadata and path searches, bookmarks, repeated updates,
and partial history resumed to EOF. Unsupported probes leave old entries
untouched before the worker replaces the session.

Initial implementation validation passed:

- `nix develop --command cargo fmt --all -- --check`
- `nix develop --command cargo clippy --all-targets -- -D warnings`
- `nix develop --command cargo test --all-targets --locked`: 157 passed, three
  opt-in tests ignored, including 48 main-target tests with headless GPUI.
- `nix build`
- `nix flake check`: all checks passed on x86_64-linux; incompatible
  aarch64-linux and aarch64-darwin checks were omitted by Nix.
- ASCII, local Markdown links, checklist consistency, and whitespace checks.

The deep UI workflow also verifies that retained author identities survive both
F5 and automatic reload, alongside existing selection and viewport assertions.
Public worker workflows cover incremental extension followed by merge and rewind
fallbacks, and same-HEAD retry after repairing a failed path search. Failed
queries deliberately start a fresh session on reload so errors remain retryable.

Nix cache access and Git index writes required approved execution outside the
sandbox, as recorded in AGENTS.md. Native Wayland/macOS/X11 smoke tests and
large-repository probes were not run. General merge reconciliation remains
deferred, and no nixpkgs timing or RSS measurements have been collected.

Resumable verification and bounded prefix filtering passed all-target tests
(163 passed, three opt-in tests ignored), formatting, and clippy with warnings
denied. Coverage includes updates larger than 256 commits and 2 MiB, a
9000-commit path-filtered prefix, old partial streams, superseded/cancelled
verification, late divergence, interrupted searches, public worker bookmarks,
and 600-commit manual and automatic UI reloads. `nix build` and `nix flake check`
also passed for this follow-up on x86_64-linux; Nix omitted incompatible
aarch64-linux and aarch64-darwin checks. Native desktop smoke tests and
large-repository benchmarks remain unrun.

All prior reload commits and this follow-up are retained on the local
`incremental-history-reload` branch. Local `main` was restored to `2afe1ac`, its
pre-reload tip, matching the locally recorded `origin/main`.
