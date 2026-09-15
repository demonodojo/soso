#!/usr/bin/env python3
"""Captura una base reproducible del checkout sin alterarlo (ficha T01).

La base declarada de una sesión de automejora es el árbol de trabajo tal y
como está: un commit, lo que hay en el índice, lo que hay encima del índice y
los archivos nuevos. Un `git rev-parse HEAD` no basta para reconstruirla, y un
`tar` del directorio arrastraría `target/`, modelos y firmware.

Esta captura escribe, fuera de los archivos fuente:

  baseline.json      manifiesto versionado (`schema_version` 1)
  diff/staged.diff   `git diff --cached --binary`
  diff/unstaged.diff `git diff --binary`
  sources/<ruta>     contenido literal de los archivos nuevos seleccionados
  logs/<nombre>.log  argv, cwd, código de salida y salidas de cada comando

y comprueba que el checkout no cambió durante la captura repitiendo el estado
y los hashes (sonda). Si difieren, la captura se declara inestable y no se
escribe `baseline.json`: una base que se movió mientras se leía no sirve.

Nunca modifica el repositorio de origen: todos los comandos son de lectura y
llevan `--no-optional-locks` para no reescribir siquiera el índice. No hace
`stash`, `reset`, `add` ni `commit`.

Subcomandos:

  capture      captura la base           (`--repo`, `--out`)
  reconstruct  la reconstruye y verifica (`--capture`, `--into`)
  suites       ejecuta las suites base sobre un árbol y anota el resultado

Las suites base de C6 no se ejecutan durante `capture`: el manifiesto solo
declara su argv y su disponibilidad, para que un build largo no quede
escondido dentro de una captura que debe ser rápida y repetible.
"""
import argparse
import base64
import copy
import datetime
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys

ESQUEMA = 1

# Árbol vacío de git: sirve de base para el diff staged de un repo sin commits.
ARBOL_VACIO = "4b825dc642cb6eb9a060e54bf8d69288fbee4904"

# Los archivos nuevos por debajo de estos prefijos no se guardan como fuente
# ni se recorren: son discos de modelos, firmware y artefactos de build.
PREFIJOS_EXCLUIDOS = (
    "target/",
    "models/",
    "rootfs/lib/firmware/",
    "lxdde/firmware/",
    "user/target/",
    "kernel/target/",
)

# Prefijos que además no se hashean: pueden ser gigabytes.
PREFIJOS_SIN_HASH = ("target/", "models/", "user/target/", "kernel/target/")

LIMITE_FUENTE = 1 << 20  # 1 MiB: por encima se registra ruta+hash, no contenido
LIMITE_HASH = 256 << 20  # 256 MiB: por encima ni se hashea

# Lista explícita: solo estas variables se versionan. Nunca se vuelca el
# entorno entero, que trae credenciales y rutas ajenas a la base.
VARIABLES_SOSO = (
    "SOSO_BUILD",
    "SOSO_CHECK_PROFILE",
    "SOSO_DRIVERS",
    "SOSO_FIRMWARE",
    "SOSO_LIVE_MODEL",
    "SOSO_LIVE_OFFLINE",
    "SOSO_LXDDE",
    "SOSO_LXDDE_MODE",
    "SOSO_MODELS_DIR",
    "SOSO_QEMU_ACCEL",
    "SOSO_QEMU_MEM",
    "SOSO_QEMU_SMP",
    "SOSO_RUST_VENDOR",
    "SOSO_TEST_JOBS",
    "SOSO_VERSION",
)

# Variables de build no-SOSO que cambian lo que se compila. Misma regla:
# lista explícita y corta.
VARIABLES_BUILD = (
    "CARGO_TARGET_DIR",
    "RUSTUP_TOOLCHAIN",
    "RUSTC_WRAPPER",
    "CARGO_BUILD_JOBS",
)

# Defensa: aunque alguien añada un nombre así a las listas, solo se registra
# si estaba definida, jamás su valor.
SECRETO = re.compile(r"TOKEN|KEY|SECRET|PASS|CRED|AUTH", re.IGNORECASE)

# (nombre, argv). La captura registra disponibilidad y versión, no falla si
# falta una herramienta: eso también es parte de la base.
HERRAMIENTAS = (
    ("git", ["git", "--version"]),
    ("cargo", ["cargo", "--version"]),
    ("rustc", ["rustc", "--version"]),
    ("rustup", ["rustup", "show", "active-toolchain"]),
    ("clang", ["clang", "--version"]),
    ("lld", ["ld.lld", "--version"]),
    ("qemu", ["qemu-system-x86_64", "--version"]),
    ("opencode", ["opencode", "--version"]),
    ("python3", ["python3", "--version"]),
)

# Suites base de C6, con su cwd relativo al árbol. `soso-llm` se compila con
# cwd `user/`, que es donde vive su configuración de target y linker.
SUITES_BASE = (
    ("core-std", ["cargo", "test", "-p", "soso-llm-core", "--features", "std"], "."),
    ("guest-llm", ["cargo", "build", "--release", "-p", "soso-llm"], "user"),
    ("xtask-check", ["cargo", "xtask", "check"], "."),
    ("xtask-test", ["cargo", "xtask", "test"], "."),
)

LOCKFILES = ("Cargo.lock", "kernel/Cargo.lock", "user/Cargo.lock", "rootfs/src/soso/Cargo.lock")


