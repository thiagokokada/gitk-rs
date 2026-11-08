# Commit search

The filter box combines a text phrase and qualifiers with AND. Text matching
ignores case. Use double quotes for values containing spaces, for example:

```text
author:"Alice Smith" path:ci.yml after:2026-01-01
message:"update dependencies" merge:false
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
| `path:text` | Changed path containing the literal text, including directory components |
| `before:YYYY-MM-DD` | Author's recorded calendar date strictly before this day |
| `after:YYYY-MM-DD` | Author's recorded calendar date strictly after this day |
| `merge:true` / `merge:false` | Commits with multiple parents / other commits |

Unqualified text searches the hash, author name/email, and full message as one
contiguous phrase, preserving the original plain-text filter behavior. By default,
all text and path metacharacters are literal.

Enable **Regex** next to the filter box, or press **Alt+X**, to interpret text
and path values as regular expressions. The shortcut also works while typing
in the filter box. For example:

```text
author:Alice|Bob path:\.ya?ml$ message:^fix
author:"Alice Smith|Bob Jones" after:2026-01-01
```

Turn **Regex** off (or press **Alt+X** again) to force literal substring searches, including characters such
as `.`, `*`, `[`, and `\`. Double quotes group values with spaces in either mode;
they do not disable regex. Escape a quote inside a value with `\"`. Backslashes
in patterns such as `\S+` are preserved. Quote an unqualified expression containing
a colon so it is not interpreted as a qualifier.

Regex matching ignores case by default; inline flags such as `(?-i)` and `(?m)`
can override case matching or enable multiline anchors. Date and merge filters
keep their existing date/boolean meanings. Patterns use Rust's `regex` syntax;
lookaround and backreferences are unsupported. Compilation errors appear below
the input and can be corrected without reloading. There are no separate glob
operators: use `.*` for an arbitrary sequence in regex mode.

The toggle is saved automatically in `state.toml` under `[search]` as `regex = true`
or `false`, alongside layout and diff state. Existing state files default to
literal mode. Regex mode supports at most 16 text/path patterns per query.

Path search covers history reachable from the opened HEAD, as does the ordinary
history list. It matches additions, modifications, deletions, and both sides of
renames. Merges compare with their first parent, as in the diff view. It does
not follow a file through earlier names automatically. Paths containing invalid
UTF-8 are decoded with replacement characters for substring matching. Dates use
the author's recorded timezone and exclude the boundary day.
