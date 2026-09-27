# T74 — Recuperar y acreditar la campaña interrumpida del modelo 7B

**Hito:** SI-0 / SI-2 · **Estado:** completada (2026-09-26).
**Dependencias:** [T14](T14-evaluacion-modelo.md), [T19](T19-qemu-e2e.md).
**Contrato:** C2–C3, C5–C6 y [NATIVO.md](NATIVO.md).
**Seguimiento:** [T74](seguimiento/T74.md).

## Problema y evidencia

La comparación elegida el 25-sep ya empezó. En la campaña 7B del 26-sep,
Q01–Q04 pasan tres veces; Q05 tiene un éxito y dos EAGAIN inmediatos;
Q06–Q10 devuelven `Connection reset by peer`. El agregado cuenta 4 sólidos,
1 inestable y 13/30 intentos correctos, pero no mide la calidad de los cinco
últimos casos. `correr_campana` trata el exit de verificación como «NO-GO
medido» y el arnés publica `guest_ok`, exit 0. Se confunden una evaluación
completa desfavorable y una campaña que perdió el servicio.

Evidencia conservada: `target/self-improvement/tasks/T14/campana-t14-7b.json`,
`resultado-campana-7b.md` y [seguimiento/T14.md](seguimiento/T14.md).
El informe canónico de T19 fue sobrescrito por el 7B; la copia 3B está en
`target/self-improvement/tasks/T14/campana-t14-3b.json`.
El lock 7B sólo declara nombre y directorio: faltan revisión y hashes.

## Alcance y contexto

- `xtask/src/test_llm_api.rs`: `correr_campana`, sesión de servidor y artefactos.
- `crates/soso-improve-core/src/evaluacion.rs` y `protocolo.rs`: conservar el
  desenlace por intento y distinguir cobertura de veredicto de calidad.
- `tools/soso-improve/src/lib.rs` y adaptador guest si cambia el informe común;
  pruebas existentes de evaluación/API, perfil y documentación T14.

Inspeccionar `user/soso-llm/src/net.rs` y `crates/soso-http/src/lib.rs` sólo
si la reproducción apunta allí. Un defecto independiente de kernel/runtime
se deriva con reproducción y bloqueo; no se convierte esta ficha en un port.

## Pasos

1. Preservar los artefactos por modelo e intento, con banco, configuración,
   revisión, hashes de manifest/index/tokenizer/pesos y argv sin secretos.
   Completar el lock 7B con la herramienta de perfil existente. No cambiar C1
   ni el selector live para hacer el experimento.
2. Añadir una regresión con endpoint controlado que falle después de un caso:
   no puede producir «campaña completa / guest_ok». Distinguir transporte
   perdido de un timeout propio de un caso con servicio recuperado. Mantener
   el denominador y los intentos fallidos; nunca convertirlos en éxitos.
3. Reproducir Q05 de forma acotada y después la transición Q04→Q05; conservar
   serial, log del servidor, respuesta, errno y estado del proceso. Separar
   timeout, cancelación no recuperada, error del cliente y caída del servidor.
   No atribuir `EAGAIN` a una causa sin medirla ni subir plazos a ciegas.
4. Corregir la causa dentro del alcance, o registrar una ficha derivada que
   bloquee el cierre. Probar que una petición posterior vuelve a responder.
5. Repetir 10 casos × 3 con el mismo banco, directorio de intento exclusivo y
   recursos declarados. Q07 debe medirse. Publicar GO/NO-GO de calidad sólo
   con cobertura válida; una interrupción conserva `go=false` y causa explícita.
   Actualizar T14 sin presentar los presupuestos de la tirada rota como calibración.

## Comprobación y cierre

Pruebas host: `cargo test -p soso-improve-core -p soso-improve`, más la prueba
focalizada del arnés creada en esta ficha. Negativos: pérdida del endpoint,
tirada filtrada y segundo modelo que no sobrescribe al primero.

Reproducción registrada (un solo QEMU a la vez; hoy los puertos son fijos):

```sh
SOSO_QEMU_MEM=12G cargo xtask test-llm-api \
  --model-dir target/qwen2.5-coder-7b-model \
  --profile target/self-improvement/tasks/T14/model-lock-7b.json --campana
```

Para el diagnóstico usar `--caso Q05 --repeticiones 1`; su resultado es
parcial. La campaña completa anterior duró 4662 s: no repetirla antes de
resolver la reproducción acotada. Conservar comando, exit code y cobertura.

- [x] Identidad y artefactos por intento completos, sin sobrescrituras.
- [x] Prueba negativa distingue interrupción y NO-GO válido.
- [x] Causa diagnosticada y recuperación acreditada contra el guest real.
- [x] Campaña completa y seguimiento T14 actualizados, sea GO o NO-GO.

**Validación nativa:** parcial. La campaña 10×3 corrió contra el endpoint del
guest. El clasificador de cobertura corrió en el host; ejecutarlo dentro de
soso sigue en T49. Resumen: [seguimiento/T74.md](seguimiento/T74.md).
