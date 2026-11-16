# History loading performance

History now uses one Git revision traversal and an ordered pool of native
`cat-file` readers. `--git-workers N` and Preferences support 1-8 workers for
both history loading and content search; one reader keeps the direct buffered
pipe. Indexed graph lane lookup and direct continuation packing reduce graph
preparation work.

Reader queues and chunks are bounded, cancellation terminates and reaps Git
children, and every child exit status is checked before EOF. The first viewport
can load while remaining readers start. History still streams to EOF by default;
`--limit` pauses loading on demand.

## Final measurements

Measured on 2026-11-11, Linux x86_64 (Intel Core Ultra X7 358H), with the pinned
Nix Rust 1.99.0 toolchain and Git 2.56.0. All variants loaded **1,080,074
commits** to EOF with 256 graph lanes, using five measured rounds after warm-up.

The fixture pinned nixpkgs commit `452cd10fe859df9b1e49abb4afd978691b5d2b3f`.
These are warm-cache history-worker measurements including retained accounting.

| Implementation               | Median full load | Median first batch |
| ---------------------------- | ---------------: | -----------------: |
| Original single reader       |           9.38 s |              24 ms |
| Updated graph, single reader |           9.32 s |              26 ms |
| Updated graph, two readers   |           6.57 s |              26 ms |

The two-reader median within-round reduction was **27.6%**, ranging from **24.0%
to 31.2%**. The difference between the pooled full-load medians was 29.9%.

Accounted retained data was identical at **610,776,884 bytes**, excluding
allocator, runtime, and child-process overhead.

Native two and eight-reader output matched Git for every commit in this fixture.

## Opt-in history memory probe

- `GITK_PROFILE_REPOSITORY`: required repository path; the probe only reads it.
- `GITK_PROFILE_COMMITS`: prefetch target; must be an integer greater than 64
  that fits usize. Stops earlier if Git reaches EOF.
- `GITK_PROFILE_READERS`: reader count; integer from 1 to 8.
- Run explicitly; this test is ignored by ordinary test runs:

```sh
nix develop --command env \
  GITK_PROFILE_REPOSITORY="$HOME/Projects/nixpkgs" \
  GITK_PROFILE_COMMITS=100000 \
  cargo test --lib profile_retained_history --locked -- --ignored --nocapture
```

Compare the same repository HEAD, loaded count, reader count, and graph lane
cap. The probe prints its graph lane cap; it does not accept the application's
`--graph-cols` option. Accounted retained bytes exclude allocator/runtime
overhead. Linux RSS reports the history-only test process and live children
separately; these are neither desktop nor peak-memory measurements. Git may have
exited at EOF.
