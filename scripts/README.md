The optional X11 smoke test exercises a real window. Routine regression tests run
headlessly with `cargo test`; running this tool is a separate GUI check.

Run the complete smoke script inside a NixOS VM with:

```sh
nix build .#gui-smoke --print-build-logs
```

The guest uses IceWM with an X11 session and Mesa's software Vulkan driver
(Lavapipe). No host display, desktop environment, or GPU is needed. The test runs
the packaged application as an unprivileged user and copies screenshots,
`application.log`, and `driver.log` to `result/artifacts/gui-smoke/`. On failure,
logs are also printed and the driver attempts to copy artifacts before exiting;
use `--keep-failed` to retain the failed build directory.

This opt-in package is available on `x86_64-linux` and `aarch64-linux`; it is not
part of `nix flake check`. NixOS VM tests require a Linux builder with KVM
(`/dev/kvm`, the `kvm` and `nixos-test` Nix system features). Other hosts, including
macOS, can use a configured remote Linux builder; select its architecture
explicitly, for example:

```sh
nix build .#packages.x86_64-linux.gui-smoke --max-jobs 0 --print-build-logs
```

GitHub Actions runs the VM test in the Ubuntu Nix job on pushes and pull
requests to `main`, and uploads successful runs' screenshots and logs as the
`gui-smoke-x11` artifact. Failed runs print the smoke logs in the job output.

To run against your existing X11 session instead:

```sh
python scripts/gui_smoke.py [BINARY] [OUTPUT_DIRECTORY]
```

It creates disposable repositories, exercises navigation and repository workflows,
and saves screenshots. Use `--help` for arguments; output defaults to a directory
under `/tmp`.

The test needs an X11 display (XWayland works), Git, `xdotool`, `xwininfo`,
ImageMagick's `import`, and the application's native runtime libraries. It checks
window ownership before taking screenshots or controlling a window, and focus
before sending input. Avoid changing focus during a run.

Formatting, linting, and headless script tests are included in `nix flake check`.
To run only these checks during development:

```sh
nix develop --command ruff format --check scripts
nix develop --command ruff check scripts
nix develop --command python -B -m unittest discover -s scripts -v
```

To refresh the README screenshot from a clean nixpkgs checkout on X11/XWayland,
run from the project root:

```sh
nix run .#update-screenshot -- ~/Projects/nixpkgs
```

The script loads all commits before applying a regex query with metadata filters,
selects a matching commit, and captures a line diff. Dates are relative to HEAD.
It uses isolated preferences and leaves the repository unchanged. Avoid changing
window focus during capture. Logs, configuration, and capture metadata remain in
the printed `/tmp` directory; `screenshot.png` is replaced only after success.
Use `nix run .#update-screenshot -- --help` to customize the query, selected row,
output path, size, or timeout. The wrapper supplies the application and capture
tools; running `python scripts/update_screenshot.py --help` also documents how to
use an existing binary.
