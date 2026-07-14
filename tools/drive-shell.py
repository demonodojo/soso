import socket, sys, time
cmds = open(sys.argv[1]).read().splitlines()
s = socket.create_connection(("127.0.0.1", 4555), timeout=40)
s.settimeout(40)
buf = b""; transcript = b""
def rx():
    global buf, transcript
    d = s.recv(4096); buf += d; transcript += d
def wait_prompt():
    # Si conectamos tarde (o el kernel aún arranca), el prompt ya salió o
    # se perderá lo que enviemos: provocar uno nuevo con \r hasta verlo.
    global buf
    s.settimeout(3)
    intentos = 0
    while b"soso> " not in buf:
        try: rx()
        except TimeoutError:
            intentos += 1
            if intentos > 13: raise
            s.sendall(b"\r")
    s.settimeout(40)
    buf = buf.split(b"soso> ", 1)[1]
for c in cmds:
    wait_prompt()
    s.sendall(c.encode() + b"\r"); time.sleep(0.2)
end = time.time() + 4
while time.time() < end:
    try: rx()
    except Exception: break
inicio = max(transcript.find(b"kernel-shell"), 0)
sys.stdout.write(transcript[inicio:].decode(errors="replace"))