class ErrorCaptura(Exception):
    """Error de uso o de entorno: la captura no llega a empezar."""


class CapturaInestable(Exception):
    """El checkout cambió mientras se capturaba."""

    def __init__(self, diferencias):
        super().__init__("el checkout cambió durante la captura")
        self.diferencias = diferencias


# --- utilidades -------------------------------------------------------------


def sha256_bytes(datos):
    return hashlib.sha256(datos).hexdigest()


def sha256_archivo(ruta):
    h = hashlib.sha256()
    with open(ruta, "rb") as f:
        for trozo in iter(lambda: f.read(1 << 20), b""):
            h.update(trozo)
    return h.hexdigest()


def ruta_json(crudo):
    """Representación JSON de una ruta en bytes.

    Devuelve (texto, dict_extra). Si no es UTF-8 válido, el texto se sustituye
    y se adjunta la forma base64, que sí reconstruye los bytes originales.
    """
    try:
        return crudo.decode("utf-8"), {}
    except UnicodeDecodeError:
        return crudo.decode("utf-8", "replace"), {"ruta_b64": base64.b64encode(crudo).decode()}


def ruta_fs(crudo):
    """Ruta usable por el sistema de archivos, conservando bytes raros."""
    return os.fsdecode(crudo)


def ruta_segura(crudo):
    """Rechaza rutas absolutas o con `..`: nada escribe fuera de la captura."""
    texto = ruta_fs(crudo)
    if os.path.isabs(texto) or texto.split("/")[0] == ".." or "/../" in texto:
        raise ErrorCaptura(f"ruta no admitida en la captura: {texto!r}")
    return texto


def ahora():
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def entorno_git(cwd):
    """Entorno que impide a git subir por encima del directorio pedido.

    Sin esto, un comando lanzado en un directorio que no es un repositorio
    (por ejemplo un destino recién creado) encuentra el repositorio que lo
    contiene y actúa sobre él. Un `checkout` así mueve el HEAD del checkout
    del usuario, que es justo lo que esta herramienta promete no tocar.
    """
    env = os.environ.copy()
    padre = os.path.dirname(os.path.abspath(cwd))
    techos = [padre]
    if env.get("GIT_CEILING_DIRECTORIES"):
        techos.append(env["GIT_CEILING_DIRECTORIES"])
    env["GIT_CEILING_DIRECTORIES"] = ":".join(techos)
    env["GIT_DISCOVERY_ACROSS_FILESYSTEM"] = "0"
    return env


def argv_git_arbol(arbol, argumentos, base=None):
    """Comando git atado a un árbol concreto: `--git-dir` corta el descubrimiento."""
    arbol = os.path.abspath(arbol)
    return (base or GIT_BASE)[:1] + [
        "-C", arbol,
        "--git-dir", os.path.join(arbol, ".git"),
        "--work-tree", arbol,
    ] + (base or GIT_BASE)[1:] + argumentos


class Registro:
    """Escribe un log por comando: argv, cwd, código de salida y salidas."""

    def __init__(self, directorio):
        self.directorio = directorio
        os.makedirs(directorio, exist_ok=True)
        self.usados = {}

    def nombre_libre(self, nombre):
        n = self.usados.get(nombre, 0)
        self.usados[nombre] = n + 1
        return nombre if n == 0 else f"{nombre}-{n + 1}"

    def ejecutar(self, nombre, argv, cwd, timeout=None, env=None):
        """Ejecuta argv (sin shell) y deja su log. Nunca lanza por exit != 0."""
        nombre = self.nombre_libre(nombre)
        destino = os.path.join(self.directorio, f"{nombre}.log")
        try:
            p = subprocess.run(
                argv, cwd=cwd, capture_output=True, timeout=timeout, check=False, env=env
            )
            codigo, salida, error, motivo = p.returncode, p.stdout, p.stderr, None
        except FileNotFoundError:
            codigo, salida, error, motivo = None, b"", b"", "no disponible"
        except subprocess.TimeoutExpired as e:
            codigo = None
            salida = e.stdout or b""
            error = e.stderr or b""
            motivo = f"timeout tras {timeout}s"
        cabecera = (
            f"# argv: {json.dumps(argv)}\n"
            f"# cwd: {cwd}\n"
            f"# exit: {codigo if codigo is not None else motivo}\n"
        )
        with open(destino, "wb") as f:
            f.write(cabecera.encode())
            f.write(b"--- stdout ---\n")
            f.write(salida)
            f.write(b"\n--- stderr ---\n")
            f.write(error)
        return {
            "nombre": nombre,
            "argv": argv,
            "cwd": cwd,
            "exit_code": codigo,
            "motivo": motivo,
            "log": os.path.relpath(destino, os.path.dirname(self.directorio)),
            "stdout": salida,
            "stderr": error,
        }


# --- lectura de git ---------------------------------------------------------

# `--no-optional-locks` evita que una lectura reescriba el índice del origen.
# `core.abbrev=40` y `diff.renames=false` hacen los diffs byte a byte estables
# entre el repo original y su reconstrucción.
GIT_BASE = ["git", "--no-optional-locks", "-c", "core.quotepath=false", "-c", "core.abbrev=40"]
GIT_DIFF = GIT_BASE + ["-c", "diff.renames=false", "-c", "diff.noprefix=false"]


