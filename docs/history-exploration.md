# History exploration

Use the History scope control to browse branches and tags without checkout.
Filters search within the chosen scope. See [search](search.md) for ranges.

## Compare changes

Choose **Set as comparison base** from a commit's context menu, then **Compare
with base** from another commit's context menu. A compact row shows the pinned
base or both endpoints. Its **Comparison** menu offers Clear comparison base,
plus Swap endpoints and Exit comparison while a comparison is active. The pinned
base survives filters and scope changes within the repository. Comparison uses
the two commit trees directly, including unrelated histories; it does not
substitute their merge base. Old/New display options refer to the base/target
respectively. The existing diff presentation handles renames, binary notices,
mode changes, symlinks and submodule pointers; it does not expand binary
contents or submodule history.

Ordinary row selection leaves comparison endpoints fixed. Swap reverses them;
setting another base or choosing Compare with base replaces that endpoint. Exit
comparison returns to the scope, query, selected commit and viewport where the
comparison began. The base remains pinned until cleared. Changing repository
clears comparisons and their return locations. Menus provide keyboard access.

## File history

Select a file and open **File actions… > Show file history**, or use the file's
context menu. Choose Target or Base / parent explicitly; for a deleted file,
choose its last existing parent version. The header identifies the starting
revision and path. Each result uses its historical filename, and selecting it
shows that file's change. **Back to repository history** restores your previous
scope, filter, selection and viewport.

This uses Git's single-file `--follow` rename heuristic. Copies, splits and
ambiguous merges do not have complete identity tracking. File results can skip
ancestry, so their graph rows are isolated rather than drawing false parent
edges. Path filters remain substring searches without rename following.

## Merge parents

Merge details offer **Changes relative to: Parent …**. The initial parent is
Parent 1; selecting another updates the file list and diff together. IDs and
available loaded subjects/ref labels identify each parent, including octopus
merges. **Go to parent (reveal ancestry)** explicitly browses the parent's
ancestry with filters cleared. **Back to merge** restores the previous scope,
query, viewport, selection and chosen parent. Changing the displayed parent does
not change the first-parent semantics of path/content searches. Root commits
continue to use Git's root diff, without hard-coding an empty-tree object ID.

## Blame and line investigation

**File actions… > Blame this version** opens the complete committed file with
attribution beside each line. Choose the base/parent or target version
explicitly. Attribution identifies the source commit, original filename and
original line, even when the selected version has a different filename. Select
attribution to see its summary. Use Up/Down in the blame pane or the visible
line buttons to select lines; Enter or **Open commit** reveals that commit's
file history. **Back to blame** restores the file version, viewport and selected
line. **Back from blame** restores the preceding diff and repository position.

**Blame previous version** uses the previous revision/path reported by Git and
opens that complete snapshot. It does not guess a matching line in the parent.
For merges, open the commit and choose a parent explicitly. Attribution
describes committed snapshots, including files subsequently deleted, rather than
working tree edits. Binary content is unsupported; missing objects and shallow
history are reported by Git or marked as history boundaries. Invalid UTF-8 file
contents are displayed with replacement characters; paths retain their original
bytes.
