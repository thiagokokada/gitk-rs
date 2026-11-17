# History exploration

Use the History scope control to browse branches and tags without checkout.
Filters search within the chosen scope. See [search](search.md) for ranges.

## Compare changes

Select a commit and use **Compare changes… > Set as comparison base**, or use
its context menu. Select another commit and choose **Compare with base**. The
pinned base survives filters and scope changes within the repository. Comparison
uses the two commit trees directly, including unrelated histories; it does not
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
