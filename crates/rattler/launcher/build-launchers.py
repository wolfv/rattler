"""Build the Windows Python entry-point launchers with Zig.

These are the CLI launchers embedded by `rattler` (see
`crates/rattler/src/install/entry_point.rs`) and copied next to every
`*-script.py` on Windows to proxy `console_scripts` entry points to
`python.exe`.

Zig cross-compiles every Windows target from a single host, so the whole
matrix (including `win-arm64`) is produced without needing native ARM
runners. Output binaries are named `cli-{32,64,arm64}.exe` to match the
`include_bytes!` paths in `entry_point.rs`.
"""

import argparse
import subprocess
from pathlib import Path

# Maps the conda-style platform to the Zig target triple. We use the `gnu`
# (mingw) ABI because it produces small, self-contained binaries that do not
# depend on `vcruntime140.dll` being present in `Scripts`.
TARGETS = {
    "win-32": "x86-windows-gnu",
    "win-64": "x86_64-windows-gnu",
    "win-arm64": "aarch64-windows-gnu",
}

LAUNCHER_DIR = Path(__file__).resolve().parent


def build_launcher(platform: str, out_dir: Path) -> Path:
    zig_target = TARGETS[platform]
    out_dir.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        [
            "zig",
            "build",
            "-Doptimize=ReleaseSmall",
            f"-Dtarget={zig_target}",
            "-Dgui=false",
            "--prefix-exe-dir",
            str(out_dir),
        ],
        cwd=LAUNCHER_DIR,
        check=True,
    )
    # `build.zig` names the artifact after the architecture, e.g. `cli-arm64.exe`.
    suffix = platform.split("-", 1)[1]
    produced = out_dir / f"cli-{suffix}.exe"
    if not produced.is_file():
        raise RuntimeError(f"expected {produced} to be built but it is missing")
    return produced


def main() -> None:
    parser = argparse.ArgumentParser(description="Build the Windows entry-point launchers.")
    parser.add_argument(
        "--platform",
        choices=sorted(TARGETS),
        action="append",
        help="Platform(s) to build. Defaults to all of them.",
    )
    parser.add_argument(
        "--out-dir",
        type=Path,
        default=LAUNCHER_DIR / "binaries",
        help="Directory to write the launchers into.",
    )
    args = parser.parse_args()

    platforms = args.platform or list(TARGETS)
    for platform in platforms:
        produced = build_launcher(platform, args.out_dir)
        print(f"Built {produced}")


if __name__ == "__main__":
    main()
