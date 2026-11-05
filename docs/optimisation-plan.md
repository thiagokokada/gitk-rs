# Optimisation plan

Status: implementation in progress on branch `optimisation`. Keep each validated
change in a separate commit; retain measurements for each increment.

## Baseline: real nixpkgs history

On 2026-11-04, both applications were driven through **100,000 and 200,000 loaded
commits**, twice each, on the same nixpkgs checkout. Counts were verified in logs
and screenshots. The `nix` filter returned **30,026 matches from 200,000 commits**
in both applications; clearing it restored the history.

| Loaded commits | Rust application RSS | Go application RSS | Git child RSS, either app |
| --- | --- | --- | --- |
| 100,000 | 335-336 MiB | 561-564 MiB | 326-327 MiB |
| 200,000 | 562-563 MiB | 1,171-1,216 MiB | About 438 MiB |

Application RSS excludes Git. Neither application's process showed swap at these
checkpoints. Git's RSS includes substantial clean file-backed mappings; it must
not be described as application heap. At 200,000, Rust's Git child had about
406 MiB private clean pages and 30 MiB anonymous memory.

The broad-filter observation consumed 0.15-0.16 CPU seconds in Rust and
0.64-0.66 in Go. These include UI/diff work over a fixed observation interval,
not input-to-paint latency. Screenshots showed the completed filter in both apps.
Rust's five-second idle samples used 0.4-0.6% of one core; Go's rounded to zero
at the sampling resolution.

The balanced run accumulated about 5.47 versus 3.25 application CPU seconds by
100,000 commits, and 10.81 versus 9.07 by 200,000, for Rust versus Go. This suggests
investigating Rust's loading CPU cost despite its lower retained memory. It does
not isolate parsing, graph building, rendering, or highlighting.

### Scope and limitations

- Release packages: Rust revision `72a1803`, Go revision `fc472ac`.
- nixpkgs HEAD: `d72d63e4ca69cbc3141157034a1302eb84894a12`.
- Linux/NixOS, Ryzen 5 7600X, 32 GiB RAM, X11 via XWayland.
- Same 1920x1332 window, dark mode, syntax enabled, 200 graph lanes,
  10,000-commit batches, and automatic refresh disabled.
- Rust caps batches at 10,000 internally. A large `--limit` alone does not prove
  that 100,000 commits were loaded: browsing demand must advance the stream.
- Fresh preferences/cache per launch; OS caches were not flushed. Both application
  orders were exercised. There is no persistent history index to warm.
- The balanced run waited for complete 10,000-commit boundaries before each End
  key. The initial driver also reacted to partial Rust snapshots. Its extra
  selections make its navigation timing unsuitable for direct comparison.
- Wall times contain driver delays and the pause for measurements at 100,000;
  do not present them as parser throughput or a startup-speed comparison.
- This is a small observational sample, not an isolated statistical benchmark.
  Native Wayland, macOS, million-commit browsing, precise frame latency, and
  pathological large diffs remain unmeasured.

Exact observations and provenance are in
[optimisation-baseline.json](optimisation-baseline.json). Raw local logs,
screenshots, and 100 ms process samples were saved under
`/tmp/gitk-rs-large-history-results`; those temporary files may be cleaned up.
The temporary diagnostic drivers were not added to the repository.

## Findings that determine priority

1. **Retained graph geometry is the largest identified Rust allocation.**
   Accounting over the same 200,000 commits found about 293 MiB of edge-vector
   capacity for 229 MiB of live edge payload: roughly **64 MiB of spare capacity**.
   Parent-ID vectors reserve about 18.3 MiB for 6.5 MiB of live elements.
   This accounts for payload capacities, not allocator overhead or a heap profile.
   The 200-lane cap is frequently reached: 115,822 rows carry the overflow flag.
   Preserve that behavior when changing representations.
2. **Graph construction does repeated work.**
   [GraphBuilder::push](../src/graph.rs) clones the lane frontier, including owned
   target-ID strings, for every commit. It then repeatedly performs linear target
   lookups while walking that frontier. A wide graph can make this quadratic in
   lane count. This is a code-derived candidate; no CPU profile attributes a
   percentage of the observed loading time to it yet.
3. **Large-hunk highlighting repeats prefix parsing.**
   [DiffDocument::highlight](../src/diff.rs) starts at the hunk boundary on each
   page request. Scrolling deep into one large hunk repeatedly parses its prefix.
   This stress run exercised ordinary commit diffs, not that worst case.
4. **Filtering and idle polling are secondary.**
   [Commit::searchable_text](../src/git.rs) allocates and lowercases combined text
   on every scan, unlike Go's cached text. Nevertheless, the measured Rust filter
   was already inexpensive at 200,000 commits. UI mailbox timers poll every 16 ms,
   Git cancellation workers every 5 ms, and workspace reconciliation every 100 ms.
   These are candidates for later work rather than reasons to redesign history.

## Increment 1: reduce retained graph allocation

- [x] Add focused, opt-in Rust allocation accounting for real history and a small
  deterministic wide-graph fixture; keep large runs out of normal CI.
