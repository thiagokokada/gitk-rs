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
