# Commit search

The filter box combines a text phrase and qualifiers with AND. Text matching ignores
case. Use double quotes for values containing spaces, for example:

```text
author:"Alice Smith" path:ci.yml after:2026-01-01
message:"update dependencies" merge:false
```

Open Help with F1 or ? for the syntax reference. Recognized qualifiers appear as
badges below the input. Invalid filters show an explanation there; correct the input to resume searching. The status bar reports
matching and loaded counts and shows `filtering` while results are incomplete.
Results arrive incrementally; zero matches while filtering is still in progress
does not mean the search is complete.

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
contiguous phrase, preserving the original plain-text filter behavior. Unknown
qualifier names, empty values, unclosed quotes, invalid dates, and multiple path filters produce errors. There are no
regular expressions or wildcard operators; path metacharacters are literal.
Dates use the author's recorded timezone and exclude the boundary day.

Path search covers history reachable from the opened HEAD, as does the ordinary
history list. It matches additions, modifications, deletions, and both sides of
renames. Merges compare with their first parent, as in the diff view. It does not
follow a file through earlier names automatically. Paths containing invalid UTF-8
are decoded with replacement characters for substring matching.

## Implementation and probes

The `Qualifier` enum supplies canonical names, value validation, and Help entries;
parsed metadata predicates store enum variants rather than string keys.
Metadata predicates run in bounded chunks on the history worker and reuse retained
commit objects. A path query sends only metadata candidates through one cancellable
`git diff-tree --stdin` process. Raw NUL-delimited records preserve unusual names;
changed paths are matched and discarded immediately. The process inspects each
candidate's diff rather than performing a second history traversal. No changed-file
lists, path index, or persistent cache are built.

Candidate input uses two bounded batches of at most 1,000 IDs; output holds at
most 256 commit results. History loading also pauses when 4,096 candidates are
outstanding (with at most one batch of overshoot). The ordinal map removes entries
as results arrive. Matching rows remain in history order. Cancellation closes both
mailboxes, kills and reaps Git, and joins its input/output/stderr threads. UI
request IDs reject stale snapshots. Git errors surface rather than appearing as
zero matches. An invalid query can be corrected without reloading the repository.
Changing a query retains ordinary history and restarts only its predicate scan
and optional diff process.

With `--limit`, satisfying demand closes the diff input and drains the final
results before pausing. Scrolling farther starts a new diff stream for newly
loaded candidates; it does not repeat earlier tree checks. Default prefetch runs
to EOF using one history stream and one diff stream per query.

The opt-in probe measures the actual history worker, including graph preparation,
with first-result and completion timings. It reads the repository without
modifying it, streams to EOF, and prints HEAD, loaded count, matches and lane cap.
On Linux it samples test-process and live-child RSS separately at the first result
and completion; these are not desktop or peak-memory measurements. Optional
`GITK_SEARCH_REFINE` measures another query against the already retained history:

```sh
nix develop --command env \
  GITK_SEARCH_REPOSITORY="$HOME/Projects/nixpkgs" \
  GITK_SEARCH_QUERY='path:ci.yml after:2026-01-01' \
  cargo test --lib profile_metadata_search --locked -- --ignored --nocapture
```

Path queries can still inspect many trees when metadata is broad or there are
no matching paths. Combine metadata qualifiers to reduce that work. Performance
observations include the repository HEAD and exact workload; the probe is not a
desktop benchmark or a release gate.

## Observed nixpkgs workload

On 2026-11-08, the pinned Nix development toolchain's debug worker probe read
nixpkgs HEAD `2331afaed4dc876d129e87b162f640430eb850ee`, with 256 graph lanes
and all 1,113,366 reachable commits retained. Filesystem caches were not controlled.

| Query | First result | Completion | Matches |
| --- | --- | --- | --- |
| `path:ci.yml after:2026-01-01` | No matches | 50,805 ms | 0 |
| `path:.github/workflows/check.yml after:2026-01-01` | 180 ms (5,064 loaded) | 49,540 ms | 195 |
| `author:alice after:2026-01-01` applied after the second query | Not sampled | 393 ms | 8 |

The second run sampled test-process RSS at 13,988 kB at the first result and
756,988 kB after completion. Its two Git children separately reported 250,792 kB
and 346,760 kB at the first result; both had exited at completion. These samples
exclude desktop rendering and do not measure peak RSS. A separate mid-run diagnostic
saw the diff Git child at 1,050,124 kB RSS without recording a loaded count; that
sample cannot be compared at equivalent loaded counts. Git's pack mappings and
caches remain a resource cost despite bounded application queues.

The local Go implementation at `fc472ac7040d6982daa123c3577e684a198ad9b4`
only supports plain substring filtering, so it cannot run an equivalent metadata
or changed-path query. These are Rust worker observations, not Go comparisons.

Validation passed on x86_64 Linux: Rust formatting, Clippy with warnings denied,
all 136 active tests (including headless GPUI), `nix build`, and `nix flake check`.
Separate native Wayland and X11 startup smoke checks created windows and loaded
history. macOS and aarch64 checks were not run on this host.