- [x] Reduce unused edge-vector capacity without changing the logical graph.
  Compare exact reservation and compact final storage. Measure the CPU cost of
  shrinking or copying each row before choosing a representation.
- [x] Address spare parent-vector capacity if measurements justify a separate
  small change. Preserve typed IDs and SHA-1/SHA-256 behavior.
- [x] Repeat the matched 100,000/200,000-commit workload and report application,
  Git child, and anonymous/private memory separately.

Acceptance: node, edge, color, merge, lane-hole, overflow, and lane-cap behavior
are unchanged; selection and deep scrolling still work. Show measured memory
savings and any loading-CPU tradeoff. Do not claim the theoretical 64 MiB as an
achieved RSS reduction before measuring it.

## Increment 2: reduce graph construction work

- [x] Profile the unchanged loading path on wide real history before changing it.
  Separate Git reading/parsing, graph preparation, and UI application where useful.
- [x] Avoid cloning every lane's owned target string for every row.
- [x] Replace repeated frontier scans if profiling confirms their cost. Keep any
  lookup storage bounded by the configured lane count and preserve stable lanes.
- [x] Test chains, branch/merge patterns, octopus merges, sparse lane holes,
  overflow at several caps, and color continuity. Compare complete logical rows
  against the previous implementation using deterministic fixtures.
- [x] Repeat the baseline and record CPU, elapsed workload time, and memory.

Acceptance: equivalent graph output and navigation behavior, lower measured
loading cost, no whole-history lookup index, and no new retained duplication.

## Increment 3: bound repeated large-hunk highlighting work

- [x] Add an opt-in large single-hunk fixture and measure successive page requests,
  deep jumps, reverse scrolling, and rapid cancellation.
- [x] Retain parser state or sparse checkpoints within a byte budget per active
  diff. Preserve separate old/new parser states for removed and added code.
- [x] Invalidate on document/selection and theme changes; reject stale results.
- [x] Add unit and worker tests for multiline comments/strings, Unicode, hunk/file
  boundaries, theme switches, cancellation, and checkpoint eviction.

Acceptance: paged colors match parsing from the hunk start, deep pages do less
repeated work, and cache bytes remain bounded. Retain full language coverage.

## Increment 4: optional work after profiling

- [x] Reviewed filtering: defer scratch-buffer changes because the measured broad
  filter already costs about 0.15 CPU seconds for 200,000 commits, off the UI thread.
- [x] Replace 5 ms cancellation polling with shared condition-variable notifications;
  stop workspace timer wakes entirely while watching is disabled.
- [x] Test shared wakeups, rapid completion, cancellation, shutdown, watcher failure,
  and manual reload. Compare idle behavior with watching on and off.
- [x] Reviewed UI mailbox notification migration: defer it. Remaining measured
  idle CPU is about 0.04 seconds over 12 seconds; a broader async-channel change
  offers little demonstrated practical benefit in this workload.

Acceptance: deterministic timed wakeups are removed, child cancellation/reaping
and refresh behavior stay intact, and no extra dependencies or retained text are
introduced. Filtering and UI mailbox migration were explicitly assessed and
left unchanged under the plan's optional-work criterion.

## Validation and comparison procedure

Use the pinned Nix toolchain and current release builds. Start both applications
against the same fixed nixpkgs HEAD with:

```sh
BINARY --limit 10000 --graph-cols 200 --mode dark --nowatch --verbose REPOSITORY
```

Advance to each loaded-history boundary, verifying actual counts rather than
assuming the CLI limit is a total. At 100,000 and 200,000, wait for background
work to settle, sample application and child processes separately, then exercise
navigation and a broad filter. Capture own-window screenshots. Do not let a rare
query silently load the rest of history and invalidate the count comparison.
Repeat with identical settings and record cache conditions and any driver delays.
Use focused fixtures for highlighting; history count does not stress a large hunk.
Do not recreate the removed Python release probe as part of runtime optimisation.

For each implementation increment, run relevant unit, real-Git worker, and
headless GPUI tests, plus the standard checks:

```sh
nix develop --command cargo fmt --all -- --check
nix develop --command cargo clippy --all-targets -- -D warnings
nix develop --command cargo test --all-targets --locked
nix build
nix flake check
```

Smoke-test native X11 and Wayland for UI changes, and use macOS CI for portability.
Report unavailable checks explicitly. Keep one Git stream and demand-driven
loading; do not introduce SQLite, a persistent history cache, repeated deep
`--skip` scans, or a lower graph cap to manufacture a benchmark improvement.

## Implementation evidence

### Increment 1: compact immutable storage

- Edges and parent IDs now use boxed slices; complete rows match a frozen
  pre-optimisation oracle over 512 deterministic DAG commits at caps
  1, 2, 3, 8, 32 and 200. Existing SHA-1/SHA-256 parsing remains unchanged.
- At 200,000 commits, edge storage drops from 306,785,160 to 239,630,754 bytes,
  parent-vector storage from 19,202,304 to 6,835,320 bytes, and Entry size from
  264 to 248 bytes (payload accounting, not a heap profile).
