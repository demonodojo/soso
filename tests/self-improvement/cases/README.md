# Banco de casos de automejora

25 casos con aceptación automatizable, creados por la ficha
[T02](../../../docs/self-improvement/T02-banco.md). Sirven para medir al modelo
y al agente antes de dejarles tocar el sistema, y para comparar campañas entre
sí.

| Clase | Casos | Qué se juzga | Verificador |
|---|---|---|---|
| `programacion` | 10 (`P01`–`P10`) | un programa Rust de un archivo contra vectores reservados | `soso-improve verificar programa` |
| `protocolo` | 10 (`Q01`–`Q10`) | la respuesta HTTP/SSE grabada del servidor | `soso-improve verificar protocolo` |
| `repo` | 5 (`R01`–`R05`) | un árbol del proyecto contra una prueba reservada | `soso-improve verificar repo` |

## Qué ve el agente y qué no

```
visible/<id>/     enunciado y entradas: esto es lo único que el lanzador copia
reservado/<id>/   vectores, respuestas esperadas, pruebas y soluciones
```

`banco.paquete_visible(caso)` devuelve exactamente lo copiable. La raíz de lo
reservado se mueve fuera del checkout que ve el agente con `--reservado` o con
`SOSO_BANCO_RESERVADO`; los verificadores no necesitan nada más. Mientras el
banco se desarrolla vive dentro del repositorio, que es cómodo y no engaña a
nadie: un agente con acceso a este árbol puede leerlo. Antes de una campaña de
verdad hay que sacarlo.

## Partición

`desarrollo` o `reservado` no se eligen: salen del identificador.

```
sha256("soso-banco-v1|" + id)[:8] % 5 < 2  ->  reservado
```

Así la partición se rehace sin guardar listas y nadie puede mover un caso al
lado que le conviene. Los casos `reservado` no se usan mientras se itera: son
la medida final.

## Sellado

`banco.json` guarda la distribución, el conteo por partición y una **huella**
de todos los casos. Cada caso guarda además el SHA-256 de sus archivos
visibles. Una campaña anota la huella al empezar: si al terminar no es la
misma, los números no son comparables.

```sh
cargo run -q -p soso-improve -- banco listar   # los 25 casos con clase y partición
cargo run -q -p soso-improve -- banco validar  # estructura, hashes y huella
cargo run -q -p soso-improve -- banco sellar   # recalcula partición, hashes y huella
```

Dentro de soso, lo mismo con `soso-improve banco <dir> listar|validar`.

## Umbrales

`banco.json` tiene el hueco (`umbrales`) y **está vacío a propósito**: T02 no
ejecuta ningún modelo, así que cualquier número que pusiera aquí sería
inventado. Los fija [T14](../../../docs/self-improvement/T14-evaluacion-modelo.md)
con sus medidas, y desde ese momento no se tocan durante una campaña.

## Ejecutar un verificador a mano

```sh
# programación: el candidato es un archivo .rs
soso-improve verificar programa  --caso P01 --candidato solucion.rs

# protocolo: el candidato es la respuesta grabada
soso-improve verificar protocolo --caso Q01 --respuesta respuesta.json

# repo: el candidato es el árbol modificado
soso-improve verificar repo      --caso R01 --arbol /ruta/al/arbol
```

Códigos de salida iguales en los tres: `0` pasa, `2` falla, `1` el caso o el
candidato no se pueden usar (no compila, falta el archivo, el árbol no es git).
La diferencia importa: un candidato que no compila no es lo mismo que uno que
compila y responde mal.

`verificar repo` deja el árbol como lo encontró —borra la prueba que
inserta y revierte el parche si lo aplicó— y se niega a sobrescribir un archivo
que ya estuviera ahí. Con `--con-referencia` aplica la solución reservada antes
de medir: es el control que demuestra que la aceptación distingue una solución
correcta de la base, no una forma de resolver el caso.

## Pruebas

```sh
cargo test -p soso-improve-core -p soso-improve
```

Comprueban la estructura, el sellado, la separación de lo reservado y —lo que
de verdad importa— que cada verificador distingue una solución correcta de una
incorrecta. Los casos de repo necesitan cargo y un árbol del proyecto, así que
su discriminación se ejecuta aparte y queda en
`target/self-improvement/tasks/T02/`.
