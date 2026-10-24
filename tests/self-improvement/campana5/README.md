# Banco ciego 5 (R41–R50, 2026-10-23)

Diez tareas de mejora sobre la base `target/gemma-work/base0` (commit 6f558f072), de tipos mezclados: endurecimiento (3), corrección de
defectos reales (3), funcionalidad nueva (2), rendimiento (1) y ampliación de API compatible (1). Redactadas **sin mirar** los resultados de
las campañas ni el comportamiento del agente, para medir al arnés congelado (v41) y al combo Qwen3.6-35B-A3B → 27B denso, y calibrar con Claude.

Formato igual que `campana4`: `repo/Rxx.json` (especificación), `visible/Rxx/enunciado.md` (lo único que ve el agente) y
`reservado/Rxx/{aceptacion.rs,referencia.patch}` (jamás llega al agente). Cada tarea falla «tal cual» y pasa con `--con-referencia`.

## Congelación y medida
Arnés congelado: `target/gemma-work/target-v41/debug/soso-improve` (v41) y `campaign-17.json` (sin inyección, 5400 s por tarea). No se toca el
arnés ni el banco durante la medida. Revisión: las diez tareas pasan el verificador oficial (tal cual: fallo; `--con-referencia`: ok) sobre
árboles limpios (2026-10-23). Medida prevista: combo MoE → denso (`combo-moe-denso.sh`) y calibración con Claude.

## Incidencias de infraestructura durante la medida (no cambian la estrategia del agente)
- **R43 (MoE, pasada A):** `ErrorModelo` por una llamada con el JSON roto (el modelo se quedó repitiendo `0` hasta cortarse) que el arnés v41 guardó en el
  historial; llama-server la releía en cada petición y contestaba 500 sin fin. **v42** no guarda llamadas con JSON roto (sustituye los argumentos por `{}`
  y devuelve el error al modelo). Las tareas afectadas se repiten con v42 en carpeta nueva; el resto de la medida sigue con v41.
- **Claude, R46–R50:** la primera pasada cortó por el límite de sesión de la suscripción y la repetición dio «Bloqueado» por refs de base antiguas
  (`correr.sh` ya las borra): R46–R50 de Claude se repiten en carpeta nueva.

