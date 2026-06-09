# Windows entry-point launchers

This directory contains the sources for the Windows launchers that `rattler`
embeds (see `crates/rattler/src/install/entry_point.rs`) and copies next to
every `*-script.py` in a prefix's `Scripts` directory. Their job is to proxy a
Python `console_scripts` entry point to the adjacent `python.exe`, because
shebangs do not work on Windows.

## Provenance

The launchers are based on the [CPython 3.7 launcher][cpython-launcher],
patched for the conda ecosystem. The sources and build recipe are maintained
upstream at [`conda/conda-launchers`][conda-launchers]; this directory vendors
them so the binaries can be built and **code-signed by prefix.dev** directly
from this repository.

- `launcher.c` — CPython 3.7 `PC/launcher.c` with
  `cpython-launcher-c-mods-for-setuptools.3.7.patch` already applied.
- `cpython-launcher-c-mods-for-setuptools.3.7.patch` — the conda/setuptools
  patch, kept for provenance.
- `launcher.manifest` — application manifest (`asInvoker`, OS compatibility).
- `build.zig` — Zig build script.
- `cpython-LICENSE` / `LICENSE` — license files (Python-2.0 AND BSD-3-Clause).

[cpython-launcher]: https://github.com/python/cpython/blob/3.7/PC/launcher.c
[conda-launchers]: https://github.com/conda/conda-launchers

## Building

Zig cross-compiles every Windows target (including `win-arm64`) from a single
host, so no native ARM runner is required:

```bash
pixi run -e launchers build-launchers          # builds all three
pixi run -e launchers build-launchers --platform win-arm64
```

This produces `cli-32.exe`, `cli-64.exe`, and `cli-arm64.exe` in
`crates/rattler/launcher/binaries/`.

## Releasing (signed binaries)

The committed launchers in `crates/rattler/resources/cli-*.exe` are produced
and code-signed by the **`Update Launcher Binaries`** workflow
(`.github/workflows/launchers.yml`). To regenerate them:

1. Push to the `launcher-build-branch` (or trigger the workflow manually).
2. The workflow builds all three launchers with Zig, signs them with Azure
   Trusted Signing, and commits the signed `.exe` files back to
   `crates/rattler/resources/`.

The launchers expose debug output when `PYLAUNCH_DEBUG=1` is set.