def git(repo, argumentos, registro=None, nombre=None, base=None):
    argv = (base or GIT_BASE) + argumentos
    entorno = entorno_git(repo)
    if registro is not None:
        r = registro.ejecutar(nombre or "git", argv, repo, env=entorno)
        if r["exit_code"] != 0:
            raise ErrorCaptura(
                f"falló {' '.join(argv)} (exit {r['exit_code']}): "
                f"{r['stderr'].decode('utf-8', 'replace').strip()}"
            )
        return r["stdout"]
    p = subprocess.run(argv, cwd=repo, capture_output=True, check=False, env=entorno)
    if p.returncode != 0:
        raise ErrorCaptura(
            f"falló {' '.join(argv)} (exit {p.returncode}): "
            f"{p.stderr.decode('utf-8', 'replace').strip()}"
        )
    return p.stdout


def estado_crudo(repo):
    """`git status` porcelain v1 con rutas delimitadas por NUL."""
    return git(repo, ["status", "--porcelain=v1", "-z", "--untracked-files=all"])


def parsear_estado(crudo):
    """Parsea el formato -z, donde un renombrado ocupa dos campos."""
    campos = crudo.split(b"\0")
    entradas, i = [], 0
    while i < len(campos):
        campo = campos[i]
        i += 1
        if not campo:
            continue
        if len(campo) < 4 or campo[2:3] != b" ":
            raise ErrorCaptura(f"entrada de estado ilegible: {campo!r}")
        xy = campo[:2].decode("ascii")
        camino = campo[3:]
        origen = None
        if "R" in xy or "C" in xy:
            if i >= len(campos):
                raise ErrorCaptura(f"renombrado sin ruta de origen: {campo!r}")
            origen = campos[i]
            i += 1
        texto, extra = ruta_json(camino)
        entrada = {"xy": xy, "ruta": texto}
        entrada.update(extra)
        if origen is not None:
            texto_o, extra_o = ruta_json(origen)
            entrada["origen"] = texto_o
            if extra_o:
                entrada["origen_b64"] = extra_o["ruta_b64"]
        entradas.append(entrada)
        entrada["_bytes"] = camino
    return entradas


def head_de(repo):
    p = subprocess.run(
        GIT_BASE + ["rev-parse", "HEAD"], cwd=repo, capture_output=True, check=False,
        env=entorno_git(repo),
    )
    if p.returncode != 0:
        return None
    commit = p.stdout.decode().strip()
    detalle = git(repo, ["log", "-1", "--format=%H%x00%T%x00%an%x00%aI%x00%s", commit])
    partes = detalle.split(b"\0")
    rama = git(repo, ["rev-parse", "--abbrev-ref", "HEAD"]).decode().strip()
    return {
        "commit": commit,
        "arbol": partes[1].decode(),
        "autor": partes[2].decode("utf-8", "replace"),
        "fecha": partes[3].decode(),
        "asunto": partes[4].decode("utf-8", "replace").strip(),
        "rama": rama,
    }


def diffs_de(repo, head, registro):
    """Diffs binarios de índice y árbol de trabajo, con su lista de archivos."""
    base = head["commit"] if head else ARBOL_VACIO
    staged = git(
        repo,
        ["diff", "--cached", "--binary", "--no-color", "--no-ext-diff", "--no-textconv", base],
        registro,
        "git-diff-staged",
        GIT_DIFF,
    )
    unstaged = git(
        repo,
        ["diff", "--binary", "--no-color", "--no-ext-diff", "--no-textconv"],
        registro,
        "git-diff-unstaged",
        GIT_DIFF,
    )
    nombres_staged = git(
        repo, ["diff", "--cached", "--name-status", "-z", base], registro,
        "git-name-status-staged", GIT_DIFF,
    )
    nombres_unstaged = git(
        repo, ["diff", "--name-status", "-z"], registro,
        "git-name-status-unstaged", GIT_DIFF,
    )
    return {
        "staged": (staged, nombres_staged),
        "unstaged": (unstaged, nombres_unstaged),
    }


def parsear_name_status(crudo):
    """Lista (estado, ruta) de un `--name-status -z`.

    `R`/`C` traen tres campos: estado, origen y destino. Aquí los diffs se
    piden con `diff.renames=false`, pero el formato se respeta igual para que
    cambiar esa opción no desalinee el parseo.
    """
    campos = [c for c in crudo.split(b"\0") if c]
    salida, i = [], 0
    while i + 1 < len(campos):
        estado = campos[i].decode("ascii", "replace")
        i += 1
        origen = None
        if estado[:1] in ("R", "C"):
            if i + 1 >= len(campos):
                break
            origen = campos[i]
            i += 1
        texto, extra = ruta_json(campos[i])
        i += 1
        entrada = {"estado": estado, "ruta": texto}
        entrada.update(extra)
        if origen is not None:
            entrada["origen"] = ruta_json(origen)[0]
        salida.append(entrada)
    return salida


# --- archivos nuevos --------------------------------------------------------


def motivo_exclusion(relativa, tamano):
    if relativa.startswith(PREFIJOS_EXCLUIDOS):
        return "prefijo excluido (artefactos, modelos o firmware)"
    if tamano > LIMITE_FUENTE:
        return f"tamaño {tamano} > {LIMITE_FUENTE}"
    return None


