# T64 — `sosh` no entiende comillas: una ruta con espacios es inescribible

**Hito:** SI-4 · **Tipo:** Corrección de userspace (shell) · **Estado:** pendiente.

**Dependencias:** [T62](T62-argv-en-los-programas.md). **La origina:** T62, paso 4.

## Problema

Desde [T62](T62-argv-en-los-programas.md) el argv viaja entero de punta a
punta: `sosh` pasa las palabras que tokenizó, `libsoso` y el kernel no las
vuelven a juntar, y `main` las recibe separadas. Un programa puede recibir
`/tmp/con espacio.txt` como **un** argumento, y la sonda
`argv/ruta-con-espacios` lo acredita dentro de soso.

Lo que sigue sin poderse es **escribirlo desde la shell**. `tokenize`
(`user/sosh/src/main.rs`) corta en cualquier `char::is_whitespace` y no conoce
`'`, `"` ni `\`:

```
$ cat "/tmp/con espacio.txt"
cat: "/tmp/con: no existe
cat: espacio.txt": no existe
```

Las comillas ni siquiera se quitan: llegan como parte de la palabra. Para el
usuario, la diferencia con el fallo de T62 es ninguna; el sitio donde se pierde
el argumento, sí.

## Por qué no se hizo en T62

T62 cambió **cómo se transporta** un argv que ya existía. Esto es distinto:
hay que decidir una **sintaxis** —qué comillas, si hay escapes, qué pasa con
`\` dentro de comillas dobles, si `'` es literal como en POSIX— y cada opción
compromete a la shell para siempre. Mezclarlo con el transporte habría
escondido una decisión de diseño dentro de una corrección mecánica.

## Decisión pendiente

1. **Subconjunto POSIX**: `'literal'` sin escapes, `"con $nada pero con \\"
   escapes"`, `\<espacio>` fuera de comillas. Es lo que espera cualquiera.
2. **Sólo comillas dobles**, sin escapes. Más simple, y deja fuera el caso de
   una ruta con comilla dentro.

Conviene la 1 si va a haber usuarios; la 2 si sosh se queda como shell de
arranque y diagnóstico. Elegir antes de tocar el tokenizador.

## Alcance

`user/sosh/src/main.rs` (`tokenize` y los tipos de `Token`). El resto del
camino ya está: `CmdSpec.args` es `Vec<String>` y se pasa como argv.

## Pasos

1. Elegir sintaxis y dejarlo escrito.
2. Implementar en `tokenize`, quitando las comillas de la palabra resultante.
3. Comillas sin cerrar: error claro, no una palabra a medias.
4. Prueba en el shard `sys`: `cat "/tmp/t64/con espacio.txt"` desde sosh, que
   **falle** antes del cambio.
5. Revisar que las redirecciones y el pipe siguen partiéndose igual, y que una
   ruta entrecomillada tras `>` funciona.

## Comprobación

`cargo xtask test` con un paso nuevo en el shard `sys`.

## Cierre y condición de bloqueo

- [ ] Implementación terminada.
- [ ] Comprobaciones ejecutadas y evidencia guardada.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). Es la shell de soso y la prueba se escribe
desde ella, así que no hay otro sitio donde comprobarlo. Validación nativa:
**pendiente**.
