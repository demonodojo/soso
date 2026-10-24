#!/usr/bin/env python3
"""Prueba aislada, offline, por consola serie. Conserva imagen y log en target/sal/test."""
import os
import pathlib
import re
import selectors
import shutil
import subprocess
import time

REPO = pathlib.Path(__file__).resolve().parents[2]
OUT = REPO / "target/sal/test"
ROOT = OUT / "rootfs"

def preparar():
    OUT.mkdir(parents=True, exist_ok=True)
    if ROOT.exists():
        shutil.rmtree(ROOT)
    for d in ["bin", "etc", "tmp", "var", "tests", "lib/sal/bin"]:
        (ROOT / d).mkdir(parents=True, exist_ok=True)
    (ROOT / "etc/soso-release").write_text("sal-test\n")
    for name in ["init", "sosh", "echo", "cat", "rm", "mkdir", "cp", "wild-soso", "halt"]:
        source = REPO / "rootfs/bin" / name
        if source.exists(): shutil.copy2(source, ROOT / "bin" / name)
    shutil.copy2(REPO / "user/target/x86_64-soso-user/release/examples/sonda", ROOT / "bin/sal-sonda")
    shutil.copy2(REPO / "target/sal/native/sal", ROOT / "bin/sal")
    shutil.copy2(REPO / "target/sal/native/sal-llc", ROOT / "lib/sal/bin/sal-llc")
    for name in ["runtime.o", "kernels.o", "libsal-soso.a", "link.ld", "manifest.txt"]:
        shutil.copy2(REPO / "target/sal/sdk" / name, ROOT / "lib/sal" / name)
    for name in ["hello", "runtime", "sal-contar"]:
        shutil.copy2(REPO / "target/sal" / name, ROOT / "tests" / name)
    for p in (REPO / "tests/sal").glob("*.sal"):
        shutil.copy2(p, ROOT / "tests" / p.name)
    shutil.copy2(REPO / "user/sal/sal-contar.sal", ROOT / "tests/sal-contar.sal")
    fixtures = {"vacio": b"", "utf8": "ñ€🙂\nfin".encode(), "nul": b"a\0b", "invalido": b"\xc0\xaf", "truncado": b"\xf0\x9f", "surrogate": b"\xed\xa0\x80"}
    for name, content in fixtures.items(): (ROOT / "tests" / name).write_bytes(content)
    subprocess.run([str(REPO / "target/release/mkfs-soso"), str(ROOT), str(OUT / "data.img"), "512"], check=True)

