#!/usr/bin/env python3
"""Pruebas del banco de casos (ficha T02).

Comprueban lo que hace útil a un banco: que estén los 25 casos declarados, que
sus identificadores y particiones no dependan de nadie, que el material
reservado no viaje con lo visible y —lo que de verdad importa— que cada
verificador **distingue** una solución correcta de una incorrecta.

Los casos de programación se compilan de verdad con `rustc`; los de protocolo
se juzgan sobre documentos de respuesta grabados. Los de repo necesitan un
árbol del proyecto y una compilación de cargo, así que aquí solo se comprueba
su estructura: su discriminación se ejecuta como paso explícito con
`verificar_repo.py` y queda en la evidencia de la ficha.
"""
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

RAIZ = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CASOS = os.path.join(RAIZ, "tests", "self-improvement", "cases")
VERIFICADORES = os.path.join(CASOS, "verificadores")


def cargar(nombre, ruta):
    spec = importlib.util.spec_from_file_location(nombre, ruta)
    modulo = importlib.util.module_from_spec(spec)
    sys.modules[nombre] = modulo
    spec.loader.exec_module(modulo)
    return modulo


banco = cargar("banco", os.path.join(CASOS, "banco.py"))
ver_programa = cargar("verificar_programa", os.path.join(VERIFICADORES, "verificar_programa.py"))
ver_protocolo = cargar("verificar_protocolo", os.path.join(VERIFICADORES, "verificar_protocolo.py"))
ver_repo = cargar("verificar_repo", os.path.join(VERIFICADORES, "verificar_repo.py"))

CASOS_CARGADOS = banco.cargar_casos()


def de_clase(clase):
    return [c for c in CASOS_CARGADOS if c["clase"] == clase]


def reservado(caso_id, nombre):
    return os.path.join(banco.raiz_reservada(), caso_id, nombre)


def hay_rustc():
    return shutil.which("rustc") is not None


class TestEstructura(unittest.TestCase):
    def test_distribucion_10_10_5(self):
        self.assertEqual(len(de_clase("programacion")), 10)
        self.assertEqual(len(de_clase("protocolo")), 10)
        self.assertEqual(len(de_clase("repo")), 5)
        self.assertEqual(len(CASOS_CARGADOS), 25)

    def test_identificadores_unicos(self):
        ids = [c["id"] for c in CASOS_CARGADOS]
        self.assertEqual(len(ids), len(set(ids)))
        self.assertEqual(ids, sorted(ids))

    def test_cada_caso_cumple_el_esquema(self):
        for c in CASOS_CARGADOS:
            with self.subTest(caso=c["id"]):
                limpio = {k: v for k, v in c.items() if not k.startswith("_")}
                self.assertEqual(banco.validar_esquema(limpio), [])

    def test_el_banco_no_tiene_problemas(self):
        self.assertEqual(banco.comprobar(CASOS_CARGADOS), [])

    def test_la_particion_sale_de_la_regla(self):
        for c in CASOS_CARGADOS:
            with self.subTest(caso=c["id"]):
                self.assertEqual(c["particion"], banco.particion_de(c["id"]))
        # Y la regla reparte de verdad: ninguna clase se queda sin reservados.
        for clase in banco.CLASES:
            reservados = [c for c in de_clase(clase) if c["particion"] == "reservado"]
            self.assertGreater(len(reservados), 0, f"clase {clase} sin casos reservados")
            self.assertLess(len(reservados), len(de_clase(clase)))

    def test_la_huella_del_manifiesto_corresponde(self):
        with open(os.path.join(CASOS, "banco.json"), encoding="utf-8") as f:
            manifiesto = json.load(f)
        self.assertEqual(manifiesto["huella"], banco.huella(CASOS_CARGADOS))
        self.assertEqual(manifiesto["distribucion"], banco.DISTRIBUCION)
        # Los umbrales aún no existen: T14 los fija con medidas, no esta ficha.
        self.assertEqual(manifiesto["umbrales"]["estado"], "no fijados")

    def test_los_hashes_de_entrada_sellan_lo_visible(self):
        for c in CASOS_CARGADOS:
            with self.subTest(caso=c["id"]):
                self.assertEqual(c["hashes_entrada"], banco.hashes_calculados(c))
                self.assertTrue(c["hashes_entrada"])