def clasificar_nuevos(repo, entradas):
    """Separa los archivos nuevos en fuentes guardadas y exclusiones.

    De los excluidos se registra ruta, tamaño y hash (salvo que ni siquiera se
    deban recorrer), para que la exclusión quede declarada y no silenciosa.
    """
    incluidos, excluidos = [], []
    for e in entradas:
        if not e["xy"].startswith("??"):
            continue
        relativa = ruta_segura(e["_bytes"])
        absoluta = os.path.join(repo, relativa)
        registro = {"ruta": e["ruta"]}
        if "ruta_b64" in e:
            registro["ruta_b64"] = e["ruta_b64"]
        if os.path.islink(absoluta):
            registro.update(
                {"tipo": "symlink", "destino": os.readlink(absoluta), "bytes": 0}
            )
            incluidos.append(registro)
            continue
        if not os.path.isfile(absoluta):
            registro.update({"tipo": "otro", "motivo": "no es archivo regular"})
            excluidos.append(registro)
            continue
        tamano = os.path.getsize(absoluta)
        motivo = motivo_exclusion(relativa, tamano)
        if motivo:
            sin_hash = relativa.startswith(PREFIJOS_SIN_HASH) or tamano > LIMITE_HASH
            registro.update(
                {
                    "tipo": "archivo",
                    "bytes": tamano,
                    "sha256": None if sin_hash else sha256_archivo(absoluta),
                    "motivo": motivo,
                }
            )
            excluidos.append(registro)
            continue
        registro.update(
            {
                "tipo": "archivo",
                "bytes": tamano,
                "sha256": sha256_archivo(absoluta),
                "modo": oct(os.stat(absoluta).st_mode & 0o777),
                "fuente": "sources/" + relativa,
            }
        )
        incluidos.append(registro)
    incluidos.sort(key=lambda r: r["ruta"])
    excluidos.sort(key=lambda r: r["ruta"])
    return incluidos, excluidos


def guardar_fuentes(repo, salida, entradas):
    """Copia el contenido de los archivos nuevos seleccionados."""
    raiz = os.path.join(salida, "sources")
    for e in entradas:
        if not e["xy"].startswith("??"):
            continue
        relativa = ruta_segura(e["_bytes"])
        absoluta = os.path.join(repo, relativa)
        if os.path.islink(absoluta):
            destino = os.path.join(raiz, relativa)
            os.makedirs(os.path.dirname(destino), exist_ok=True)
            if os.path.lexists(destino):
                os.remove(destino)
            os.symlink(os.readlink(absoluta), destino)
            continue
        if not os.path.isfile(absoluta):
            continue
        tamano = os.path.getsize(absoluta)
        if motivo_exclusion(relativa, tamano):
            continue
        destino = os.path.join(raiz, relativa)
        os.makedirs(os.path.dirname(destino), exist_ok=True)
        shutil.copyfile(absoluta, destino)
        os.chmod(destino, os.stat(absoluta).st_mode & 0o777)


# --- sonda de estabilidad ---------------------------------------------------


def sonda(repo):
    """Huella del checkout: estado, diffs y hashes de los archivos nuevos.

    Se toma antes y después de escribir la captura. Si cambia, lo capturado no
    describe ningún estado real del repositorio.
    """
    crudo = estado_crudo(repo)
    entradas = parsear_estado(crudo)
    head = head_de(repo)
    base = head["commit"] if head else ARBOL_VACIO
    staged = git(
        repo,
        ["diff", "--cached", "--binary", "--no-color", "--no-ext-diff", "--no-textconv", base],
        base=GIT_DIFF,
    )
    unstaged = git(
        repo,
        ["diff", "--binary", "--no-color", "--no-ext-diff", "--no-textconv"],
        base=GIT_DIFF,
    )
    nuevos = {}
    for e in entradas:
        if not e["xy"].startswith("??"):
            continue
        relativa = ruta_segura(e["_bytes"])
        absoluta = os.path.join(repo, relativa)
        if relativa.startswith(PREFIJOS_SIN_HASH):
            nuevos[relativa] = "no-hashado"
        elif os.path.islink(absoluta):
            nuevos[relativa] = "link:" + os.readlink(absoluta)
        elif os.path.isfile(absoluta):
            if os.path.getsize(absoluta) > LIMITE_HASH:
                nuevos[relativa] = "no-hashado"
            else:
                nuevos[relativa] = sha256_archivo(absoluta)
        else:
            nuevos[relativa] = "no-regular"
    return {
        "head": head["commit"] if head else None,
        "estado": sha256_bytes(crudo),
        "staged": sha256_bytes(staged),
        "unstaged": sha256_bytes(unstaged),
        "nuevos": nuevos,
    }


def comparar_sondas(a, b):
    diferencias = []
    for clave in ("head", "estado", "staged", "unstaged"):
        if a[clave] != b[clave]:
            diferencias.append({"campo": clave, "antes": a[clave], "despues": b[clave]})
    for ruta in sorted(set(a["nuevos"]) | set(b["nuevos"])):
        antes, despues = a["nuevos"].get(ruta), b["nuevos"].get(ruta)
        if antes != despues:
            diferencias.append({"campo": f"nuevos/{ruta}", "antes": antes, "despues": despues})
    return diferencias


# --- entorno y herramientas -------------------------------------------------


