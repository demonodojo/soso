#!/usr/bin/env python3
"""Pruebas host de user/sal/escritorio (linux, sin syscalls)."""
import os
import pathlib
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parents[3]


def sal_bin():
    import json

    # Máquina de desarrollo: manda el checkout hermano ../sal (como siempre).
    source = pathlib.Path(os.environ.get("SOSO_SAL_SOURCE", REPO.parent / "sal"))
    manifest = source / "Cargo.toml"
    if manifest.is_file():
        meta = subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--manifest-path", str(manifest)],
            cwd=REPO,
            capture_output=True,
            text=True,
            check=True,
        )
        target = json.loads(meta.stdout)["target_directory"]
        return pathlib.Path(target) / "release" / "sal"
    # CI no tiene ese checkout: usa el toolchain empaquetado de la release del
    # pin (`cargo xtask sal-fetch`) o el compilado localmente. Sólo vale si su
    # source.lock es el del pin; un target/sal viejo no se cuela.
    pin = REPO / "config/sal-source.lock"
    if pin.is_file():
        for lock_rel, rel in (
            ("toolchain/source.lock", "toolchain/sal"),
            ("source.lock", "native/sal"),
        ):
            cand = REPO / "target" / "sal" / rel
            lock = REPO / "target" / "sal" / lock_rel
            if cand.is_file() and lock.is_file() and lock.read_bytes() == pin.read_bytes():
                return cand
    return None
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
    if comprobar_spawn_sin_fds_literales():
        return 1
    SAL = sal_bin()
    if SAL is None or not SAL.is_file() or not SDK.is_dir():
        # Sin toolchain no hay nada que compilar: se omite, no se finge un pase.
        print(
            "escritorio host: OMITIDO (falta el toolchain de Sal; "
            "ejecuta cargo xtask sal-fetch o sal-build)",
            file=sys.stderr,
        )
        return 0
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
