# R32 — `Sosofs::resolve` entiende `..`

## Qué hay hoy
`Sosofs::resolve(path)` (en `sosofs`, `src/fs.rs`) convierte una ruta en el número de inode.
Parte de la raíz, ignora los componentes vacíos y `.`, y busca cada componente con `lookup` en el
directorio actual. Un componente `..` no tiene tratamiento: se busca como si fuera el nombre de un
fichero llamado `..`, y como el sistema de ficheros no guarda entradas `..`, siempre devuelve
`FsError::NotFound`. Por eso una ruta como `/etc/../a/b` o `/a/b/..` no se puede resolver, y el
código que compone rutas relativas (`cd ..`, enlaces, herramientas) tiene que normalizarlas antes.

## Qué se pide
Que `resolve` interprete `..` como «subir al directorio padre» del directorio que se ha recorrido
hasta ese punto:

- `/a/b/..` resuelve al inode de `/a`; `/a/b/c/../..` también;
- la raíz es su propio padre: `/..`, `/../..` y `/a/../..` resuelven a `ROOT_INODE`, y se puede seguir
  bajando después (`/../etc` es `/etc`);
- `..` sólo tiene sentido sobre un directorio: si el componente anterior es un fichero
  (`/a/x.txt/..`), el error es `FsError::NotADir`, igual que hoy lo sería `/a/x.txt/y`;
- un componente que no existe sigue dando `NotFound` aunque después venga un `..`
  (`/nada/../a` falla);
- las rutas sin `..` se comportan exactamente como ahora (barras repetidas o finales, `.`, cadena
  vacía = raíz).

No hace falta tocar el formato en disco ni la firma pública.

## Alcance
Solo `crates/sosofs/src/fs.rs`, función `resolve`.

## Comprobación
`cargo test -p sosofs --features std`, antes y después de tu cambio.
