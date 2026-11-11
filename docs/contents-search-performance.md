# Content search workers

`contents:` search uses long-lived `git diff-tree --stdin` processes. Set
`GITK_SEARCH_WORKERS` to an integer from 1 through 8 to change the pool size;
unset or invalid values use 2. Path-only searches use one process.

Batches of up to 64 metadata candidates are assigned round-robin and results
are drained in submission order. The existing session-wide candidate limit
bounds pending work, rather than multiplying that limit by the worker count.
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
