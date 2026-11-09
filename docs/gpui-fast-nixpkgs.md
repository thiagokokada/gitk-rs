# GPUI Fast on nixpkgs, 2026-11-09

Fast improved headless scrolling preparation more near the top of nixpkgs
history than in the earlier linear fixture. Deeper history and diff gains were
similar to the fixture. Native full-history loading, startup and memory were
essentially unchanged. History virtualization remains enabled.

## Repository and method

- Read the local nixpkgs checkout at HEAD
  `5e6fbb384b694837c762676ff4e299d545f3948c`.
- Loaded all 1,114,966 reachable commits to EOF on both backends, with graph
  lane cap 256. No prefetch limit or watcher was enabled.
- Used GPUI Kit 0.9.0, upstream GPUI 0.3.11 and Fast 0.1.13 from Cargo.lock,
  release builds with pinned Nix Rust 1.99.0, Apple M2 Pro, macOS 27.0.1.
- Ran three fresh processes per backend and mode after validation and compilation
  finished, in upstream/Fast, Fast/upstream, upstream/Fast order. Native binaries
  report `0.1.0-57eb997`; the probe's source hash accompanies the raw data.
- Compared history at rows 0 and 10,000, and a 387-line highlighted Nix diff for
  `eb82e507d5e2ed0a71bb6f2f7c37d3ba547399e1`, row 394 in this history.
  The diff changes two files in `python-modules/flashinfer-jit-cache`.
- The headless window was 1280x850 logical pixels, scale 2, accessibility off.
  Each scenario discarded 32 warmup steps and measured 240 wheel samples,
  moving 40 pixels per step and reversing every 16 steps. Assertions verified
  actual scrolling and the deeper viewport position.

The [probe instructions](gpui-fast.md#read-only-repository-probe) describe
reproduction. Scroll samples measure input dispatch, runnable test tasks and
one explicit draw using GPUI's test text system, without a GPU renderer.
History loading, page waits and highlighting waits are outside the samples.
This measures UI preparation, not native GPU scrolling or display frame time.

The application retained its bounded 512-row display snapshots; it did not
materialize all commits into UI elements. Temporary test preferences and view
state were isolated from user settings. The measured repository's HEAD and
working-tree status were checked before and after and remained unchanged.

## Scrolling results

Values are medians of three per-run medians. The p95 columns are medians of
three per-run p95 values, not pooled percentiles.

| Scenario | Upstream median | Fast median | Reduction | Upstream p95 | Fast p95 |
| --- | ---: | ---: | ---: | ---: | ---: |
| History near row 0 | 1.838 ms | 0.814 ms | 55.7% | 2.301 ms | 1.130 ms |
| History near row 10,000 | 1.848 ms | 1.153 ms | 37.6% | 2.101 ms | 1.417 ms |
| Highlighted Nix diff | 1.746 ms | 0.972 ms | 44.3% | 2.004 ms | 1.118 ms |

The earlier 5,000-commit linear fixture showed 31.8% less history preparation
time and 45.4% less diff preparation time. The top-of-history improvement on
nixpkgs was larger; the deeper viewport and diff improvements were comparable.
The selected content, graph and visible rows differ, so this does not isolate
repository size as the cause. Virtualization keeps drawing work tied to the
viewport even when all million commits are retained.

The initial headless setup reached EOF in a median 12.635 seconds upstream and
12.254 seconds on Fast. This includes test setup and UI updates during streaming;
it is not Git-only parsing throughput.

## Native macOS startup, full loading and memory

Each native process opened nixpkgs with `--timings --verbose --nowatch --no-limit
--mode=dark --graph-cols=256 --nosyntax=false`, selected the default HEAD diff,
and ran until its history log reported EOF at 1,114,966 loaded commits. It then
ran for five more seconds before deliberate SIGTERM termination. Both backends
used the same saved font/layout settings; no native scrolling input was injected.

Memory sampling used `ps -axo pid=,ppid=,rss=` with a 50 ms sleep between calls.
Process-inspection overhead lengthens the actual sampling interval. Application
and summed live-child RSS were tracked separately throughout loading and after
EOF. The raw data includes the complete sampled timeline.

| Native measurement, median of three runs | Upstream | Fast |
| --- | ---: | ---: |
| First window paint callback from `main()` | 540.958 ms | 537.018 ms |
| First history/selection paint callback | 632.516 ms | 627.322 ms |
| Full history EOF observed | 12.343 s | 12.412 s |
| Maximum sampled application RSS after EOF | 818.844 MiB | 821.297 MiB |
| Maximum sampled application RSS during entire run | 818.844 MiB | 821.297 MiB |
| Maximum sampled live-child RSS, summed separately | 976.406 MiB | 981.422 MiB |

Native startup and full-history loading did not show a material improvement.
Application RSS was about 2.45 MiB higher with Fast at the same loaded count.
The first upstream launch used less RSS than its later runs; per-run values
are preserved rather than treating the small median difference as a guarantee.

These are sampled maxima, not peak memory or macOS physical footprint. The
application and child maxima can occur at different times and must not be added
as though they were concurrent. Children can exit between samples. Startup paint
callbacks do not measure GPU completion or screen presentation. Git, graph
construction and retained history use the same application code on both backends.

## Evidence and validation

[Raw readings and metadata](measurements/gpui-fast-nixpkgs-2026-11-09.json) include
all six headless and six native processes, repository identity, loaded counts,
viewport positions, timing samples summarized per run, and native RSS timelines.

Formatting, Clippy on both backends, the ordinary Rust suite on both backends
(168 passed, six opt-in tests ignored), `nix build`, and macOS `nix flake check`
passed. All six real-repository release probes passed. Linux checks and native
Wayland/X11 smoke, native GPU scrolling and accessibility-on performance were
not run on this macOS host.