class TestSeparacionDeLoReservado(unittest.TestCase):
    def test_el_paquete_visible_no_contiene_material_reservado(self):
        for c in CASOS_CARGADOS:
            with self.subTest(caso=c["id"]):
                rutas = banco.paquete_visible(c)
                self.assertTrue(rutas)
                for r in rutas:
                    self.assertTrue(r.startswith("visible/"), r)
                    self.assertTrue(os.path.isfile(os.path.join(CASOS, r)))
                for r in c["reservado"]:
                    self.assertNotIn(r, rutas)

    def test_lo_visible_no_filtra_la_solucion(self):
        """Ningún enunciado incluye el texto de su solución reservada."""
        for c in CASOS_CARGADOS:
            with self.subTest(caso=c["id"]):
                visible = ""
                for r in banco.paquete_visible(c):
                    with open(os.path.join(CASOS, r), encoding="utf-8") as f:
                        visible += f.read()
                for r in c["reservado"]:
                    nombre = os.path.basename(r)
                    if nombre not in ("referencia.rs", "referencia.patch"):
                        continue
                    with open(reservado(c["id"], nombre), encoding="utf-8") as f:
                        solucion = f.read()
                    lineas = [ln.strip() for ln in solucion.splitlines()
                              if len(ln.strip()) > 25 and not ln.strip().startswith(("//", "-", "+++", "---", "@@", "#"))]
                    for ln in lineas:
                        self.assertNotIn(ln, visible,
                                         f"{c['id']}: el enunciado repite una línea de la solución")

    def test_la_raiz_reservada_se_puede_mover_fuera(self):
        """Una campaña real guarda lo reservado fuera del checkout del agente."""
        with tempfile.TemporaryDirectory(prefix="banco-reservado-") as tmp:
            destino = os.path.join(tmp, "oculto")
            shutil.copytree(os.path.join(CASOS, "reservado"), destino)
            self.assertEqual(banco.raiz_reservada(destino), destino)
            codigo, informe = ver_protocolo.verificar(
                "Q01", os.path.join(destino, "Q01", "correcta.json"), reservado=destino)
            self.assertEqual(codigo, 0, informe)
        # Y por variable de entorno, que es lo que usará el lanzador.
        previo = os.environ.get(banco.ENV_RESERVADO)
        os.environ[banco.ENV_RESERVADO] = "/ruta/inventada"
        try:
            self.assertEqual(banco.raiz_reservada(), "/ruta/inventada")
        finally:
            if previo is None:
                os.environ.pop(banco.ENV_RESERVADO, None)
            else:
                os.environ[banco.ENV_RESERVADO] = previo

    def test_el_argv_del_comprobador_no_nombra_lo_reservado(self):
        for c in CASOS_CARGADOS:
            with self.subTest(caso=c["id"]):
                argv = c["comprobador"]["argv"]
                self.assertTrue(os.path.isfile(os.path.join(RAIZ, argv[1])),
                                f"el verificador {argv[1]} no existe")
                for arg in argv:
                    self.assertFalse(arg.startswith("reservado/"))


@unittest.skipUnless(hay_rustc(), "hace falta rustc para compilar los candidatos")
class TestCasosDeProgramacion(unittest.TestCase):
    def test_la_referencia_pasa_todos_los_vectores(self):
        for c in de_clase("programacion"):
            with self.subTest(caso=c["id"]):
                codigo, informe = ver_programa.verificar(
                    c["id"], reservado(c["id"], "referencia.rs"))
                self.assertEqual(codigo, 0, informe)
                self.assertEqual(informe["pasados"], informe["total"])
                self.assertGreaterEqual(informe["total"], 2,
                                        "un caso con un solo vector no mide nada")

    def test_la_solucion_incorrecta_se_detecta(self):
        for c in de_clase("programacion"):
            with self.subTest(caso=c["id"]):
                codigo, informe = ver_programa.verificar(
                    c["id"], reservado(c["id"], "incorrecta.rs"))
                self.assertEqual(codigo, 2, informe)
                self.assertEqual(informe["estado"], "fallo")
                self.assertGreater(informe["total"] - informe["pasados"], 0)

    def test_un_candidato_que_no_compila_es_error_no_fallo(self):
        with tempfile.TemporaryDirectory() as tmp:
            malo = os.path.join(tmp, "malo.rs")
            with open(malo, "w", encoding="utf-8") as f:
                f.write("fn main() { esto no es rust }\n")
            codigo, informe = ver_programa.verificar("P01", malo)
        self.assertEqual(codigo, 1)
        self.assertEqual(informe["estado"], "error")
        self.assertIn("compila", informe["motivo"])

    def test_un_candidato_ausente_se_informa(self):
        codigo, informe = ver_programa.verificar("P01", "/no/existe/candidato.rs")
        self.assertEqual(codigo, 1)
        self.assertEqual(informe["estado"], "error")


