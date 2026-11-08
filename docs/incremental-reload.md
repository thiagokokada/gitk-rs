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
  excessive delta size, or incompatible traversal metadata.
- [ ] Validate manual and automatic reload, selection and viewport bookmarks,
  filtering, partial history continuation, stale snapshots, and fallbacks with
  deterministic real Git fixtures and headless GPUI tests. Record limitations
  and commit each validated implementation milestone.

## Safety and cost

Keep Git and retained-history work on the history worker. Snapshot generations
remain independent of the lifetime of a reusable Git stream. Cancel and reap
the old stream when it is replaced or the worker stops. Reloads of the same
repository are handled between bounded worker steps; they must not cancel the
stream before the worker can decide whether to retain it.

Only accept linear extensions, where the new commits form an unbroken chain
ending at the old HEAD. This avoids changing the ordering or graph lanes of the
retained suffix. Speculative delta loading is limited to 256 commits and 2 MiB
of accounted commit data. Fall back when this budget or an 8192-candidate path
search budget is exceeded. This preserves normal streaming for larger updates.
Reindexing
retained entries still takes O(loaded commits) worker CPU, but must not reparse
old commits or duplicate the entire retained history. Existing UI references
may require copying the small referenced subset of entries during reindexing.

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
