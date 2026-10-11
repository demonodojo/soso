# Primera tarea del agente nativo con build remoto (T37)

**Fecha:** 2026-10-11. **Ubicaciones reales:** agente = **soso** (ELF `soso-improve agente` en el guest) · inferencia = **soso** (`soso-llm serve` del mismo guest, `qwen2.5-coder-14b`) · build = **host** (`soso-forja-server`, compila `hola-std` para el target de soso).

## Tarea
Cambiar la línea que imprime `user/hola-std/src/main.rs` (`hola desde soso-std`) por `hola-t37-forja`, compilar y comprobar el binario ya aplicado. Aceptación externa: otra sesión SSH ejecuta `/bin/hola-std` y lee el fuente; no cuenta lo que diga el agente.

## Cómo se ejecuta
`cargo xtask test-agent-forja` (= `test-llm-api --agente-forja`; reintentos en `target/self-improvement/first-improvement/lanzar-guest-agente-forja.sh`).

1. El arnés levanta el servidor de Forja en el host y escribe el enunciado en el guest con `echo`.
2. `soso-improve agente --perfil forja --forja-host 10.0.2.2 --forja-token …` corre **dentro de soso**. Herramientas: `read`, `grep`, `glob`, `reemplazar_texto`, `bash` (sólo `/bin/hola-std`) y `forja` (`sync`/`build`/`all`). El **token de Forja lo pone el adaptador**: no viaja en la conversación y se enmascara si la salida lo repite. El diagnóstico del compilador vuelve al modelo como resultado de la herramienta (nadie lo copia al prompt).
3. Otra sesión comprueba `/bin/hola-std` y el fuente. Después, las dos pruebas negativas de T36: recibo de otro build → «recibo rechazado … no se escribió nada»; sin sync previo → «no hay un sync previo».

## Resultado (`tests/self-improvement/opencode/t37-agente-forja/`)
`read` → `reemplazar_texto` → `forja all` → `bash /bin/hola-std`, 5 turnos, 1213 s. `/bin/hola-std` imprime `hola-t37-forja`; el fuente está cambiado; ambos rechazos de recibo se ven. El servidor del modelo siguió operativo después (las sesiones de comprobación lo usaron).

## Qué falló antes (conservado en `…-1-ruta-mal`)
El agente pidió `src/soso/user/…` (relativa a /src/soso) y, con sólo «no existe», se rindió con una disculpa. Dos arreglos: el «no existe» de las herramientas lista ahora lo que hay en el directorio más cercano existente, y el enunciado/prompt dicen que las rutas son relativas al checkout.

## Límites
- **Etapa mixta:** el build es remoto (host). T51 exige repetir build, pruebas e informe **sin Forja**; esto no lo acredita.
- «Repetir la aceptación sobre candidato limpio» **no** se hizo: no hay una segunda reconstrucción del candidato desde cero.
- Una sola tarea, un solo intento aceptado; no es una campaña. El fallo del primer intento fue de la herramienta, no del modelo.
- El validador de T26 no interviene aquí (la aceptación es la ejecución del binario); la integración Forja↔`tarea validar` queda para T43/T51.
