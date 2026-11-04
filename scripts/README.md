These optional tools exercise a real X11 window. Routine regression tests run
headlessly with `cargo test`; running these tools is a separate desktop check.

- `python scripts/gui_smoke.py [BINARY] [OUTPUT_DIRECTORY]` creates disposable
  repositories, exercises navigation and repository workflows, and saves screenshots.
- `python scripts/release_probe.py BINARY go|rust REPOSITORY [OUTPUT_DIRECTORY]`
  records two eight-second trials with separate application and child-process memory
  samples. The second trial reuses the first trial's cache and preferences. Startup
  markers differ between Go and Rust, so these are observations, not equivalent
  render-time benchmarks. Only run it against a repository you intend to inspect.

Both need an X11 display (XWayland works), Git, `xdotool`, `xwininfo`, ImageMagick's
`import`, and the application's native runtime libraries. The probe also needs
`libXRes.so.1`; `GITK_RS_XRES_LIBRARY` can supply its full path. Each tool checks
window ownership before taking screenshots or controlling a window. The smoke
test also checks focus before sending input. Avoid changing focus during a run.
Use `--help` for arguments; output defaults to directories under `/tmp`.

Formatting, linting, and headless script tests are included in `nix flake check`.
To run only these checks during development:

```sh
nix develop --command ruff format --check scripts
nix develop --command ruff check scripts
nix develop --command python -B -m unittest discover -s scripts -v
```
