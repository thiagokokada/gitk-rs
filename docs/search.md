# Commit search

The filter box combines a text phrase and qualifiers with AND. Text matching
ignores case. Use double quotes for values containing spaces, for example:

```text
author:"Alice Smith" path:ci.yml after:2026-01-01
message:"update dependencies" merge:false
contents:"old function name" path:src/
```

Open Help with F1 or ? for the syntax reference. Recognized qualifiers appear
as badges below the input. Invalid filters show an explanation there; correct
the input to resume searching. The status bar reports matching and loaded
counts and shows `filtering` while results are incomplete. Results arrive
incrementally; zero matches while filtering is still in progress does not mean
the search is complete.

| Qualifier | Matches |
| --- | --- |
| `author:text` | Author name or email substring |
| `committer:text` | Committer name or email substring |
| `message:text` | Full commit message substring |
| `hash:text` | Commit hash substring |
| `contents:text` | Added or removed text line containing the literal text |
| `path:text` | Changed path containing the literal text, including directory components |
| `before:YYYY-MM-DD` | Author's recorded calendar date strictly before this day |
| `after:YYYY-MM-DD` | Author's recorded calendar date strictly after this day |
| `merge:true` / `merge:false` | Commits with multiple parents / other commits |

Unqualified text searches the hash, author name/email, and full message as one
contiguous phrase, preserving the original plain-text filter behavior. By default,
all text, path, and contents metacharacters are literal.

Enable **Regex** next to the filter box, or press **Alt+R**, to interpret text,
path, and contents values as regular expressions. The shortcut also works while
typing in the filter box. For example:

```text
author:Alice|Bob path:\.ya?ml$ message:^fix
author:"Alice Smith|Bob Jones" after:2026-01-01
```

Regex matching ignores case by default; inline flags such as `(?-i)` and `(?m)`
can override case matching or enable multiline anchors. Date and merge filters
keep their existing date/boolean meanings. Lookaround and backreferences are
unsupported. Compilation errors appear below the input and can be corrected
without reloading. There are no separate glob operators: use `.*` for an
arbitrary sequence in regex mode.

Path search covers history reachable from the opened HEAD, as does the ordinary
history list. It matches additions, modifications, deletions, and both sides of
renames. Merges compare with their first parent, as in the diff view. It does
not follow a file through earlier names automatically. Paths containing invalid
UTF-8 are decoded with replacement characters for substring matching. Dates use
the author's recorded timezone and exclude the boundary day.

Contents search examines added and removed text lines, excluding the diff's
headers and unchanged context. Matching is per line, without the leading `+` or
`-`; regex anchors apply to that line. It includes root commits and deletions,
and compares merges with their first parent. Binary patches are not searched.
One `contents:` qualifier is allowed per query. Combining `path:` and
`contents:` requires both to match the commit; they can match different files.
Like path search, it covers history reachable from HEAD and reports results
incrementally.

A function present in the current file only appears in contents results when a
commit adds or removes a matching line. Searching a large history for an older
change can take time. Add metadata qualifiers such as `author:` or `after:`
when you know them to reduce the commits whose diffs need searching. A `path:`
filter narrows results but currently does not skip generating unrelated
candidate diffs.
