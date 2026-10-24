#!/usr/bin/env python3
"""Comprueba la revisión base y el contenido exacto del port en el checkout Sal."""
import hashlib
import os
from pathlib import Path
import subprocess
import sys

root = Path(__file__).resolve().parents[1]
source = Path(os.environ.get("SOSO_SAL_SOURCE", root.parent / "sal"))
files = [source / "Cargo.toml", source / "Cargo.lock"]
for directory in ["src", "runtime"]:
    files.extend(p for p in (source / directory).rglob("*") if p.is_file())
digest = hashlib.sha256()
for p in sorted(files):
    digest.update(str(p.relative_to(source)).encode() + b"\0")
    digest.update(p.read_bytes())
    digest.update(b"\0")
revision = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], text=True).strip()
identity = f"sal-source=1\nbase={revision}\nsha256={digest.hexdigest()}\n"
lock = root / "config/sal-source.lock"
if sys.argv[1:] == ["--update"]:
    lock.write_text(identity)
elif sys.argv[1:]:
    sys.exit("uso: sal-source.py [--update]")
elif not lock.is_file() or lock.read_text() != identity:
    sys.exit("El checkout de Sal no coincide con config/sal-source.lock. Revisa los cambios antes de actualizar el pin.")
print(identity, end="")