def variables_entorno():
    salida = {}
    for nombre in tuple(VARIABLES_SOSO) + tuple(VARIABLES_BUILD):
        if SECRETO.search(nombre):
            salida[nombre] = {
                "definida": nombre in os.environ,
                "valor": None,
                "motivo": "nombre con aspecto de secreto",
            }
        elif nombre in os.environ:
            salida[nombre] = {"definida": True, "valor": os.environ[nombre]}
        else:
            salida[nombre] = {"definida": False, "valor": None}
    return salida


def primera_linea(datos):
    texto = datos.decode("utf-8", "replace").strip()
    return texto.splitlines()[0] if texto else ""


def sondar_herramientas(registro, cwd):
    salida = []
    for nombre, argv in HERRAMIENTAS:
        r = registro.ejecutar(f"tool-{nombre}", argv, cwd, timeout=60)
        disponible = r["exit_code"] == 0
        salida.append(
            {
                "nombre": nombre,
                "argv": argv,
                "disponible": disponible,
                "exit_code": r["exit_code"],
                "version": primera_linea(r["stdout"] or r["stderr"]) if disponible else None,
                "motivo": r["motivo"],
                "log": r["log"],
            }
        )
    return salida


def toolchain_declarada(repo):
    salida = {"rust_toolchain_toml": None, "lockfiles": []}
    ruta = os.path.join(repo, "rust-toolchain.toml")
    if os.path.isfile(ruta):
        with open(ruta, "rb") as f:
            datos = f.read()
        texto = datos.decode("utf-8", "replace")
        canal = re.search(r'^\s*channel\s*=\s*"([^"]+)"', texto, re.MULTILINE)
        salida["rust_toolchain_toml"] = {
            "sha256": sha256_bytes(datos),
            "bytes": len(datos),
            "canal": canal.group(1) if canal else None,
        }
    for relativa in LOCKFILES:
        ruta = os.path.join(repo, relativa)
        if os.path.isfile(ruta):
            salida["lockfiles"].append(
                {"ruta": relativa, "sha256": sha256_archivo(ruta), "bytes": os.path.getsize(ruta)}
            )
    return salida


def exclude_local(repo, salida):
    """`.git/info/exclude` no viaja en un clone: sin él, el árbol reconstruido
    puede mostrar archivos nuevos que el original ocultaba."""
    git_dir = git(repo, ["rev-parse", "--git-dir"]).decode().strip()
    if not os.path.isabs(git_dir):
        git_dir = os.path.join(repo, git_dir)
    ruta = os.path.join(git_dir, "info", "exclude")
    if not os.path.isfile(ruta):
        return None
    destino = os.path.join(salida, "git", "info-exclude")
    os.makedirs(os.path.dirname(destino), exist_ok=True)
    shutil.copyfile(ruta, destino)
    return {"archivo": "git/info-exclude", "sha256": sha256_archivo(ruta)}


# --- destino ----------------------------------------------------------------


def validar_destino(repo, salida):
    """Exige un destino nuevo y fuera de los archivos fuente del repo."""
    repo_real = os.path.realpath(repo)
    salida_real = os.path.realpath(salida)
    if salida_real == repo_real:
        raise ErrorCaptura("el destino no puede ser el propio repositorio")
    if repo_real.startswith(salida_real + os.sep):
        raise ErrorCaptura("el destino no puede contener al repositorio")
    if os.path.lexists(salida):
        if not os.path.isdir(salida) or os.path.islink(salida):
            raise ErrorCaptura(f"el destino existe y no es un directorio: {salida}")
        if os.listdir(salida):
            raise ErrorCaptura(f"el destino existe y no está vacío: {salida}")
    if salida_real.startswith(repo_real + os.sep):
        relativa = os.path.relpath(salida_real, repo_real)
        p = subprocess.run(
            GIT_BASE + ["check-ignore", "-q", "--no-index", relativa],
            cwd=repo,
            capture_output=True,
            check=False,
            env=entorno_git(repo),
        )
        if p.returncode != 0:
            raise ErrorCaptura(
                f"destino dentro de los archivos fuente del repo: {relativa} "
                "(solo se admite una ruta ignorada por git, p. ej. target/)"
            )


# --- captura ----------------------------------------------------------------


def escribir_json(ruta, datos):
    """Escritura durable: temporal, flush y rename (C5)."""
    temporal = ruta + ".tmp"
    with open(temporal, "w", encoding="utf-8") as f:
        json.dump(datos, f, indent=2, ensure_ascii=False)
        f.write("\n")
        f.flush()
        os.fsync(f.fileno())
    os.replace(temporal, ruta)


