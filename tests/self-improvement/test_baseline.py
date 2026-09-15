#!/usr/bin/env python3
"""Pruebas de `scripts/self-improvement/baseline.py` (ficha T01).

Cada fixture es un repositorio git real creado en un temporal: el formato de
`git status -z`, los parches binarios y el comportamiento de `git apply` son
justo lo que hay que comprobar, así que no se simula git.

Las pruebas verifican las dos propiedades que dan valor a la captura:

  - el checkout de origen queda byte a byte igual después de capturarlo;
  - la captura reconstruye la base declarada y sus hashes coinciden.
"""
import hashlib
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

RAIZ = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SCRIPT = os.path.join(RAIZ, "scripts", "self-improvement", "baseline.py")


def cargar_modulo():
    spec = importlib.util.spec_from_file_location("baseline", SCRIPT)
    modulo = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(modulo)
    return modulo


baseline = cargar_modulo()


def git(repo, *argumentos):
    p = subprocess.run(["git"] + list(argumentos), cwd=repo, capture_output=True, check=False)
    if p.returncode != 0:
        raise AssertionError(
            f"git {' '.join(argumentos)} falló: {p.stderr.decode('utf-8', 'replace')}"
        )
    return p.stdout


def escribir(repo, relativa, contenido, modo=None):
    ruta = os.path.join(repo, relativa)
    os.makedirs(os.path.dirname(ruta), exist_ok=True)
    datos = contenido.encode() if isinstance(contenido, str) else contenido
    with open(ruta, "wb") as f:
        f.write(datos)
    if modo is not None:
        os.chmod(ruta, modo)
    return ruta


def sha256_archivo(ruta):
    h = hashlib.sha256()
    with open(ruta, "rb") as f:
        for trozo in iter(lambda: f.read(1 << 16), b""):
            h.update(trozo)
    return h.hexdigest()


def huella_arbol(repo):
    """Estado observable del checkout: git status, contenido e índice.

    Sirve para exigir que capturar no altere nada, ni siquiera `.git/index`.
    """
    estado = git(repo, "--no-optional-locks", "status", "--porcelain=v1", "-z", "-uall")
    archivos = {}
    for base, dirs, nombres in os.walk(repo):
        if ".git" in dirs:
            dirs.remove(".git")
        for nombre in nombres:
            ruta = os.path.join(base, nombre)
            relativa = os.path.relpath(ruta, repo)
            if os.path.islink(ruta):
                archivos[relativa] = "link:" + os.readlink(ruta)
            else:
                archivos[relativa] = f"{sha256_archivo(ruta)}:{oct(os.stat(ruta).st_mode & 0o777)}"
    indice = os.path.join(repo, ".git", "index")
    return {
        "estado": estado,
        "archivos": archivos,
        "indice": sha256_archivo(indice) if os.path.isfile(indice) else None,
    }


class BaseTemporal(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # Entorno git hermético: la configuración global del usuario no debe
        # cambiar el resultado de estas pruebas.
        cls._entorno = {}
        for clave, valor in (
            ("GIT_CONFIG_GLOBAL", os.devnull),
            ("GIT_CONFIG_SYSTEM", os.devnull),
            ("GIT_AUTHOR_NAME", "t01"),
            ("GIT_AUTHOR_EMAIL", "t01@example.invalid"),
            ("GIT_COMMITTER_NAME", "t01"),
            ("GIT_COMMITTER_EMAIL", "t01@example.invalid"),
        ):
            cls._entorno[clave] = os.environ.get(clave)
            os.environ[clave] = valor

    @classmethod
    def tearDownClass(cls):
        for clave, valor in cls._entorno.items():
            if valor is None:
                os.environ.pop(clave, None)
            else:
                os.environ[clave] = valor

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="t01-")
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.repo = os.path.join(self.tmp, "repo")
        os.makedirs(self.repo)
        git(self.repo, "init", "--quiet", "-b", "main")
        escribir(self.repo, "README.md", "base\n")
        escribir(self.repo, "src/lib.rs", "pub fn uno() -> u32 { 1 }\n")
        escribir(self.repo, "datos/blob.bin", bytes(range(256)) * 8)
        escribir(self.repo, ".gitignore", "/target/\n")
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "--quiet", "-m", "base")

    def destino(self, nombre="captura"):
        return os.path.join(self.tmp, nombre)

    def capturar(self, salida=None, **kwargs):
        salida = salida or self.destino()
        kwargs.setdefault("omitir_herramientas", True)
        return baseline.capturar(self.repo, salida, **kwargs), salida


