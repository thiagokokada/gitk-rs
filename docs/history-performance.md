# History loading performance

History uses one `git rev-list --parents` traversal, piped directly into `git
cat-file --buffer --batch`. Git echoes the traversal's parent lists with each
object, so shallow boundaries and replacement ancestry keep Git's normal
semantics. Rust parses raw commit headers and formats dates. Both processes run
on the history worker; OS pipe backpressure bounds pending data. Loading still
streams to EOF by default and pauses for browsing demand with `--limit`.

The raw reader reuses one bounded buffer. It does not write a cache, retain a
second history, count commits first, or restart traversal with `--skip`. Both
Git processes drain bounded stderr and are terminated and reaped on
cancellation or stream replacement. Both exit statuses are checked at EOF.

Non-UTF-8 output configuration uses the original formatted log stream.
Individual commits with other encodings or unusual metadata use a bounded,
single-commit formatted fallback. Oversized raw objects are drained without
retaining them before that fallback, allowing large signatures while preserving
the formatted record size limit. Repositories needing many such fallbacks may
benefit less. Date formatting uses chrono, already present in the dependency
graph.

## Measurement

On 2026-11-10, the release-mode history probe was run on macOS arm64 with the
pinned Nix toolchain. The baseline was commit
`5378f37323219d9846d43264813750348914052e`. The read-only fixture was
[nixpkgs](https://github.com/NixOS/nixpkgs/) at
`5e6fbb384b694837c762676ff4e299d545f3948c`, with 1,114,966 loaded commits, EOF
reached, and the default 256 graph lanes.

Three runs per executable alternated in the order baseline/candidate,
candidate/baseline, baseline/candidate, using the same environment:

| Probe elapsed time | Run 1 | Run 2 | Run 3 | Median |
| --- | --- | --- | --- | --- |
| Original formatted log | 13.419 s | 11.968 s | 11.968 s | 11.968 s |
| Buffered raw objects | 7.350 s | 7.910 s | 7.376 s | 7.376 s |

The median decreased by 38.4%. These are history-only measurements including
retained-data accounting, not desktop startup or cold-disk guarantees. The
accounted retained bytes were identical at 629,734,925; this excludes allocator
overhead, runtime overhead, and child-process memory. RSS was not measured. The
new pipeline has two Git children rather than one.

To reproduce a full-load probe, choose a target larger than the reachable
history:

```sh
nix develop --command env \
  GITK_PROFILE_REPOSITORY="$HOME/Projects/nixpkgs" \
  GITK_PROFILE_COMMITS=2000000 \
  cargo test --release --lib profile_retained_history --locked -- --ignored --nocapture
```

The probe also reports the time to prepare its first batch. To compare every
commit's order, ID, parents, signatures, dates, and full message with the
original Git format, without retaining both histories:

```sh
nix develop --command env \
  GITK_PROFILE_REPOSITORY="$HOME/Projects/nixpkgs" \
  cargo test --release --lib profile_history_stream_parity --locked -- --ignored --nocapture
```

The parity check passed for all 1,114,966 commits at the fixture HEAD above.

## Literal contents search

Literal `contents:` queries with a usable ASCII fragment pass a conservative
`-G` expression to `git diff-tree` before transferring patches to Rust. Git's
`--always` retains a header for every candidate, and `--pickaxe-all` retains
all changed files on a hit so combined path/contents predicates can match
different files. Rust still checks the complete original pattern. Regex and
unsupported literal patterns use the original patch stream. Rename detection
stays disabled.

The prefilter uses up to 64 characters from an ASCII alphanumeric/underscore
fragment of at least three characters, with explicit upper/lowercase
alternatives. It excludes `i` and `k` from fragments because non-ASCII
characters can lowercase to those ASCII letters in Rust. The prefilter may
accept extra commits; it must never exclude a match accepted by the original
matcher.

A local macOS sample on 2026-11-10 used the latest 10,000 commits reachable from
nixpkgs HEAD `f7252b8a2bdc62907fe6fb4a968a6e86279c15dd`, with the
`copy_closure` query. The same commit ID list was sent to both commands through
`--stdin`. Both used `--always --root -r --raw -z --no-renames --no-ext-diff
--no-textconv --diff-merges=first-parent --patch --unified=0 --no-color
--format=%x00%H%x0a`. The candidate additionally used `--pickaxe-all` and
`-G '[cC][oO][pP][yY]_[cC][lL][oO][sS][uU][rR][eE]'`.

| Git command | Elapsed | Emitted bytes |
| --- | --- | --- |
| Original patch stream | 6.251 s | 59,046,028 |
| Literal prefilter | 5.104 s | 430,000 |

Git still needs to inspect candidate changes, and a repository-wide search can
remain slow when matching edits are far back in history. Metadata qualifiers
such as `author:` or `after:` reduce candidates before diff processing; `path:`
is currently evaluated within the diff stream and does not provide that same
early pruning.