class TestCasosDeProtocolo(unittest.TestCase):
    def test_la_respuesta_correcta_cumple(self):
        for c in de_clase("protocolo"):
            with self.subTest(caso=c["id"]):
                codigo, informe = ver_protocolo.verificar(
                    c["id"], reservado(c["id"], "correcta.json"))
                self.assertEqual(codigo, 0, informe)
                self.assertEqual(informe["pasadas"], informe["total"])
                self.assertGreaterEqual(informe["total"], 3)

    def test_la_respuesta_incorrecta_se_detecta(self):
        for c in de_clase("protocolo"):
            with self.subTest(caso=c["id"]):
                codigo, informe = ver_protocolo.verificar(
                    c["id"], reservado(c["id"], "incorrecta.json"))
                self.assertEqual(codigo, 2, informe)
                self.assertGreater(informe["total"] - informe["pasadas"], 0)

    def test_las_peticiones_visibles_son_json(self):
        for c in de_clase("protocolo"):
            with self.subTest(caso=c["id"]):
                ruta = os.path.join(CASOS, "visible", c["id"], "peticion.json")
                with open(ruta, encoding="utf-8") as f:
                    peticion = json.load(f)
                self.assertIn("messages", peticion)
                self.assertIn("model", peticion)

    def test_el_unicode_partido_entre_trozos_se_reensambla(self):
        """El corte de Q06 cae dentro de un carácter: hay que juntarlo antes."""
        with open(reservado("Q06", "correcta.json"), encoding="utf-8") as f:
            crudo = json.load(f)
        self.assertGreater(len(crudo["trozos_b64"]), 1)
        import base64
        trozos = [base64.b64decode(t) for t in crudo["trozos_b64"]]
        with self.assertRaises(UnicodeDecodeError,
                               msg="el corte debería partir un carácter multibyte"):
            trozos[0].decode("utf-8")
        doc = ver_protocolo.Documento(crudo)
        self.assertEqual(doc.errores_formato, [])
        self.assertIn("☕", doc.mensaje()["content"])

    def test_un_documento_ilegible_es_error_no_fallo(self):
        with tempfile.TemporaryDirectory() as tmp:
            malo = os.path.join(tmp, "malo.json")
            with open(malo, "w", encoding="utf-8") as f:
                f.write("{esto no es json")
            codigo, informe = ver_protocolo.verificar("Q01", malo)
        self.assertEqual(codigo, 1)
        self.assertEqual(informe["estado"], "error")


class TestCasosDeRepo(unittest.TestCase):
    def test_cada_caso_declara_base_rutas_y_aceptacion(self):
        for c in de_clase("repo"):
            with self.subTest(caso=c["id"]):
                self.assertEqual(len(c["base"]["commit"]), 40)
                self.assertTrue(c["rutas_editables"])
                for r in c["rutas_editables"]:
                    self.assertFalse(r.startswith("/"))
                    self.assertFalse(r.startswith("tests/self-improvement"))
                aceptacion = c["aceptacion"]
                self.assertEqual(aceptacion["argv"][0], "cargo")
                self.assertTrue(aceptacion["destino_prueba"].endswith(".rs"))
                self.assertIn(c["id"], aceptacion["destino_prueba"])
                self.assertIn(c["origen"], ("reproduccion", "mejora_especificada"))

    def test_la_prueba_y_el_parche_reservados_existen(self):
        for c in de_clase("repo"):
            with self.subTest(caso=c["id"]):
                self.assertTrue(os.path.isfile(reservado(c["id"], "aceptacion.rs")))
                parche = reservado(c["id"], "referencia.patch")
                self.assertTrue(os.path.isfile(parche))
                with open(parche, encoding="utf-8") as f:
                    texto = f.read()
                self.assertTrue(texto.startswith("diff --git"))
                # El parche solo toca rutas que el caso declara editables.
                tocadas = {ln.split(" b/")[-1].strip()
                           for ln in texto.splitlines() if ln.startswith("diff --git")}
                self.assertEqual(tocadas, set(c["rutas_editables"]))

    def test_el_verificador_no_sobrescribe_una_prueba_existente(self):
        caso = de_clase("repo")[0]
        with tempfile.TemporaryDirectory() as tmp:
            os.makedirs(os.path.join(tmp, ".git"))
            destino = os.path.join(tmp, caso["aceptacion"]["destino_prueba"])
            os.makedirs(os.path.dirname(destino), exist_ok=True)
            with open(destino, "w", encoding="utf-8") as f:
                f.write("// algo que ya estaba\n")
            codigo, informe = ver_repo.verificar(caso["id"], tmp)
            self.assertEqual(codigo, 1)
            self.assertIn("ya existe", informe["motivo"])
            with open(destino, encoding="utf-8") as f:
                self.assertEqual(f.read(), "// algo que ya estaba\n")

    def test_un_arbol_que_no_es_git_se_rechaza(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(banco.ErrorBanco):
                ver_repo.verificar(de_clase("repo")[0]["id"], tmp)


if __name__ == "__main__":
    unittest.main()
