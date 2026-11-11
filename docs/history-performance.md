# History loading performance

History now uses one Git revision traversal and an ordered pool of native
`cat-file` readers, defaulting to two. `--history-readers N` and Preferences
support 1-64 readers; one reader keeps the direct buffered pipe. Indexed graph
lane lookup and direct continuation packing reduce graph preparation work.

Reader queues and chunks are bounded, cancellation terminates and reaps Git
children, and every child exit status is checked before EOF. The first viewport
can load while remaining readers start. History still streams to EOF by
default; `--limit` pauses loading on demand.

## Final measurements

Measured on 2026-11-11, Linux x86_64 (Intel Core Ultra X7 358H), with the
pinned Nix Rust 1.99.0 toolchain and Git 2.56.0. All variants loaded
**1,080,074 commits** to EOF with 256 graph lanes, using five measured rounds
after warm-up.

The fixture pinned nixpkgs commit `452cd10fe859df9b1e49abb4afd978691b5d2b3f`.
These are warm-cache history-worker measurements including retained accounting.

| Implementation | Median full load | Median first batch |
| --- | ---: | ---: |
| Original single reader | 9.38 s | 24 ms |
| Updated graph, single reader | 9.32 s | 26 ms |
| Updated graph, default two readers | 6.57 s | 26 ms |

The default's median within-round reduction was **27.6%**, ranging from **24.0%
to 31.2%**. The difference between the pooled full-load medians was 29.9%.

Accounted retained data was identical at **610,776,884 bytes**, excluding
allocator, runtime, and child-process overhead.

Native two and eight-reader output matched Git for every commit in this
fixture.

To run the opt-in probe against a repository, choose a commit target beyond
EOF:

```sh
nix develop --command env \
  GITK_PROFILE_REPOSITORY="$HOME/Projects/nixpkgs" \
  GITK_PROFILE_COMMITS=2000000 \
  GITK_PROFILE_READERS=2 \
  cargo test --release --lib profile_retained_history --locked -- --ignored --nocapture
```
