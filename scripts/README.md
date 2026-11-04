The optional X11 smoke test exercises a real window. Routine regression tests run
headlessly with `cargo test`; running this tool is a separate desktop check.

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