def capturar(repo, salida, omitir_herramientas=False):
    if not os.path.isdir(os.path.join(repo, ".git")) and not os.path.isfile(
        os.path.join(repo, ".git")
    ):
        p = subprocess.run(
            GIT_BASE + ["rev-parse", "--is-inside-work-tree"],
            cwd=repo, capture_output=True, check=False, env=entorno_git(repo),
        )
        if p.returncode != 0:
            raise ErrorCaptura(f"{repo} no es un repositorio git")
    validar_destino(repo, salida)
    os.makedirs(salida, exist_ok=True)
    registro = Registro(os.path.join(salida, "logs"))

    antes = sonda(repo)

    crudo = git(repo, ["status", "--porcelain=v1", "-z", "--untracked-files=all"],
                registro, "git-status")
    entradas = parsear_estado(crudo)
    head = head_de(repo)
    diffs = diffs_de(repo, head, registro)

    os.makedirs(os.path.join(salida, "diff"), exist_ok=True)
    seccion_diffs = {}
    for nombre in ("staged", "unstaged"):
        datos, nombres = diffs[nombre]
        destino = os.path.join(salida, "diff", f"{nombre}.diff")
        with open(destino, "wb") as f:
            f.write(datos)
        seccion_diffs[nombre] = {
            "archivo": f"diff/{nombre}.diff",
            "sha256": sha256_bytes(datos),
            "bytes": len(datos),
            "archivos": parsear_name_status(nombres),
        }

    incluidos, excluidos = clasificar_nuevos(repo, entradas)
    guardar_fuentes(repo, salida, entradas)

    herramientas = [] if omitir_herramientas else sondar_herramientas(registro, repo)

    estado_publico = []
    for e in entradas:
        limpio = {k: v for k, v in e.items() if not k.startswith("_")}
        estado_publico.append(limpio)

    manifiesto = {
        "schema_version": ESQUEMA,
        "herramienta": "scripts/self-improvement/baseline.py",
        "creado": ahora(),
        "repo": {"ruta": os.path.realpath(repo)},
        "git": {
            "head": head,
            "estado": estado_publico,
            "estado_sha256": sha256_bytes(crudo),
            "diffs": seccion_diffs,
            "info_exclude": exclude_local(repo, salida),
            "comandos": {
                "estado": GIT_BASE + ["status", "--porcelain=v1", "-z", "--untracked-files=all"],
                "diff_base": GIT_DIFF,
            },
        },
        "nuevos": {
            "incluidos": incluidos,
            "excluidos": excluidos,
            "limite_fuente_bytes": LIMITE_FUENTE,
            "prefijos_excluidos": list(PREFIJOS_EXCLUIDOS),
        },
        "reproduccion": {
            "completa": not excluidos,
            "no_almacenados": [r["ruta"] for r in excluidos],
            "nota": "la base declarada excluye los archivos listados; su ruta y "
                    "hash quedan registrados, su contenido no",
        },
        "entorno": {
            "politica": "lista explícita de variables; el entorno completo no se versiona",
            "variables": variables_entorno(),
        },
        "herramientas": herramientas,
        "toolchain": toolchain_declarada(repo),
        "suites_base": [
            {
                "nombre": nombre,
                "argv": argv,
                "cwd": cwd,
                "estado": "no ejecutada",
                "motivo": "las suites base se ejecutan como paso explícito "
                          "(`baseline.py suites`) para no ocultar un build largo",
            }
            for nombre, argv, cwd in SUITES_BASE
        ],
    }

    despues = sonda(repo)
    diferencias = comparar_sondas(antes, despues)
    manifiesto["estabilidad"] = {
        "sondas": 2,
        "estable": not diferencias,
        "sonda": {k: v for k, v in antes.items() if k != "nuevos"},
    }
    if diferencias:
        escribir_json(os.path.join(salida, "inestable.json"),
                      {"schema_version": ESQUEMA, "creado": ahora(), "diferencias": diferencias})
        raise CapturaInestable(diferencias)

    escribir_json(os.path.join(salida, "baseline.json"), manifiesto)
    return manifiesto


def resumen_estable(manifiesto):
    """Manifiesto sin campos volátiles, para comparar dos capturas iguales."""
    copia = copy.deepcopy(manifiesto)
    copia.pop("creado", None)
    return copia


# --- reconstrucción ---------------------------------------------------------


def leer_manifiesto(captura):
    ruta = os.path.join(captura, "baseline.json")
    if not os.path.isfile(ruta):
        raise ErrorCaptura(f"no hay baseline.json en {captura}")
    with open(ruta, encoding="utf-8") as f:
        manifiesto = json.load(f)
    if manifiesto.get("schema_version") != ESQUEMA:
        raise ErrorCaptura(f"schema_version no soportada: {manifiesto.get('schema_version')}")
    return manifiesto


def recuperar_excluidos(manifiesto, origen, destino):
    """Trae del origen los archivos declarados pero no almacenados.

    La captura no guarda blobs grandes (firmware, modelos), pero algunos son
    entradas que las suites necesitan: `cargo xtask check` exige el firmware
    de iwlwifi en el rootfs. Se copian del repositorio de origen y se
    comprueban contra el hash del manifiesto: si no coincide, el origen ya no
    es el de la captura y no se acepta el archivo.
    """
    recuperados, problemas = [], []
    for entrada in manifiesto["nuevos"]["excluidos"]:
        if not entrada.get("sha256"):
            continue  # artefactos sin hash: no forman parte de la base
        relativa = entrada["ruta"]
        if "ruta_b64" in entrada:
            relativa = ruta_fs(base64.b64decode(entrada["ruta_b64"]))
        fuente = os.path.join(origen, relativa)
        if not os.path.isfile(fuente):
            problemas.append({"campo": f"excluidos/{relativa}",
                              "esperado": entrada["sha256"], "obtenido": "ausente en el origen"})
            continue
        real = sha256_archivo(fuente)
        if real != entrada["sha256"]:
            problemas.append({"campo": f"excluidos/{relativa}",
                              "esperado": entrada["sha256"], "obtenido": real})
            continue
        ruta_destino = os.path.join(destino, relativa)
        os.makedirs(os.path.dirname(ruta_destino), exist_ok=True)
        shutil.copyfile(fuente, ruta_destino)
        recuperados.append(relativa)
    return recuperados, problemas


