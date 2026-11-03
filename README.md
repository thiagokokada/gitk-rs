# gitk-rs

A Rust/GPUI port of gitk-go for Linux/NixOS, designed for bounded-memory browsing
of very large Git histories. See [PLAN.md](PLAN.md) for milestones and acceptance
criteria and [AGENTS.md](AGENTS.md) for contributor instructions.

**Status:** initial scaffold, not a working Git history viewer. The window shell
and CLI build and pass lint/tests and Nix package checks. Development and installed
executables passed short startup smoke tests on Wayland and X11.
History, diffs, syntax highlighting and the remaining workflows are still pending.

## Development

Nix with flakes enabled supplies Rust and native libraries. The flake pins nixpkgs;
the shell deliberately does not depend on the application package, so it can be
used without first building the application:

```sh
nix develop
cargo fmt --all
cargo test --all-targets --locked
cargo clippy --all-targets -- -D warnings
cargo run -- --mode dark /path/to/repository
```

Cargo.lock and flake.lock pin the dependency graph and Nix environment. Verify
packaging with:

```sh
nix build
nix flake check
./result/bin/gitk-rs --help
./result/bin/gitk-rs /path/to/repository
```

The scaffold accepts --mode dark|light, --timings, --help and --version. A path beginning
with a dash can follow --. Repository paths may contain spaces or non-UTF8 bytes.
The GUI uses the published [GPUI 0.2.2 API](https://docs.rs/gpui/0.2.2/gpui/).
Linux enables Wayland and X11; macOS dependency configuration is preparatory and
has not been validated. A desktop session and a working GPU driver are required
for GUI smoke tests. The package supplies loaders, not a machine-specific driver.

The remaining CLI controls are tracked in the plan and are not accepted yet.
The small [xattr compatibility patch](vendor/README.md) is required by GPUI's
transitive archive dependency on current libc releases.

--timings reports window creation and first CPU paint callback timestamps to stderr
and a bounded recent-sample CPU render-to-paint p95 on graceful exit. Its writer
runs off the UI thread. These measurements exclude process-loader time and do not
measure compositor presentation, GPU execution, or history readiness yet.

## Environment limitations observed

In the restricted execution sandbox, connecting to the Nix daemon socket returns
Operation not permitted despite its world-writable mode. DNS lookup of crates.io
and GitHub also fails. These are observed execution-environment restrictions;
they do not imply a problem with Nix in the host user's terminal.

An approved execution exception now permits running Nix outside that sandbox.
The pinned development shell, dependency downloads, formatting, lint/tests,
packaged build and flake checks now succeed through that exception. This does not
change host Nix permissions. See PLAN.md for validation details and pending work.

The initial pin inherited from gitk-go used an older glibc than this machine's
graphics driver. Updating the pin to the host nixpkgs revision resolved Vulkan
initialization on both backends. If a future NixOS upgrade causes GPU startup
failures, Vulkan loader diagnostics can distinguish ABI incompatibility from
missing devices or drivers; do not work around it by forcing a vendor-specific
driver in the package.
