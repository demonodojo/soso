"""Test de fase 5: matar QEMU (SIGKILL) en mitad de una ráfaga de escrituras
y comprobar que sosofs siempre remonta íntegro.

Cada ronda: arranca QEMU, manda writes sin parar, kill -9 tras un retardo
variable, rearranca y verifica con ls/cat que lo visible es consistente
(cada fichero presente está completo; la generación anunciada, legible).
"""
import os, signal, socket, subprocess, sys, time

ROOT = "/home/jmdiez/Trabajo/demonodojo/soso"
QEMU = [
    "qemu-system-x86_64", "-machine", "q35", "-m", "256M",
    "-drive", f"format=raw,file={ROOT}/target/soso-bios.img",
    "-drive", f"file={ROOT}/target/soso-data.img,format=raw,if=none,id=data0",
    "-device", "virtio-blk-pci,drive=data0",
    "-serial", "tcp:127.0.0.1:4555,server=on,wait=off",
    "-display", "none", "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04",
    "-no-reboot",
]

def conectar():
    for _ in range(100):
        try:
            s = socket.create_connection(("127.0.0.1", 4555), timeout=2)
            s.settimeout(3)
            return s
        except OSError:
            time.sleep(0.2)
    raise RuntimeError("no se pudo conectar al serie")

def esperar_prompt(s, buf):
    """Devuelve (recibido_nuevo, resto_tras_prompt)."""
    recibido = b""
    intentos = 0
    while b"soso> " not in buf:
        try:
            d = s.recv(4096)
            if not d:
                raise ConnectionResetError("EOF: QEMU murió")
            recibido += d
            buf += d
        except TimeoutError:
            intentos += 1
            if intentos > 15:
                raise
            s.sendall(b"\r")
    return recibido, buf.split(b"soso> ", 1)[1]

def sesion(comandos, tras=None):
    """Arranca QEMU, ejecuta comandos y devuelve el transcript.
    Si `tras` es (n, delay), tras el comando n espera delay y hace kill -9."""
    p = subprocess.Popen(QEMU, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        s = conectar()
        buf, transcript = b"", b""
        transcript += buf
        for i, c in enumerate(comandos):
            try:
                recibido, buf = esperar_prompt(s, buf)
                transcript += recibido
                s.sendall(c.encode() + b"\r")
            except (TimeoutError, BrokenPipeError, ConnectionResetError):
                break  # QEMU murió bajo nosotros: es lo esperado con kill
            if tras and i == tras[0]:
                time.sleep(tras[1])
                p.send_signal(signal.SIGKILL)
        # apurar la salida restante
        fin = time.time() + 3
        while time.time() < fin:
            try:
                d = s.recv(4096)
                if not d:
                    break
                buf += d
            except OSError:
                break
        transcript += buf
        s.close()
    finally:
        try:
            p.kill()
        except OSError:
            pass
        p.wait()
    return transcript.decode(errors="replace")

def verificar(ronda):
    """Rearranca y comprueba consistencia; devuelve la generación."""
    cmds = ["df", "ls /", "ls /r"] + [f"cat /r/k{i:02}" for i in range(NFILES)] + ["halt"]
    t = sesion(cmds)
    assert "NoValidSuperblock" not in t and "BadChecksum" not in t and "Corrupt" not in t, \
        f"ronda {ronda}: FS dañado tras el kill:\n{t}"
    assert "generación" in t, f"ronda {ronda}: no montó:\n{t}"
    gen = int(t.split("generación ")[1].split()[0].rstrip(","))
    # Cada fichero visible en ls debe leerse entero y con el contenido esperado.
    completos = 0
    for i in range(NFILES):
        nombre = f"k{i:02}"
        esperado = f"contenido {i:02} " + "x" * 40
        if nombre in t.split("ls /r", 1)[1].split("soso>")[0]:
            assert esperado in t, f"ronda {ronda}: {nombre} presente pero incompleto:\n{t}"
            completos += 1
    return gen, completos

NFILES = 30

# Preparación: crear /r una vez (transacción propia).
sesion(["mkdir /r", "halt"])

rondas = []
for ronda in range(8):
    escrituras = [f"write /r/k{i:02} contenido {i:02} " + "x" * 40 for i in range(NFILES)]
    # kill tras el comando `n` con un retardo fino distinto cada ronda:
    # así el SIGKILL cae en puntos diferentes del commit.
    n = 2 + ronda * 3
    delay = (ronda % 5) * 0.017
    sesion(escrituras, tras=(n, delay))
    gen, completos = verificar(ronda)
    rondas.append((n, delay, gen, completos))
    print(f"ronda {ronda}: kill tras write #{n}+{delay:.3f}s -> gen {gen}, {completos} ficheros completos, monta OK")

print("\nTODAS LAS RONDAS OK: el FS montó íntegro tras cada kill -9")