class TestCaptura(BaseTemporal):
    def test_repo_limpio(self):
        """Fixture: repo limpio. Sin diffs, sin archivos nuevos, base completa."""
        manifiesto, salida = self.capturar()
        self.assertTrue(os.path.isfile(os.path.join(salida, "baseline.json")))
        self.assertEqual(manifiesto["schema_version"], 1)
        self.assertEqual(manifiesto["git"]["estado"], [])
        self.assertEqual(manifiesto["git"]["diffs"]["staged"]["bytes"], 0)
        self.assertEqual(manifiesto["git"]["diffs"]["unstaged"]["bytes"], 0)
        self.assertEqual(manifiesto["nuevos"]["incluidos"], [])
        self.assertTrue(manifiesto["reproduccion"]["completa"])
        self.assertTrue(manifiesto["estabilidad"]["estable"])
        commit = git(self.repo, "rev-parse", "HEAD").decode().strip()
        self.assertEqual(manifiesto["git"]["head"]["commit"], commit)

    def test_staged_y_unstaged_sobre_el_mismo_archivo(self):
        """Fixture: staged+unstaged sobre un mismo archivo."""
        escribir(self.repo, "src/lib.rs", "pub fn uno() -> u32 { 2 }\n")
        git(self.repo, "add", "src/lib.rs")
        escribir(self.repo, "src/lib.rs", "pub fn uno() -> u32 { 3 }\n")
        manifiesto, salida = self.capturar()

        entradas = {e["ruta"]: e["xy"] for e in manifiesto["git"]["estado"]}
        self.assertEqual(entradas["src/lib.rs"], "MM")
        self.assertGreater(manifiesto["git"]["diffs"]["staged"]["bytes"], 0)
        self.assertGreater(manifiesto["git"]["diffs"]["unstaged"]["bytes"], 0)

        destino = os.path.join(self.tmp, "arbol")
        verificacion = baseline.reconstruir(salida, destino)
        self.assertTrue(verificacion["ok"], verificacion["problemas"])
        # El árbol reconstruido conserva las dos capas: índice y árbol.
        with open(os.path.join(destino, "src/lib.rs"), encoding="utf-8") as f:
            self.assertEqual(f.read(), "pub fn uno() -> u32 { 3 }\n")
        indexado = git(destino, "show", ":src/lib.rs").decode()
        self.assertEqual(indexado, "pub fn uno() -> u32 { 2 }\n")

    def test_binario(self):
        """Fixture: binario. El parche `--binary` debe reproducir los bytes."""
        contenido = bytes((i * 7 + 13) % 256 for i in range(4096))
        escribir(self.repo, "datos/blob.bin", contenido)
        git(self.repo, "add", "datos/blob.bin")
        contenido2 = bytes((i * 11 + 3) % 256 for i in range(5000))
        escribir(self.repo, "datos/blob.bin", contenido2)
        _, salida = self.capturar()

        destino = os.path.join(self.tmp, "arbol")
        verificacion = baseline.reconstruir(salida, destino)
        self.assertTrue(verificacion["ok"], verificacion["problemas"])
        with open(os.path.join(destino, "datos/blob.bin"), "rb") as f:
            self.assertEqual(f.read(), contenido2)
        self.assertEqual(git(destino, "show", ":datos/blob.bin"), contenido)

    def test_ruta_con_espacios(self):
        """Fixture: ruta con espacios, seguida y nueva; y un renombrado."""
        escribir(self.repo, "un archivo con espacios.txt", "uno\n")
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "--quiet", "-m", "espacios")
        escribir(self.repo, "un archivo con espacios.txt", "dos\n")
        git(self.repo, "mv", "src/lib.rs", "src/otro nombre.rs")
        escribir(self.repo, "nuevo con espacios.txt", "nuevo\n")

        manifiesto, salida = self.capturar()
        rutas = {e["ruta"] for e in manifiesto["git"]["estado"]}
        self.assertIn("un archivo con espacios.txt", rutas)
        self.assertIn("nuevo con espacios.txt", rutas)
        # El renombrado ocupa dos campos NUL: se conserva el origen.
        renombrados = [e for e in manifiesto["git"]["estado"] if "R" in e["xy"]]
        self.assertEqual(len(renombrados), 1)
        self.assertEqual(renombrados[0]["origen"], "src/lib.rs")
        self.assertEqual(renombrados[0]["ruta"], "src/otro nombre.rs")

        fuente = os.path.join(salida, "sources", "nuevo con espacios.txt")
        self.assertTrue(os.path.isfile(fuente))
        destino = os.path.join(self.tmp, "arbol")
        verificacion = baseline.reconstruir(salida, destino)
        self.assertTrue(verificacion["ok"], verificacion["problemas"])
        with open(os.path.join(destino, "un archivo con espacios.txt"), encoding="utf-8") as f:
            self.assertEqual(f.read(), "dos\n")

    def test_archivo_nuevo_y_exclusiones(self):
        """Fixture: archivo nuevo. Contenido guardado; grandes y artefactos, no."""
        escribir(self.repo, "notas/apunte.md", "hola\n", modo=0o640)
        escribir(self.repo, "ejecutable.sh", "#!/bin/sh\n", modo=0o755)
        grande = os.urandom(baseline.LIMITE_FUENTE + 512)
        escribir(self.repo, "grande.bin", grande)
        escribir(self.repo, "target/artefacto.o", b"\x00" * 16)  # ignorado por git

        manifiesto, salida = self.capturar()
        incluidos = {e["ruta"]: e for e in manifiesto["nuevos"]["incluidos"]}
        excluidos = {e["ruta"]: e for e in manifiesto["nuevos"]["excluidos"]}

        self.assertIn("notas/apunte.md", incluidos)
        self.assertEqual(incluidos["notas/apunte.md"]["sha256"],
                         sha256_archivo(os.path.join(self.repo, "notas/apunte.md")))
        self.assertEqual(incluidos["ejecutable.sh"]["modo"], "0o755")
        with open(os.path.join(salida, "sources", "notas/apunte.md"), encoding="utf-8") as f:
            self.assertEqual(f.read(), "hola\n")

        # El grande se declara con ruta y hash, pero su contenido no se guarda.
        self.assertIn("grande.bin", excluidos)
        self.assertEqual(excluidos["grande.bin"]["sha256"], hashlib.sha256(grande).hexdigest())
        self.assertIn("tamaño", excluidos["grande.bin"]["motivo"])
        self.assertFalse(os.path.exists(os.path.join(salida, "sources", "grande.bin")))
        self.assertFalse(manifiesto["reproduccion"]["completa"])
        self.assertIn("grande.bin", manifiesto["reproduccion"]["no_almacenados"])

        # `target/` está ignorado: ni aparece en el estado ni se recorre.
        self.assertNotIn("target/artefacto.o", incluidos)
        self.assertNotIn("target/artefacto.o", excluidos)

        destino = os.path.join(self.tmp, "arbol")
        verificacion = baseline.reconstruir(salida, destino)
        self.assertTrue(verificacion["ok"], verificacion["problemas"])
        self.assertEqual(verificacion["no_reproducidos"], ["grande.bin"])
        self.assertEqual(oct(os.stat(os.path.join(destino, "ejecutable.sh")).st_mode & 0o777),
                         "0o755")

    def test_recuperar_excluidos_desde_el_origen(self):
        """Los blobs declarados y no almacenados se recuperan verificando hash."""
        grande = os.urandom(baseline.LIMITE_FUENTE + 64)
        escribir(self.repo, "rootfs/lib/firmware/iwlwifi-prueba.ucode", grande)
        _, salida = self.capturar()

        destino = os.path.join(self.tmp, "arbol")
        verificacion = baseline.reconstruir(salida, destino, con_excluidos=True)
        self.assertTrue(verificacion["ok"], verificacion["problemas"])
        self.assertEqual(verificacion["recuperados_del_origen"],
                         ["rootfs/lib/firmware/iwlwifi-prueba.ucode"])
        self.assertEqual(verificacion["no_reproducidos"], [])
        recuperado = os.path.join(destino, "rootfs/lib/firmware/iwlwifi-prueba.ucode")
        self.assertEqual(sha256_archivo(recuperado), hashlib.sha256(grande).hexdigest())

    def test_recuperar_excluidos_rechaza_un_origen_movido(self):
        """Si el origen ya no tiene el contenido capturado, no se acepta."""
        escribir(self.repo, "rootfs/lib/firmware/iwlwifi-prueba.ucode",
                 os.urandom(baseline.LIMITE_FUENTE + 64))
        _, salida = self.capturar()
        escribir(self.repo, "rootfs/lib/firmware/iwlwifi-prueba.ucode",
                 os.urandom(baseline.LIMITE_FUENTE + 64))

        destino = os.path.join(self.tmp, "arbol")
        verificacion = baseline.reconstruir(salida, destino, con_excluidos=True)
        self.assertFalse(verificacion["ok"])
        self.assertEqual(verificacion["problemas"][0]["campo"],
                         "excluidos/rootfs/lib/firmware/iwlwifi-prueba.ucode")
        self.assertFalse(os.path.exists(
            os.path.join(destino, "rootfs/lib/firmware/iwlwifi-prueba.ucode")))

    def test_modificacion_concurrente(self):
        """Fixture: modificación concurrente. La captura se declara inestable."""
        escribir(self.repo, "notas/apunte.md", "antes\n")
        original = baseline.guardar_fuentes
        vueltas = []

        def entrometido(repo, salida, entradas):
            original(repo, salida, entradas)
            # Contenido distinto en cada captura: si se repitiera el mismo, la
            # segunda captura ya no vería ningún cambio y sería estable.
            vueltas.append(len(vueltas))
            escribir(repo, "notas/apunte.md", f"cambiado a mitad {len(vueltas)}\n")

        baseline.guardar_fuentes = entrometido
        self.addCleanup(setattr, baseline, "guardar_fuentes", original)

        salida = self.destino()
        with self.assertRaises(baseline.CapturaInestable):
            baseline.capturar(self.repo, salida, omitir_herramientas=True)
        self.assertFalse(os.path.exists(os.path.join(salida, "baseline.json")))
        with open(os.path.join(salida, "inestable.json"), encoding="utf-8") as f:
            inestable = json.load(f)
        campos = {d["campo"] for d in inestable["diferencias"]}
        self.assertIn("nuevos/notas/apunte.md", campos)

        codigo = baseline.main(["capture", "--repo", self.repo, "--out", self.destino("otra"),
                                "--omitir-herramientas"])
        self.assertEqual(codigo, 3)

    def test_el_checkout_no_se_altera(self):
        """Comparación del estado del repo antes y después de capturar."""
        escribir(self.repo, "src/lib.rs", "pub fn uno() -> u32 { 9 }\n")
        git(self.repo, "add", "src/lib.rs")
        escribir(self.repo, "src/lib.rs", "pub fn uno() -> u32 { 10 }\n")
        escribir(self.repo, "nuevo.txt", "nuevo\n")
        antes = huella_arbol(self.repo)
        self.capturar()
        self.assertEqual(huella_arbol(self.repo), antes)

    def test_no_actua_sobre_el_repositorio_que_lo_contiene(self):
        """Regresión: un git lanzado en un directorio sin repo subía al de fuera.

        Así fue como una reconstrucción movió el HEAD del checkout real: el
        `checkout --detach` corrió en un destino vacío y git encontró el
        repositorio de encima.
        """
        dentro = os.path.join(self.repo, "sin-repo")
        os.makedirs(dentro)
        antes = git(self.repo, "rev-parse", "HEAD")
        with self.assertRaises(baseline.ErrorCaptura):
            baseline.capturar(dentro, self.destino(), omitir_herramientas=True)
        self.assertEqual(git(self.repo, "rev-parse", "HEAD"), antes)

        # Y una reconstrucción dentro de otro repositorio no lo toca.
        _, salida = self.capturar()
        exterior = os.path.join(self.tmp, "exterior")
        os.makedirs(exterior)
        git(exterior, "init", "--quiet", "-b", "main")
        escribir(exterior, "a.txt", "a\n")
        git(exterior, "add", "-A")
        git(exterior, "commit", "--quiet", "-m", "exterior")
        huella_exterior = huella_arbol(exterior)
        head_exterior = git(exterior, "rev-parse", "HEAD")
        reflog_exterior = git(exterior, "reflog")

        verificacion = baseline.reconstruir(salida, os.path.join(exterior, "sub", "arbol"))
        self.assertTrue(verificacion["ok"], verificacion["problemas"])
        self.assertEqual(git(exterior, "rev-parse", "HEAD"), head_exterior)
        self.assertEqual(git(exterior, "reflog"), reflog_exterior)
        self.assertEqual(huella_arbol(exterior)["indice"], huella_exterior["indice"])

    def test_destino_nuevo_y_fuera_de_las_fuentes(self):
        ocupado = self.destino("ocupado")
        os.makedirs(ocupado)
        escribir(ocupado, "algo.txt", "x\n")
        with self.assertRaises(baseline.ErrorCaptura):
            baseline.capturar(self.repo, ocupado, omitir_herramientas=True)

        dentro = os.path.join(self.repo, "captura")
        with self.assertRaises(baseline.ErrorCaptura):
            baseline.capturar(self.repo, dentro, omitir_herramientas=True)
        self.assertFalse(os.path.exists(dentro))

        with self.assertRaises(baseline.ErrorCaptura):
            baseline.capturar(self.repo, self.repo, omitir_herramientas=True)

        # Una ruta ignorada por git sí es un destino válido.
        ignorado = os.path.join(self.repo, "target", "self-improvement", "base")
        manifiesto = baseline.capturar(self.repo, ignorado, omitir_herramientas=True)
        self.assertTrue(manifiesto["estabilidad"]["estable"])

    def test_info_exclude_viaja_con_la_captura(self):
        """`.git/info/exclude` no lo trae un clone; sin él sobran archivos nuevos."""
        escribir(self.repo, ".git/info/exclude", "oculto.txt\n")
        escribir(self.repo, "oculto.txt", "invisible\n")
        escribir(self.repo, "visible.txt", "v\n")
        manifiesto, salida = self.capturar()
        rutas = {e["ruta"] for e in manifiesto["nuevos"]["incluidos"]}
        self.assertIn("visible.txt", rutas)
        self.assertNotIn("oculto.txt", rutas)
        self.assertTrue(os.path.isfile(os.path.join(salida, "git", "info-exclude")))

        destino = os.path.join(self.tmp, "arbol")
        verificacion = baseline.reconstruir(salida, destino)
        self.assertTrue(verificacion["ok"], verificacion["problemas"])
        copiado = os.path.join(destino, ".git", "info", "exclude")
        self.assertEqual(sha256_archivo(copiado), manifiesto["git"]["info_exclude"]["sha256"])

    def test_determinismo_de_la_captura(self):
        escribir(self.repo, "src/lib.rs", "pub fn uno() -> u32 { 4 }\n")
        git(self.repo, "add", "src/lib.rs")
        escribir(self.repo, "nuevo.txt", "n\n")
        una, _ = self.capturar(self.destino("a"))
        otra, _ = self.capturar(self.destino("b"))
        self.assertEqual(baseline.resumen_estable(una), baseline.resumen_estable(otra))

    def test_entorno_solo_lista_explicita(self):
        os.environ["SOSO_BUILD"] = "perfil-de-prueba"
        os.environ["SOSO_FORJA_TOKEN"] = "no-debe-aparecer"
        os.environ["HOME_SECRETO_AJENO"] = "tampoco"
        self.addCleanup(os.environ.pop, "SOSO_BUILD", None)
        self.addCleanup(os.environ.pop, "SOSO_FORJA_TOKEN", None)
        self.addCleanup(os.environ.pop, "HOME_SECRETO_AJENO", None)

        manifiesto, salida = self.capturar()
        variables = manifiesto["entorno"]["variables"]
        self.assertEqual(variables["SOSO_BUILD"]["valor"], "perfil-de-prueba")
        self.assertNotIn("SOSO_FORJA_TOKEN", variables)
        self.assertNotIn("HOME_SECRETO_AJENO", variables)
        with open(os.path.join(salida, "baseline.json"), encoding="utf-8") as f:
            texto = f.read()
        self.assertNotIn("no-debe-aparecer", texto)
        self.assertNotIn("tampoco", texto)

    def test_herramientas_con_log_y_codigo_de_salida(self):
        manifiesto, salida = self.capturar(omitir_herramientas=False)
        herramientas = {h["nombre"]: h for h in manifiesto["herramientas"]}
        self.assertEqual(set(herramientas), {n for n, _ in baseline.HERRAMIENTAS})
        for h in herramientas.values():
            self.assertTrue(os.path.isfile(os.path.join(salida, h["log"])))
            if h["disponible"]:
                self.assertEqual(h["exit_code"], 0)
                self.assertTrue(h["version"])
            else:
                self.assertIsNone(h["version"])
        self.assertTrue(herramientas["git"]["disponible"])
        self.assertTrue(manifiesto["toolchain"]["rust_toolchain_toml"] is None
                        or manifiesto["toolchain"]["rust_toolchain_toml"]["sha256"])

    def test_suites_declaradas_pero_no_ejecutadas(self):
        manifiesto, salida = self.capturar()
        nombres = [s["nombre"] for s in manifiesto["suites_base"]]
        self.assertEqual(nombres, [n for n, _, _ in baseline.SUITES_BASE])
        for s in manifiesto["suites_base"]:
            self.assertEqual(s["estado"], "no ejecutada")
        self.assertFalse(os.path.exists(os.path.join(salida, "suites.json")))

    def test_suites_registran_el_fallo(self):
        """El paso posterior anota el resultado real, no lo oculta."""
        _, salida = self.capturar()
        resultado = baseline.ejecutar_suites(salida, self.repo, solo=["core-std"], timeout=120)
        por_nombre = {r["nombre"]: r for r in resultado["resultados"]}
        # El fixture no es un workspace de cargo: la suite falla y queda escrita.
        self.assertIn(por_nombre["core-std"]["estado"], ("fallo", "no ejecutada"))
        self.assertTrue(os.path.isfile(os.path.join(salida, "suites.json")))
        if por_nombre["core-std"]["log"]:
            self.assertTrue(os.path.isfile(os.path.join(salida, por_nombre["core-std"]["log"])))
        self.assertEqual(por_nombre["guest-llm"]["estado"], "omitida")

    def test_cli_forma_documentada(self):
        """`baseline.py --repo <ruta> --out <ruta>`, tal y como la fija la ficha."""
        escribir(self.repo, "nuevo.txt", "n\n")
        salida = self.destino("cli")
        p = subprocess.run(
            [sys.executable, SCRIPT, "--repo", self.repo, "--out", salida,
             "--omitir-herramientas"],
            capture_output=True, check=False,
        )
        self.assertEqual(p.returncode, 0, p.stderr.decode())
        self.assertTrue(os.path.isfile(os.path.join(salida, "baseline.json")))

        # Rutas relativas: el clone se ejecuta con otro cwd y debe seguir
        # apuntando al destino pedido.
        p = subprocess.run(
            [sys.executable, SCRIPT, "reconstruct", "--capture", "cli", "--into", "arbol"],
            cwd=self.tmp, capture_output=True, check=False,
        )
        self.assertEqual(p.returncode, 0, p.stderr.decode())
        self.assertTrue(os.path.isfile(os.path.join(self.tmp, "arbol", "nuevo.txt")))


if __name__ == "__main__":
    unittest.main()
