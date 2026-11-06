# Contributor notes

## History memory probe

Run the opt-in, headless probe with the pinned toolchain:

```sh
nix develop --command env GITK_PROFILE_REPOSITORY=/path/to/repository GITK_PROFILE_COMMITS=100000 cargo test --lib profile_retained_history --locked -- --ignored --nocapture
```

The probe reads history without modifying the repository. Compare the same HEAD,
loaded count, and graph lane cap. It reports retained record/index storage,
metadata, messages, and graph payload. Accounted bytes exclude allocator metadata,
references, the live Git stream, and test/runtime overhead; they are not RSS.
On Linux it reports the history-only test process RSS and each live child RSS
separately. This is not a desktop or peak-memory benchmark. Git can read ahead
while the probe runs, so its live RSS is not a fixed-workload comparison.

### Initial observation

On 2026-11-06, before compact storage, using nixpkgs HEAD
`0c2a5544281cab7e6c47057d184b72366ae18e49`, 100,000 loaded commits,
and 200 graph lanes:

| Retained category | Bytes |
| --- | ---: |
| Records, including Arc headers | 26,400,000 |
| Metadata payload | 23,966,557 |
| Message payload | 7,324,872 |
| Graph edge payload | 119,747,844 |
| History/filter indexes | 1,048,576 |
| Total accounted | 178,487,849 |

The observed history-only process RSS was 200,612 KiB; the Git child RSS was
337,532 KiB. One run completed loading in 2,067 ms. These are observations, not
performance guarantees. In particular, they do not establish desktop memory
usage or compare this implementation with Go.