- An experimental upper-bound reservation kept 241,005,192 edge bytes and took
  9.44 s in graph preparation; compact final slices kept 239,630,754 bytes and
  took 8.91 s versus 8.78 s before the change. Single observations are noisy;
  compact immutable storage was selected without claiming a CPU improvement.
- Matched release GUI run: 200,000 commits use about 483 MiB application RSS,
  versus 562 MiB in the balanced baseline. Git remains about 433 MiB, versus
  438 MiB before; its clean mapped pages vary. Application CPU was 11.00 s
  versus 10.81 s at that checkpoint. The driver includes deliberate delays.
- Deep navigation and filtering completed; filtering still returns 30,026
  matches. No application swap. The Nix release build and flake checks pass;
  formatting, strict Clippy, all 68 Rust tests, and the opt-in diagnostic pass.
- macOS execution is unavailable in this Linux session; portability remains
  covered by the existing CI job, not claimed as a local check.

Run the opt-in Rust accounting separately from normal CI:

```sh
GITK_BENCH_REPOSITORY=/path/to/nixpkgs nix develop --command \
  cargo test --release --test optimisation -- --ignored --nocapture
```

### Increment 2: stable-lane routing

- Profiling the unchanged graph path attributed 8.78 s of 8.94 s in the
  read/graph intervals to graph preparation (elapsed instrumentation, not a
  sampled CPU profile). Compact storage alone remained at 8.91 s.
- The frontier now copies bounded color metadata on the stack, moves the one
  displaced target, and uses unchanged lane indices for continuation edges.
  There is no whole-history lookup map and no per-row frontier-ID cloning.
- Complete logical rows match the original oracle, including 256-parent octopus
  merges, sparse roots and an evicted target reintroduced at the cap. All 70
  Rust tests, formatting, strict Clippy, release package and flake checks pass.
- Focused graph time is 0.32 s for 200,000 commits. Stream reading now takes
  1.36 s because the faster consumer exposes Git producer latency; the combined
  intervals fall from about 9.1 s after increment 1 to 1.7 s.
- Matched GUI run: application CPU at 200,000 falls from baseline 10.81 s to
  1.92 s; RSS is about 486 MiB, with no swap. The same 30,026 filter matches
  and deep navigation remain functional. Git child memory is reported separately
  in `optimisation-progress.json`; its mapped pages are not application heap.
- Native Wayland and X11 startup/history-paint checks pass. macOS execution
  remains a CI check, not a locally verified desktop test.

### Increment 3: resumable hunk parsing and bounded tokens

- A single old/new parser cursor resumes forward pages; a suffix token ring
  serves overlapping and nearby reverse pages. Token payload plus reserved ring
  storage is capped at 4 MiB / 4096 lines. Retained cursor input is limited to
  1 MiB; opaque Syntect parser allocations are not claimed as measured token bytes.
  Older/evicted pages and larger cursor inputs replay the hunk when necessary.
- Theme changes clear cursor/tokens; each diff document owns its cache. Obsolete
  work preserves only a valid parsed prefix. Selection/request generations retain
  their existing publication guards.
- Five normal regressions cover multiline old/new state, Unicode boundaries,
  theme/file transitions, eviction and zero cache, cancellation/resume, and large
  input cursor release. All 75 normal Rust tests, strict Clippy, formatting, Nix
  build and flake checks pass; native X11 and Wayland paint checks also pass.
- The opt-in 25,000-line Rust fixture completes 12 deep forward/reverse requests
  by parsing 22,048 lines total, retaining 884,736 token/ring bytes. Its debug
  test run took 8.4 s. The same standalone optimized caller with local debug
  dependencies took 94.533 s before versus 8.041 s after. These are controlled
  service diagnostics, not production input-to-paint or full-release timings.
- Matched 200,000-commit GUI run remains about 485 MiB RSS / 1.94 application
  CPU seconds, no swap, and 30,026 broad-filter matches. Git is about 438 MiB.

### Increment 4: event-driven cancellation and watcher-off sleep

- Cancellation uses a shared flag and condition variable. Completion and cancel
  notification synchronize with the predicate-to-wait transition, so the monitor
  cannot miss a notification that precedes its wait. Every child remains reaped.
- Existing blocked-helper cancellation plus new shared-child and rapid-normal-
  completion regressions pass, as do watcher coalescing/failure/manual-refresh
  coverage. All 77 normal Rust tests, formatting, strict Clippy, Nix release
  build and flake checks pass locally.
- Over matched 12-second idle samples, stable-thread voluntary context switches
  fall from 5,961 to 3,462 with watching off and 6,164 to 3,791 with watching on.
  The roughly 200 Hz process monitor disappears. CPU changes from 0.05 to 0.04 s
  off and stays 0.04 s on: too small to claim a meaningful CPU reduction.
- Broader UI mailbox migration and filtering changes are deferred based on these
  measurements; their queues/data models remain unchanged. Final cleanup and
  macOS CI follow-up are still to be completed before opening the PR.
