#!/usr/bin/env python3
"""Lanza soso sin ventana, abre soso-wm y ejecuta pasos con capturas de pantalla.

Sirve para *mirar* el escritorio (cursor, menús, ventanas) sin una sesión
gráfica: arranca `cargo xtask run` con monitor y QMP, teclea `soso-wm` por el
monitor y vuelca PNG que se pueden abrir con cualquier visor.

    scripts/wm-probe.py [--out DIR] [--no-wm] [--keep] PASO...

Pasos (se ejecutan en orden):
    key:alt-ret        una tecla del monitor de QEMU (sendkey)
    type:texto         teclea texto (teclado español del guest)
    move:X,Y           puntero absoluto en píxeles de pantalla (QMP)
    click:X,Y          mueve y pulsa el botón izquierdo
    down:X,Y / up      pulsa / suelta el izquierdo (arrastrar: down, move, up)
    wait:SEGUNDOS
    shot:nombre        captura a DIR/nombre.png
    pix:X,Y            imprime el color del píxel de la última captura

Ejemplo (el fallo del menú, comprobado con el puntero sobre «Archivos»):
    scripts/wm-probe.py key:alt-ret wait:3 click:20,12 wait:2 move:40,60 shot:menu
"""
import json, os, socket, subprocess, sys, time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
W, H = 1280, 800
MON, QMP = "target/wm-mon.sock", "target/wm-qmp.sock"
# Teclado español del guest: carácter -> tecla de QEMU
TECLAS = {" ": "spc", "-": "slash", ".": "dot", ",": "comma", "/": "shift-7",
          "\n": "ret", "_": "shift-slash", ":": "shift-dot"}


def esperar(cond, seg, que):
    fin = time.time() + seg
    while time.time() < fin:
        if cond():
            return
        time.sleep(1)
    sys.exit(f"wm-probe: tiempo agotado esperando {que}")


class Monitor:
    def __init__(self):
        self.s = socket.socket(socket.AF_UNIX)
        self.s.connect(str(ROOT / MON))
        self.s.settimeout(2)
        self._leer()

    def _leer(self):
        try:
            return self.s.recv(65536).decode(errors="ignore")
        except OSError:
            return ""

    def cmd(self, c, pausa=0.3):
        self.s.sendall((c + "\n").encode())
        time.sleep(pausa)
        return self._leer()


class Qmp:
    def __init__(self):
        self.s = socket.socket(socket.AF_UNIX)
        self.s.connect(str(ROOT / QMP))
        self.f = self.s.makefile("rw")
        self.f.readline()
        self.cmd({"execute": "qmp_capabilities"})

    def cmd(self, c):
        self.f.write(json.dumps(c) + "\n")
        self.f.flush()
        while True:
            r = json.loads(self.f.readline())
            if "return" in r or "error" in r:
                return r

    def ev(self, events):
        return self.cmd({"execute": "input-send-event", "arguments": {"events": events}})

    def mover(self, x, y):
        self.ev([{"type": "abs", "data": {"axis": "x", "value": int(x * 32767 / (W - 1))}},
                 {"type": "abs", "data": {"axis": "y", "value": int(y * 32767 / (H - 1))}}])

    def boton(self, abajo):
        self.ev([{"type": "btn", "data": {"button": "left", "down": abajo}}])


def main():
    args = sys.argv[1:]
    out = ROOT / "target/wm-probe"
    abrir_wm, mantener = True, False
    while args and args[0].startswith("--"):
        a = args.pop(0)
        if a == "--out":
            out = Path(args.pop(0))
        elif a == "--no-wm":
            abrir_wm = False
        elif a == "--keep":
            mantener = True
        else:
            sys.exit(f"wm-probe: opción desconocida {a}")
    out.mkdir(parents=True, exist_ok=True)
    subprocess.run(["pkill", "-x", "qemu-system-x86"], check=False)
    time.sleep(2)
    for f in (MON, MON + ".serial.log", QMP):
        (ROOT / f).unlink(missing_ok=True)
    env = dict(os.environ, SOSO_QEMU_MONITOR=MON, SOSO_QEMU_QMP=QMP, SOSO_QEMU_USB_TABLET="1")
    log = open(out / "xtask-run.log", "w")
    proc = subprocess.Popen(["cargo", "xtask", "run"], cwd=ROOT, env=env, stdout=log, stderr=log)
    serie = ROOT / (MON + ".serial.log")
    try:
        esperar(lambda: serie.exists() and "sosh" in serie.read_text(errors="ignore"), 900,
                "el prompt de sosh (compila la imagen la primera vez)")
        mon, qmp = Monitor(), Qmp()
        ultima = None

        def teclear(texto):
            for ch in texto:
                mon.cmd("sendkey " + (TECLAS.get(ch) or ch), 0.15)

        if abrir_wm:
            teclear("soso-wm\n")
            esperar(lambda: "listen puerto 7430" in serie.read_text(errors="ignore"), 60, "soso-wm")
            time.sleep(3)
        for paso in args:
            k, _, v = paso.partition(":")
            if k == "key":
                mon.cmd("sendkey " + v, 0.3)
            elif k == "type":
                teclear(v.replace("\\n", "\n"))
            elif k in ("move", "click", "down"):
                x, y = (int(n) for n in v.split(","))
                qmp.mover(x, y)
                time.sleep(0.4)
                if k == "click":
                    qmp.boton(True); time.sleep(0.2); qmp.boton(False)
                elif k == "down":
                    qmp.boton(True)
            elif k == "up":
                qmp.boton(False)
            elif k == "wait":
                time.sleep(float(v))
            elif k == "shot":
                time.sleep(1.5)
                ppm = out / (v + ".ppm")
                mon.cmd(f"screendump {ppm}", 1.0)
                from PIL import Image
                ultima = Image.open(ppm).convert("RGB")
                ultima.save(out / (v + ".png"))
                ppm.unlink()
                print(f"captura: {out / (v + '.png')}")
            elif k == "pix":
                x, y = (int(n) for n in v.split(","))
                print(f"pixel {x},{y} = {ultima.getpixel((x, y)) if ultima else 'sin captura'}")
            else:
                sys.exit(f"wm-probe: paso desconocido {paso!r}")
    finally:
        if not mantener:
            subprocess.run(["pkill", "-x", "qemu-system-x86"], check=False)
            proc.wait(timeout=30)


if __name__ == "__main__":
    main()
