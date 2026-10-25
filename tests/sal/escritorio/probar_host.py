#!/usr/bin/env python3
"""Pruebas host de user/sal/escritorio (linux, sin syscalls)."""
import os
import pathlib
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parents[3]
SAL = REPO.parent / "sal" / "target" / "release" / "sal"
SDK = REPO / "target" / "sal" / "sdk"
OUT = REPO / "target" / "sal" / "escritorio-host"
SKIP = {"bytes_type.sal", "str_type.sal"}


def main() -> int:
    if not SAL.is_file() or not SDK.is_dir():
        print("falta SDK o compilador Sal; ejecuta cargo xtask sal-build", file=sys.stderr)
        return 2
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
