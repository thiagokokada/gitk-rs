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
