# Content search performance

## Architecture

Content search uses long-lived `git diff-tree --stdin` workers. The Git workers
setting in Preferences, `--git-workers N`, and the `git_workers` preferences
key control both history object readers and content-search workers (1-8,
default 4). Path-only searches use one diff worker.

The pool splits metadata candidates into batches of 64, distributes them
round-robin, and drains results in submission order. The session-wide candidate
watermark (4,096 plus one loading/scanning batch) bounds pending work across
the pool. Cancellation signals all workers before joining them; Git processes
are terminated and reaped. Steps that send or receive work skip the idle
backoff.

Literal content prefilters use `--pickaxe-all` only with a path filter,
preserving commit-level AND semantics when path and content match different
files. Regex, Unicode, binary, merge, and changed-line matching retain their
existing behavior.

## Measurements

Release search-session benchmarks on Linux x86_64 (Ryzen 5 7600X, Git 2.56.0),
using the pinned Nix Rust toolchain, scanned 100,064 nixpkgs commits at HEAD
`1df6d69ca9e5b98a6c21e9ce2d45c1d76563e1e8`. Each workload/configuration had one
warmup and five measured runs. Baseline: `34cf42b`; implementation: `0958d00`.
History object readers were held at four while content-worker counts varied.
These measurements predate the unified Git workers setting.

| Workload | Baseline median | Four-worker median | Reduction | First result, baseline / four |
| --- | ---: | ---: | ---: | ---: |
| Rare literal (`copy_closure`) | 34.580 s | 10.765 s | 68.9% | 25,879 / 8,111 ms |
| Common literal (`version`) | 40.306 s | 14.662 s | 63.6% | 44 / 42 ms |
| Regex (`sha256.*=`) | 37.264 s | 13.056 s | 65.0% | 43 / 43 ms |
| Path + contents (`pkgs/by-name`, `version`) | 40.342 s | 13.908 s | 65.5% | 42 / 43 ms |

Among 1, 2, 4, and 8 content workers, four was the fastest configuration
keeping median first-result latency within 10% of baseline on every workload.
Eight workers improved throughput further but regressed short first-result
latency beyond 10%. Single-worker throughput was broadly unchanged and slower
for the common query. All 120 warmup/measured runs had identical ordered result
hashes per workload.

With four workers, sampled aggregate Git RSS peaked at 4.56 GiB versus 2.11 GiB
baseline; application RSS stayed below 80 MiB. Median CPU utilization was
355-399% versus 107-109% baseline (100% per core). Git RSS sums live processes
and can count shared pages repeatedly. These are search-session measurements,
not desktop measurements; idle CPU and cancellation latency were not timed.
