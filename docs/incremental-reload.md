# Incremental history reload

Manual reload and automatic HEAD-change reload use the same history worker.
Loaded commits stay in memory when the repository state permits reuse. History
is not cached on disk, and the UI does not run Git or rebuild history itself.
The traversal and raw-object pipeline are described in
[History loading performance](history-performance.md).

## Unchanged HEAD

The worker reopens the repository to check its identity, HEAD, and traversal
metadata. If HEAD is unchanged, it keeps the loaded entries, graph state,
completed search results, and any unread portion of the existing Git stream.
Branch names and ref labels are refreshed independently of the commit data.
Automatic changes to refs without a HEAD change normally refresh only labels;
working-tree changes refresh status and local diffs.

## Linear updates

When HEAD changes, the worker starts a separate Git stream at the new commit.
It follows an exact chain of single-parent commits until the parent is the
previous HEAD. New entries and their graph rows are prepared provisionally; the
old history remains intact until the entire chain has been verified.

Verification yields after 256 commits or reaching a 2 MiB accounted-data
threshold per worker step. Individual Git records have a separate size bound.
These thresholds control each step, not the total update size. A linear update
can span any number of steps without triggering a size-based full reload. New
requests are processed between steps, and superseded verification is dropped.

After verification, the new entries are prepended and retained entry ordinals
are shifted. The first old graph row gains an incoming edge. Because the prefix
is a single-parent chain, the remaining graph geometry and the old stream's
graph-builder state can be reused. If the old history was only partially
loaded, that stream continues from its existing position without skipping or
duplicating commits.

## Searches and viewport restoration

Metadata and path filters scan the new prefix in batches of at most 1000
commits. Path searches use bounded input and result queues. Completed suffix
decisions are retained; only its bounded in-flight path candidates need to be
retried. Prefix matches are joined with suffix matches once, avoiding repeated
insertion at the front of a large match list.

The UI keeps its existing snapshot while verification and prefix filtering run.
Selection and the top visible commit are bookmarked by commit ID, then restored
against the reconciled history. Generation and request IDs reject obsolete
snapshots even when a Git stream survives a UI reload.

`history::view::HistoryViewState` owns the displayed page, selection, pending
selection, local rows, viewport, and reload bookmarks. Its transitions accept
worker snapshots and return diff-selection, scroll, and history-request effects
without invoking Git or GPUI. Bookmark indices are updated before the window
applies deferred scrolling, so consecutive reloads use the reconciled viewport.
The window retains focus handling, graph painting, and execution of those
effects.

A reload that supersedes unfinished prefix filtering restarts the search over
retained commits. Changing the query also replaces unfinished search work.

## Full reloads and resource costs

The worker starts a fresh session for merges in the new prefix, divergence,
rewinds, transitions to or from an unborn HEAD, repository or graph
configuration changes, and failed history or query operations. A failed query
is retried on reload even when HEAD is unchanged.

Reuse is conservatively disabled for shallow history, replacement refs, grafts,
and traversal override environment variables. These can change ancestry without
changing the HEAD object ID. General merge reconciliation is not implemented.

Git streams are terminated and reaped when replaced or when the worker stops.
Repository and graph configuration changes cancel incompatible work; compatible
reload requests wait for the current worker step before deciding on stream
reuse.

New commit records are retained once as prospective history and moved into the
final index after verification. Reindexing the retained suffix still takes
O(loaded commits) worker CPU in one pass and can delay subsequent requests.
Entries referenced by the UI may be copied during reindexing; the entire old
history is not duplicated or reparsed. Total retained history remains unbounded
by viewport virtualization, and default loading continues to EOF. With `--limit
N`, the existing stream pauses when browsing demand is satisfied.
