# HP: diagnóstico del flush 34 — 2026-11-15

## Evidencia y alcance

ESP `/dev/sda1`, desmontada tras la lectura. Kernel `soso 0.3.28 (ee099449d-dirty)`; el contenido del dirty no se conoce. Flush 34, uptime 901384 ms. PCI WiFi 8086:24fd, xHCI 8086:9d2f, Ethernet 10ec:8168, gráfica 8086:5917. Referencia local: `lxdde/linux/` Linux 6.6.32 (Makefile); árbol fuente extraído, sin identidad de commit independiente verificada. Copias en `target/usb-diagnostic-2026-11-15/`. Sin pruebas de radio ni USB físico posteriores a la corrección.

## Etapas

| Etapa | Evidencia | Resultado |
|---|---|---|
| Arranque | `boot: task`, `sosh —` | userspace alcanzado |
| WiFi | `INIT ucode ALIVE ok`, `UCODE_ALIVE_NTFY` RT | ALIVE alcanzado |
| WiFi init | `timeout INIT_COMPLETE_NOTIF (8000)`, `FW_ASSERT_UMAC_FATAL` | fallo, también en dos recuperaciones |
| Scan | `scan sin INIT_COMPLETE`, `wifi scan: error de E/S` | fallido, no acredita ausencia de redes |
| Ethernet | `phystatus 0x84 ... enlace DOWN` | enlace no disponible; sin diagnóstico de defecto |
| USB | `command completion huérfano (ptr=0x0, esperado 0x20d27050) code=1 ... slot=2`, timeout | enumeración del puerto 6 fallida; USB de arranque funcionó |
| OTA | `SOSOKRN.MET repuesto` | reposición observada, no valida una actualización |

Hostchecks `l6-iwl-fw-hostcheck.sh` y `l6-g3-gsp-hostcheck.sh`: ambos exit 0; salidas junto al log. No prueban DMA, transporte en placa ni scan real. El banco WiFi actual verifica PHY_CFG en RT y no detecta la secuencia INIT ausente. GSP no aplica a esta GPU Intel.

## Hallazgos

1. **Secuencia 8265 incompleta, discrepancia confirmada y compatible con el síntoma; causa exacta del assert no demostrada.** `lxdde/ports/iwlwifi/iwl_trans_8000.c:860` pasa de ALIVE INIT directamente al reinicio y carga RT; `iwl_mvm_init.c:53` envía TX_ANT y PHY_CFG con calibración REGULAR y espera INIT_COMPLETE en RT. Linux `lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/fw.c:688` lee NVM, envía TX_ANT y PHY_CFG a INIT, guarda `CALIB_RES_NOTIF_PHY_DB` y espera INIT_COMPLETE; `fw.c:1470` reinicia a RT y `fw.c:1550` entrega la PHY DB. Falta una etapa entera de NVM y calibración, no solo un cambio de ID. El log muestra ALIVE seguido de timeout y assert, sin demostrar por sí solo qué comando causó el assert.
2. **Timeout de comando xHCI confirmado, origen no establecido.** `crates/xhci-nostd/src/driver.rs:742` rechaza el completion con puntero cero y termina tras 5 s. Linux `lxdde/linux/drivers/usb/host/xhci-ring.c:1686` también exige igualdad con el TRB esperado, por lo que aceptar ese evento sería injustificado. Investigar por qué se leyó cero y recuperación del anillo; no se demuestra que el evento corresponda al comando esperado ni que sea un fallo de hardware. El dispositivo del puerto 6 no llegó a identificarse.

## Orden de corrección

1. En `lxdde/ports/iwlwifi/iwl_trans_8000.c`, `iwl_mvm_init.c`, recepción RX y gestión NVM/PHY DB, ejecutar la inicialización y calibración durante INIT, recoger PHY DB y transferirla tras cargar RT, siguiendo `mvm/fw.c:688` y `:1470`. Añadir banco que compruebe orden y datos INIT→RT y errores; en placa exigir INIT_COMPLETE real, scan completo, asociación y lease WiFi, y ausencia de assert tras recuperación.
2. En `crates/xhci-nostd/src/driver.rs` y `ring.rs`, instrumentar y corregir la lectura/gestión del completion inválido y la recuperación de timeout según los resultados, manteniendo la comprobación de identidad que también hace Linux `xhci-ring.c:1686`. Banco con puntero cero, completion tardío y timeout; en placa verificar enumeración repetida del puerto 6 y que el almacenamiento live siga funcionando. La causa concreta aún requiere evidencia.

## Qué no se ha hecho

No se modificaron drivers ni se ejecutó un ciclo de placa con correcciones. No se flasheó el USB.

## Seguimiento de la corrección 8265

En el árbol de trabajo se ha implementado la lectura NVM por `NVM_ACCESS_CMD` durante INIT, la recogida de `CALIB_RES_NOTIF_PHY_DB` y el envío de las secciones a RT antes de su `PHY_CFG`. El hostcheck incluye respuestas NVM válidas, truncadas y con offset incorrecto, ausencia de PHY SKU, errores de calibración y limpieza en recuperación. `cargo xtask lx-build iwlwifi` y `SOSO_LXDDE=1 SOSO_LXDDE_MODE=iwlwifi cargo xtask build` compilaron; el hostcheck pasó con ASan/UBSan. Estos resultados no acreditan funcionamiento en el hardware.

Pendiente: arrancar en el HP con kernel y rootfs coherentes y comprobar NVM, `INIT_COMPLETE_NOTIF` de INIT, PHY DB, RT ALIVE, scan completo, asociación, lease por LxWifi y recuperación sin assert. El fallo xHCI del puerto 6 permanece pendiente e independiente.

La revisión posterior detectó otra etapa necesaria: el ucode 8265-36 declara 159744 bytes de paginación (`IWL_UCODE_TLV_PAGING`). Se ha añadido `FW_PAGING_BLOCK_CMD` tras RT ALIVE con CSS y cinco bloques DMA. El hostcheck usa el ucode real, comprueba contenido, flags, páginas del último bloque y rechazo de imagen truncada; no sustituye la prueba en placa.

La revisión de recuperación también conserva el ucode ya parseado y reutiliza el buffer DMA de carga FH, evitando que cada reintento duplique `CMD_VERSIONS` y reserve de nuevo ambos recursos. El límite de lectura NVM corresponde a los 32 KiB de `cfg/8000.c`; el banco comprueba una sección de 9000 bytes.

La secuencia HCMD identifica ahora el índice completo del TFD (8 bits), igual que `pcie/tx.c`, en vez de reducirlo a las 32 ranuras de memoria. La NVM puede requerir más de 32 comandos. El banco verifica 300 envíos, el wrap a 256 y que una respuesta de hace 32 comandos no libere la ranura actual.