def probar():
    preparar()
    shutil.copy2("/usr/share/OVMF/OVMF_VARS_4M.fd", OUT / "vars.fd")
    cmd = ["qemu-system-x86_64", "-machine", "q35", "-cpu", "max", "-m", "2048", "-smp", "2",
           "-accel", "kvm" if os.access("/dev/kvm", os.R_OK | os.W_OK) else "tcg",
           "-drive", "if=pflash,format=raw,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd",
           "-drive", f"if=pflash,format=raw,file={OUT}/vars.fd",
           "-drive", f"format=raw,snapshot=on,file={REPO}/target/soso-uefi.img",
           "-drive", f"if=none,id=root,format=raw,file={OUT}/data.img",
           "-device", "virtio-blk-pci,drive=root", "-nic", "none", "-display", "none",
           "-serial", "stdio", "-monitor", "none", "-no-reboot"]
    log = open(OUT / "serial.log", "wb")
    proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    sel = selectors.DefaultSelector()
    sel.register(proc.stdout, selectors.EVENT_READ)
    total = bytearray()
    def esperar(pattern, timeout=120):
        start = len(total)
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if re.search(pattern, total[start:]): return bytes(total[start:]).decode("utf8", "replace")
            for key, _ in sel.select(1):
                data = os.read(key.fd, 65536)
                if not data: raise RuntimeError(f"QEMU terminó: {proc.poll()}")
                log.write(data); log.flush(); total.extend(data)
        raise RuntimeError(f"timeout esperando {pattern!r}; ver {OUT}/serial.log")
    def ejecutar(line, expected=(), timeout=120):
        print(line, flush=True)
        proc.stdin.write((line + "\necho SAL_FIN_CASO\n").encode()); proc.stdin.flush()
        text = esperar(rb"\r?\nSAL_FIN_CASO\r?\n", timeout)
        if "SAL_FALLO" in text: raise AssertionError(text)
        for s in expected:
            if s not in text: raise AssertionError(f"falta {s!r}: {text}")
        print(text, flush=True)
    try:
        esperar(rb"\$ ", 120)
        ejecutar("/bin/sal-sonda")
        ejecutar("/bin/echo visible")
        ejecutar("/tests/runtime dos")
        ejecutar("/bin/sal-sonda 42 /tests/hello", ["SAL_OK código 42"])
        ejecutar('/bin/sal-sonda 0 /tests/runtime "dos palabras"', ["SAL_OK código 0", "hola ñ"])
        for path, code, wanted in [("vacio",0,"bytes=0 lineas=0"),("utf8",0,"bytes=13 lineas=1"),
                                  ("nul",1,"UTF-8 sin NUL"),("invalido",1,"UTF-8 sin NUL"),
                                  ("truncado",1,"UTF-8 sin NUL"),("surrogate",1,"UTF-8 sin NUL"),
                                  ("ausente",1,"no se puede leer")]:
            ejecutar(f"/bin/sal-sonda {code} /tests/sal-contar /tests/{path}", [f"SAL_OK código {code}", wanted])
        ejecutar("/lib/sal/bin/sal-llc --version", ["sal-llc 1"])
        ejecutar("cd /tmp")
        ejecutar("/bin/sal-sonda 0 /bin/sal build /tests/hello.sal -o /tmp/hello", ["SAL_OK código 0"], 300)
        ejecutar("/bin/sal-sonda 42 /tmp/hello", ["SAL_OK código 42"])
        ejecutar("/bin/sal-sonda 0 /bin/sal build /tests/imports.sal -o /tmp/imports", ["SAL_OK código 0"], 300)
        ejecutar("/bin/sal-sonda 0 /tmp/imports", ["respuesta=42", "SAL_OK código 0"])
        ejecutar("/bin/sal-sonda 0 /bin/sal build /tests/runtime.sal -o /tmp/runtime", ["SAL_OK código 0"], 300)
        ejecutar('/bin/sal-sonda 0 /tmp/runtime "dos palabras"', ["hola ñ", "SAL_OK código 0"])
        ejecutar("/bin/sal-sonda 1 /bin/sal build /tests/error.sal -o /tmp/error", ["SAL_OK código 1"])
        ejecutar("/bin/sal-sonda 0 /bin/sal build /tests/sal-contar.sal -o /tmp/contar", ["SAL_OK código 0"], 300)
        ejecutar("/bin/sal-sonda 0 /tmp/contar /tests/utf8", ["bytes=13 lineas=1", "SAL_OK código 0"])
        ejecutar('echo "fn main() -> Int" > /tmp/edit.sal')
        ejecutar('echo "    7" >> /tmp/edit.sal')
        ejecutar("/bin/sal-sonda 0 /bin/sal build /tmp/edit.sal -o /tmp/edit", ["SAL_OK código 0"], 300)
        ejecutar("/bin/sal-sonda 7 /tmp/edit", ["SAL_OK código 7"])
        ejecutar('echo "fn main() -> Int" > /tmp/edit.sal')
        ejecutar('echo "    9" >> /tmp/edit.sal')
        ejecutar("/bin/sal-sonda 0 /bin/sal build /tmp/edit.sal -o /tmp/edit", ["SAL_OK código 0"], 300)
        ejecutar("/bin/sal-sonda 9 /tmp/edit", ["SAL_OK código 9"])
        ejecutar("/bin/sal-sonda 9 /bin/sal run /tmp/edit.sal", ["SAL_OK código 9"], 300)
        print("SAL: todas las pruebas offline superadas", flush=True)
    finally:
        proc.terminate()
        try: proc.wait(timeout=10)
        except subprocess.TimeoutExpired: proc.kill(); proc.wait()
        sel.close(); log.close()

if __name__ == "__main__": probar()