def reconstruir(captura, destino, origen=None, con_excluidos=False):
    """Rehace la base declarada en un árbol nuevo y verifica sus hashes."""
    manifiesto = leer_manifiesto(captura)
    if os.path.lexists(destino) and (not os.path.isdir(destino) or os.listdir(destino)):
        raise ErrorCaptura(f"el destino existe y no está vacío: {destino}")
    origen = origen or manifiesto["repo"]["ruta"]
    if not os.path.isdir(origen):
        raise ErrorCaptura(f"no se encuentra el repositorio de origen {origen}")
    # Rutas absolutas: `git clone` se ejecuta con otro cwd y una ruta relativa
    # apuntaría a otro sitio.
    origen = os.path.realpath(origen)
    destino = os.path.abspath(destino)
    captura = os.path.abspath(captura)
    os.makedirs(destino, exist_ok=True)
    # Los logs viven en la captura: el árbol reconstruido debe quedar
    # exactamente en la base declarada, sin archivos añadidos por esto.
    registro = Registro(os.path.join(captura, "logs"))

    head = manifiesto["git"]["head"]
    r = registro.ejecutar("clone", ["git", "clone", "--quiet", "--no-checkout", origen, destino],
                          os.path.dirname(destino) or ".")
    if r["exit_code"] != 0:
        raise ErrorCaptura(f"git clone falló: {r['stderr'].decode('utf-8', 'replace')}")
    if not os.path.isdir(os.path.join(destino, ".git")):
        raise ErrorCaptura(
            f"el clone no dejó un repositorio en {destino}; no se ejecuta nada más "
            "(un git sin repo aquí actuaría sobre el repositorio que lo contenga)"
        )
    if head:
        r = registro.ejecutar("checkout",
                              argv_git_arbol(destino, ["checkout", "--detach", head["commit"]]),
                              destino, env=entorno_git(destino))
        if r["exit_code"] != 0:
            raise ErrorCaptura(f"checkout de {head['commit']} falló: "
                               f"{r['stderr'].decode('utf-8', 'replace')}")

    info = manifiesto["git"].get("info_exclude")
    if info:
        # Un clone no siempre trae `.git/info/`: sin este archivo, el árbol
        # reconstruido mostraría como nuevos archivos que el original ocultaba.
        destino_info = os.path.join(destino, ".git", "info")
        os.makedirs(destino_info, exist_ok=True)
        shutil.copyfile(os.path.join(captura, info["archivo"]),
                        os.path.join(destino_info, "exclude"))

    for nombre, extra in (("staged", ["--index"]), ("unstaged", [])):
        seccion = manifiesto["git"]["diffs"][nombre]
        if seccion["bytes"] == 0:
            continue
        parche = os.path.join(captura, seccion["archivo"])
        r = registro.ejecutar(
            f"apply-{nombre}",
            argv_git_arbol(destino, ["apply", "--whitespace=nowarn"] + extra + [parche]),
            destino,
            env=entorno_git(destino),
        )
        if r["exit_code"] != 0:
            raise ErrorCaptura(
                f"no se pudo aplicar {nombre}: {r['stderr'].decode('utf-8', 'replace')}"
            )

    for entrada in manifiesto["nuevos"]["incluidos"]:
        relativa = entrada.get("ruta")
        if "ruta_b64" in entrada:
            relativa = ruta_fs(base64.b64decode(entrada["ruta_b64"]))
        ruta_destino = os.path.join(destino, relativa)
        os.makedirs(os.path.dirname(ruta_destino), exist_ok=True)
        if entrada.get("tipo") == "symlink":
            if os.path.lexists(ruta_destino):
                os.remove(ruta_destino)
            os.symlink(entrada["destino"], ruta_destino)
            continue
        shutil.copyfile(os.path.join(captura, "sources", relativa), ruta_destino)
        os.chmod(ruta_destino, int(entrada.get("modo", "0o644"), 8))

    verificacion = verificar(manifiesto, destino)
    if con_excluidos:
        recuperados, problemas = recuperar_excluidos(manifiesto, origen, destino)
        verificacion["recuperados_del_origen"] = recuperados
        verificacion["no_reproducidos"] = [
            r for r in verificacion["no_reproducidos"] if r not in recuperados
        ]
        verificacion["problemas"].extend(problemas)
        verificacion["ok"] = not verificacion["problemas"]
    escribir_json(os.path.join(captura, "reconstruccion.json"), verificacion)
    return verificacion


