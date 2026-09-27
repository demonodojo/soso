# T75 — Sincronizar la receta Rust con el bootstrap de libstd

**Hito:** SI-7 · **Estado:** completada (2026-09-26).
**Resumen:** [seguimiento/T75.md](seguimiento/T75.md).
**Dependencias:** [T38](T38-toolchain-inventario.md), [T50](T50-cambios-contenido.md).
**Consumidor:** [T39](T39-bootstrap-libstd.md), cuyo cierre exige esta ficha.
**Contrato:** C6 y [NATIVO.md](NATIVO.md). **Seguimiento:** [T75](seguimiento/T75.md).

## Problema comprobado

Al abrir la ficha, `pasos_libstd` reproducía una versión anterior de
`config/rust-soso/apply-patches.sh`. Esa tabla es el hueco de partida; en host
ya está cerrado (ver el resumen). La inspección del 26-sep mostraba:

| Script actual | Receta actual |
|---|---|
| Copia módulos sueltos args/env/io/random/sync | Sólo copia los dos árboles iniciales y dl.rs |
| Ramas de despacho para esos módulos y TLS | No contiene esos pasos |
| Constantes DLL/EXE y dependencia `public = true` | Conserva las versiones anteriores |
| Espejo de directorios propios con borrado | Copia sin retirar restos |
| Contenido del overlay actualizado | `comprobar_uno(CopiarArbol)` sólo mira si existe el directorio |
| Reparaciones de estados antiguos | Marcas que pueden aceptar contenido obsoleto |

Los ocho tests históricos prueban el motor, no la equivalencia con el script
actual. D7 no bloquea comprobar esa equivalencia en copias del vendor.

## Alcance

Motor/receta en `crates/soso-improve-core/src/receta.rs`, sus pruebas,
`config/rust-soso/apply-patches.sh` como referencia transitoria y adaptadores
`tools/soso-improve` / `user/soso-improve` si hacen falta operaciones portables.
Leer el árbol `config/rust-soso/tree` y la revisión fijada por T38.
No añadir un segundo motor ni dependencias shell al guest.

## Pasos

1. Inventariar cada transformación del script actual y crear dos copias
   temporales equivalentes de los archivos de la revisión fijada del vendor.
   Aplicar script a una y receta a otra; comparar archivos, ausencias y hashes.
2. Incorporar módulos, despachos, constantes y atributos ausentes. Una marca
   textual sola no acredita que una versión vieja del parche sea correcta.
3. Acreditar convergencia desde vendor limpio, parcheado antiguo y parcheado
   actual. Borrar sólo dentro de directorios propiedad completa del overlay;
   conservar módulos upstream y rechazar anclas rotas con ruta y nombre.
4. Comprobar segunda aplicación idéntica y prueba negativa por cambio de
   ancla y un directorio existente con contenido obsoleto: no puede informarse
   `YaEstaba` por su mera existencia. La comparación debe fallar antes del arreglo en alguno de los huecos
   reales de la tabla. No fijar únicamente el número de pasos.
5. Ejecutar las mismas operaciones portables en fixture guest con T49,
   incluidas retirada de un resto y preservación de un fichero ajeno. Entregar
   inventario/hash de receta a T39. Si T72 añade la rama alloc después,
   actualizar ambas rutas y repetir la comparación antes del cierre de T39.

## Comprobación y cierre

`cargo test -p soso-improve-core -p soso-improve` y
`cargo check -p soso-improve-core --no-default-features`, más la prueba de
equivalencia creada aquí y el caso guest de T49. No ejecutar cambios sobre
el vendor del usuario para probar idempotencia. El script es oráculo de
transición, no una dependencia del ejecutor nativo final.

- [x] Equivalencia de archivos y hashes sobre la revisión fijada.
- [x] Convergencia, idempotencia y ancla ausente comprobadas.
- [x] Operaciones acreditadas en guest con artefactos y exit codes.
- [x] T39 consume la receta actualizada; no se declara std compilada por ello.

Huella de la receta en este checkout (incluye la ruta absoluta de `soso-rt`):
`92fb8dfbe403ae9946c526057e53c28187eec5cdfc1d526d3156fc792db8504d`.

**Validación nativa:** pendiente, T39/T49. El build cruzado y humo de std
pertenecen a T39; no son dependencias para empezar T75 y no crean un ciclo.
