#!/usr/bin/env python3
"""Pruebas host de user/sal/escritorio (linux, sin syscalls)."""
import os
import pathlib
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parents[3]


def sal_bin() -> pathlib.Path:
    import json

    manifest = REPO.parent / "sal" / "Cargo.toml"
    meta = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--manifest-path", str(manifest)],
        cwd=REPO,
        capture_output=True,
        text=True,
        check=True,
    )
    target = json.loads(meta.stdout)["target_directory"]
    return pathlib.Path(target) / "release" / "sal"
SDK = REPO / "target" / "sal" / "sdk"
OUT = REPO / "target" / "sal" / "escritorio-host"
SKIP = {"bytes_type.sal", "str_type.sal"}


def comprobar_spawn_sin_fds_literales() -> int:
    """Un 0 en stdin/stdout/stderr de spawn_io_* le quita al WM su propio fd."""
    mal = 0
    for f in sorted((REPO / "user/sal/escritorio").glob("*.sal")):
        for n, linea in enumerate(f.read_text().splitlines(), 1):
            if "spawn_io" in linea and "0, 0, 0, fd_kernel_log()" in linea and "fn spawn_io" not in linea:
                print(f"FALLO {f.name}:{n}: spawn con stdio 0,0,0 (usa fd_tty_heredado())", file=sys.stderr)
                mal = 1
    return mal


def main() -> int:
    SAL = sal_bin()
    if not SAL.is_file() or not SDK.is_dir():
        print("falta SDK o compilador Sal; ejecuta cargo xtask sal-build", file=sys.stderr)
        return 2
    if comprobar_spawn_sin_fds_literales():
        return 1
    OUT.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env["SAL_SDK"] = str(SDK)
    for src in sorted((REPO / "tests/sal/escritorio").glob("*.sal")):
        if src.name in SKIP:
            continue
        out = OUT / src.stem
        r = subprocess.run(
            [str(SAL), "build", "--release", str(src), "-o", str(out)],
            cwd=REPO,
            env=env,
            capture_output=True,
            text=True,
        )
        if r.returncode != 0:
            print(r.stderr or r.stdout, file=sys.stderr)
            return 1
        run = subprocess.run([str(out)], capture_output=True)
        if run.returncode != 0:
            print(f"FALLO {src.name}: código {run.returncode}", file=sys.stderr)
            return 1
        print(f"OK {src.name}")
    print("escritorio host: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