def verificar(manifiesto, arbol):
    """Compara el árbol reconstruido con la base declarada."""
    problemas = []
    actual = sonda(arbol)
    esperado = manifiesto["estabilidad"]["sonda"]
    for clave in ("head", "staged", "unstaged"):
        if actual[clave] != esperado[clave]:
            problemas.append({"campo": clave, "esperado": esperado[clave], "obtenido": actual[clave]})
    for entrada in manifiesto["nuevos"]["incluidos"]:
        relativa = entrada["ruta"]
        if "ruta_b64" in entrada:
            relativa = ruta_fs(base64.b64decode(entrada["ruta_b64"]))
        ruta = os.path.join(arbol, relativa)
        if entrada.get("tipo") == "symlink":
            if not os.path.islink(ruta) or os.readlink(ruta) != entrada["destino"]:
                problemas.append({"campo": f"nuevos/{relativa}", "esperado": "symlink",
                                  "obtenido": "ausente o distinto"})
            continue
        if not os.path.isfile(ruta):
            problemas.append({"campo": f"nuevos/{relativa}", "esperado": entrada["sha256"],
                              "obtenido": None})
            continue
        real = sha256_archivo(ruta)
        if real != entrada["sha256"]:
            problemas.append({"campo": f"nuevos/{relativa}", "esperado": entrada["sha256"],
                              "obtenido": real})
    return {
        "schema_version": ESQUEMA,
        "creado": ahora(),
        "arbol": os.path.realpath(arbol),
        "ok": not problemas,
        "problemas": problemas,
        "no_reproducidos": manifiesto["reproduccion"]["no_almacenados"],
    }


# --- suites base ------------------------------------------------------------


def ejecutar_suites(captura, arbol, solo=None, timeout=None):
    """Paso explícito posterior: ejecuta las suites base y anota su resultado."""
    manifiesto = leer_manifiesto(captura)
    registro = Registro(os.path.join(captura, "logs"))
    resultados = []
    for nombre, argv, cwd in SUITES_BASE:
        if solo and nombre not in solo:
            resultados.append({"nombre": nombre, "argv": argv, "cwd": cwd,
                               "estado": "omitida", "exit_code": None})
            continue
        destino = os.path.join(arbol, cwd)
        r = registro.ejecutar(f"suite-{nombre}", argv, destino, timeout=timeout)
        if r["exit_code"] == 0:
            estado = "ok"
        elif r["exit_code"] is None:
            estado = "no ejecutada"
        else:
            estado = "fallo"
        resultados.append({
            "nombre": nombre, "argv": argv, "cwd": cwd, "estado": estado,
            "exit_code": r["exit_code"], "motivo": r["motivo"], "log": r["log"],
        })
        print(f"suites: {nombre} → {estado} (exit {r['exit_code']})", flush=True)
    salida = {
        "schema_version": ESQUEMA,
        "creado": ahora(),
        "arbol": os.path.realpath(arbol),
        "base": manifiesto["git"]["head"]["commit"] if manifiesto["git"]["head"] else None,
        "resultados": resultados,
    }
    escribir_json(os.path.join(captura, "suites.json"), salida)
    return salida


# --- CLI --------------------------------------------------------------------


def construir_parser():
    p = argparse.ArgumentParser(
        prog="baseline.py", description="Captura reproducible de la base de trabajo"
    )
    sub = p.add_subparsers(dest="comando", required=True)

    c = sub.add_parser("capture", help="captura la base del checkout")
    c.add_argument("--repo", required=True, help="repositorio a capturar")
    c.add_argument("--out", required=True, help="destino nuevo, fuera de los archivos fuente")
    c.add_argument("--omitir-herramientas", action="store_true",
                   help="no sondear versiones de herramientas")

    r = sub.add_parser("reconstruct", help="reconstruye y verifica una captura")
    r.add_argument("--capture", required=True)
    r.add_argument("--into", required=True)
    r.add_argument("--from-repo", default=None, help="repo de origen (por defecto, el del manifiesto)")
    r.add_argument("--with-excluded", action="store_true",
                   help="copiar del origen los archivos declarados y no almacenados, "
                        "comprobando su hash (firmware y demás entradas grandes)")

    s = sub.add_parser("suites", help="ejecuta las suites base sobre un árbol")
    s.add_argument("--capture", required=True)
    s.add_argument("--tree", required=True)
    s.add_argument("--only", action="append", default=None)
    s.add_argument("--timeout", type=float, default=None)
    return p


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    # Forma documentada en la ficha: `baseline.py --repo X --out Y`.
    if argv and argv[0].startswith("-") and argv[0] not in ("-h", "--help"):
        argv.insert(0, "capture")
    args = construir_parser().parse_args(argv)
    try:
        if args.comando == "capture":
            m = capturar(args.repo, args.out, args.omitir_herramientas)
            print(f"captura en {args.out}: "
                  f"{len(m['nuevos']['incluidos'])} archivo(s) nuevo(s) guardado(s), "
                  f"{len(m['nuevos']['excluidos'])} excluido(s)")
            return 0
        if args.comando == "reconstruct":
            v = reconstruir(args.capture, args.into, args.from_repo, args.with_excluded)
            if v["ok"]:
                print(f"reconstrucción verificada en {args.into}")
                return 0
            print(f"reconstrucción con {len(v['problemas'])} problema(s)", file=sys.stderr)
            for p in v["problemas"][:20]:
                print(f"  {p['campo']}: esperado {p['esperado']} obtenido {p['obtenido']}",
                      file=sys.stderr)
            return 5
        s = ejecutar_suites(args.capture, args.tree, args.only, args.timeout)
        fallos = [r for r in s["resultados"] if r["estado"] == "fallo"]
        return 4 if fallos else 0
    except CapturaInestable as e:
        print("captura inestable: el checkout cambió mientras se leía", file=sys.stderr)
        for d in e.diferencias[:20]:
            print(f"  {d['campo']}: {d['antes']} → {d['despues']}", file=sys.stderr)
        return 3
    except ErrorCaptura as e:
        print(f"error: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
