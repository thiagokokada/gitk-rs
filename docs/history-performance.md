# History loading performance

History uses one `git rev-list --parents` traversal. By default, two persistent
`git cat-file --batch` processes read its objects, with parsing on reader threads
and graph construction on the history worker. The dispatcher preserves the
traversal's parent lists and publishes batches in their original order, preserving
shallow boundaries, replacements, and Git's default traversal order.

`--history-readers N` and the saved `history_readers` preference select 1 through
64 readers (default 2). One reader uses the original buffered direct OS pipe.
Larger pools increase child process memory; they do not limit retained history.
Readers start lazily, allowing the first 64 commits to reach the UI while the
remaining pool starts. Loading streams to EOF by default and pauses for browsing
demand with `--limit`.

Dispatch batches contain at most 64 traversal records and target 2 MiB; a single
bounded record can exceed that target by its terminating newline. Parsed output
chunks also target 2 MiB, plus at most one bounded commit. Input and output
channels have fixed capacities, and the ordered mailbox is capped at the reader
count. A worker reuses its bounded raw-object buffer. Full messages stay in memory
once, alongside shared identities and incremental graph rows; pending work is
bounded independently of retained history.

Each pool has its own shutdown token linked to the history generation's
cancellation. Dropping a provisional reload pool leaves the retained stream
usable. Shutdown closes output receivers before joining workers, terminates and
reaps Git children, and drains bounded stderr. EOF is published only after the
traversal and all object-reader exit statuses have been checked. No Python
adapter is used by the application.

Graph construction indexes active commit targets by lane and writes packed
continuations directly, avoiding repeated linear target searches and temporary
continuation-edge vectors. Differential tests compare the resulting graph rows
with the original algorithm, including merges and frontier overflow.

The end-of-history notice displays this generation's active history-worker time
as seconds with two decimal places. It includes repository discovery, reading,
filter preparation, and snapshot preparation. It excludes time paused waiting
for browsing demand and freezes at completion; later viewport requests do not
increase it. A new generation starts a new measurement. This measures worker
completion, not desktop paint or application/child CPU time.

Non-UTF-8 output configuration uses the original formatted log stream.
Individual commits with other encodings or unusual metadata use a bounded,
single-commit formatted fallback. Oversized raw objects are drained without
retaining them before that fallback, allowing large signatures while preserving
the formatted record size limit. Repositories needing many such fallbacks may
benefit less. Date formatting uses chrono, already present in the dependency
graph.

## Historical single-reader measurement

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

Set `GITK_PROFILE_READERS` to select the reader count for either opt-in probe
(default 2, range 1 through 64). Compare the same reader count as well as the same
HEAD, loaded count, and lane cap. The probe prints the selected reader count.

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
