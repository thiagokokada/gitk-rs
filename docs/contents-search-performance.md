# Content search workers

`contents:` search uses long-lived `git diff-tree --stdin` processes. Set
`GITK_SEARCH_WORKERS` to an integer from 1 through 8 to change the pool size;
unset or invalid values use 4. Path-only searches use one process.

Batches of up to 64 metadata candidates are assigned round-robin and results
are drained in submission order. The existing session-wide candidate watermark
(4,096 plus one scanning/loading batch) bounds pending work across the pool.
Each worker retains bounded input/output channels and owns cancellation and
child cleanup. Idle steps sleep briefly; steps that send or receive work do not.

## Reproducing measurements

Build the baseline at `34cf42b` in a separate worktree. Apply
[`contents-search-baseline-probe.patch`](contents-search-baseline-probe.patch)
to add the same probe instrumentation without changing baseline search behavior.
For each revision, build with the pinned toolchain:

```sh
nix develop --command cargo test --release --lib --locked --no-run
```

Pass the resulting test executable (printed by Cargo) to the harness. The
repository is only read. Run each command sequentially, without concurrent
builds or other benchmarks:

```sh
python3 scripts/benchmark-contents-search.py /tmp/baseline-test-binary \
  /path/to/nixpkgs baseline.jsonl --workers baseline
python3 scripts/benchmark-contents-search.py /tmp/current-test-binary \
  /path/to/nixpkgs current.jsonl --workers 1,2,4,8
```

Each workload/configuration gets one unmeasured warmup and five measured runs.
The probe stops history loading after reaching 100,000 commits (rounded up to
the next loading batch), then finishes filtering all loaded candidates. This
bounds benchmark scope, not application history retention. It records ordered
result hashes, end-to-end session duration, and first-result latency. Regex mode
is enabled for the regex workload. RSS is sampled every 20 ms using Linux
`/proc`: the application and the aggregate of live descendants are reported
separately. These are sampled peaks, not allocator accounting or desktop RSS.
CPU utilization includes reaped descendants and uses 100% per occupied core.
Compare hashes and loaded counts before comparing medians; exclude warmup rows.

## Measurements
Measured on 2026-11-11/12, Linux x86_64, AMD Ryzen 5 7600X (6 cores,
12 threads), Git 2.56.0, using release builds from the pinned Nix Rust toolchain.
Baseline: `34cf42b` plus the probe-only patch; implementation: `0958d00`.
The nixpkgs HEAD was `1df6d69ca9e5b98a6c21e9ce2d45c1d76563e1e8`;
100,064 commits were loaded, with four history readers and 256 graph lanes.
Each row summarizes five measured runs after a warmup, run sequentially.

Durations and CPU are medians; RSS columns are the largest sampled peak across
the five measured runs. Git RSS sums live children and can count shared pages
more than once. It is separate from application RSS. CPU uses 100% per core.

| Workload | Workers | Duration (s) | First (ms) | App RSS (MiB) | Git RSS (GiB) | CPU (%) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| rare | baseline | 34.580 | 25879 | 73.9 | 2.10 | 106.6 |
| rare | 1 | 34.484 | 25834 | 73.4 | 2.13 | 106.6 |
| rare | 2 | 19.143 | 14424 | 73.7 | 2.93 | 206.1 |
| rare | 4 | 10.765 | 8111 | 74.3 | 4.52 | 399.0 |
| rare | 8 | 8.343 | 6423 | 74.8 | 7.45 | 651.6 |
| common | baseline | 40.306 | 44 | 78.0 | 2.11 | 108.1 |
| common | 1 | 42.116 | 42 | 76.8 | 2.13 | 107.1 |
| common | 2 | 25.293 | 42 | 77.3 | 3.01 | 192.1 |
| common | 4 | 14.662 | 42 | 79.3 | 4.56 | 354.9 |
| common | 8 | 10.707 | 58 | 79.0 | 7.43 | 606.8 |
| regex | baseline | 37.264 | 43 | 78.0 | 2.11 | 109.1 |
| regex | 1 | 37.346 | 41 | 77.9 | 2.14 | 108.9 |
| regex | 2 | 22.394 | 38 | 77.8 | 3.00 | 196.0 |
| regex | 4 | 13.056 | 43 | 77.9 | 4.54 | 360.9 |
| regex | 8 | 9.692 | 57 | 81.1 | 7.46 | 613.0 |
| combined | baseline | 40.342 | 42 | 74.9 | 2.11 | 108.1 |
| combined | 1 | 40.337 | 42 | 75.1 | 2.12 | 108.0 |
| combined | 2 | 24.186 | 40 | 75.0 | 3.00 | 193.5 |
| combined | 4 | 13.908 | 43 | 77.6 | 4.54 | 360.8 |
| combined | 8 | 10.189 | 59 | 77.7 | 7.45 | 617.2 |

Workloads: rare literal `contents:copy_closure`; common literal
`contents:version`; regex `contents:sha256.*=` with regex mode enabled;
combined `path:pkgs/by-name contents:version`.

The default is **4 workers**, the fastest measured configuration whose
median first-result latency stays within 10% of baseline for every workload.
It reduces median duration by 63.6-68.9%, with at most a 2.4% median
first-result regression. Aggregate sampled Git RSS rises from about 2.11 GiB
to 4.56 GiB; application RSS with four workers stays below 80 MiB in these runs.

All 120 warmup/measured runs produced identical ordered result hashes per
workload. Single-worker throughput was broadly unchanged (and slower for the
common query); parallelism supplies the measured improvement. Eight workers
have higher throughput but regress short first-result latency beyond 10%
and increase aggregate child RSS. Use `GITK_SEARCH_WORKERS=1` or `2` to reduce
Git process memory when needed.

[Raw measurements](contents-search-measurements.jsonl) include each iteration,
CPU, RSS peaks, result counts, and ordered-result SHA-256 hashes. The baseline
harness omitted the loaded-count JSON field; the probe log and deterministic
loading boundary establish the same 100,064-commit scope.

Cancellation, query replacement, pause/resume, and bounded backpressure are
covered by real-Git regression tests, including 6,000 candidates at every worker
count. Pool cancellation signals every worker before joining any of them. Idle
steps retain backoff and the history engine waits on a condition variable when
finished. No dedicated idle-CPU or cancellation-latency timing was collected.
These are search-session measurements, not desktop/Wayland/X11 measurements;
macOS and native display smoke tests were not run for this data-path change.
