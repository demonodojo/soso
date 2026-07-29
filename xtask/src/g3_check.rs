//! `cargo xtask g3-check` — checklist G3 (firmware + lxdde nouveau + fases GSP).

use std::path::Path;
use std::process::exit;

pub fn run(_args: &[String]) {
    let root = super::project_root();
    let mut ok = 0u32;
    let mut warn = 0u32;
    let mut fail = 0u32;

    println!("=== L6 G3 — checklist bring-up GSP ===\n");

    println!("1) Firmware gb205 en rootfs");
    let fw_bins = [
        "bootloader-570.144.bin",
        "fmc-570.144.bin",
        "gsp-570.144.bin",
    ];
    let fw_root = root.join("rootfs/lib/firmware/nvidia/gb205/gsp");
    let mut fw_ok = 0u32;
    for name in &fw_bins {
        let p = fw_root.join(name);
        if p.is_file() {
            ok += 1;
            fw_ok += 1;
            println!("   OK    gb205/gsp/{name}");
        } else {
            fail += 1;
            println!("   FAIL  falta gb205/gsp/{name} — ./scripts/l6-pack-firmware.sh");
        }
    }

    println!("\n2) Capa lxdde nouveau");
    let lib = root.join("target/lxdde/liblxdde.a");
    if lib.exists() {
        ok += 1;
        println!("   OK    {}", lib.display());
    } else {
        warn += 1;
        println!("   WARN  falta lib — cargo xtask lx-build nouveau");
    }

    println!("\n3) Módulos G3 en source.list");
    for m in [
        "gsp_fw.c",
        "gsp_dma.c",
        "gsp_rm.c",
        "gsp_wpr.c",
        "gsp_libos.c",
        "gsp_rpc.c",
        "gsp_cmdq.c",
        "gsp_mmio.c",
        "gsp_bringup.c",
        "acr_fw.c",
        "falcon_lx.c",
        "acr_lx.c",
        "fmc_lx.c",
        "fsp_lx.c",
    ] {
        let p = root.join("lxdde/ports/nouveau").join(m);
        if p.exists() {
            ok += 1;
            println!("   OK    {m}");
        } else {
            fail += 1;
            println!("   FAIL  falta {m}");
        }
    }

    println!("\n4) Firmware ACR ga102 (ola 2)");
    let acr_bins = ["ucode_ahesasc.bin", "ucode_asb.bin"];
    let acr_root = root.join("rootfs/lib/firmware/nvidia/ga102/acr");
    let mut acr_ok = 0u32;
    for name in &acr_bins {
        let p = acr_root.join(name);
        if p.is_file() {
            ok += 1;
            acr_ok += 1;
            println!("   OK    ga102/acr/{name}");
        } else {
            warn += 1;
            println!("   WARN  falta ga102/acr/{name} — ./scripts/l6-pack-firmware.sh");
        }
    }

    println!("\n5) Log GSP en QEMU/VFIO (opcional)");
    let log = root.join("target/g1-vfio-serial.log");
    let gsp_log_ok = check_gsp_log(&log);
    if gsp_log_ok {
        ok += 1;
        println!("   OK    fases GSP en {}", log.display());
        for line in std::fs::read_to_string(&log).unwrap_or_default().lines() {
            if line.contains("nouveau-lx:") {
                println!("         {line}");
            }
        }
    } else {
        warn += 1;
        println!("   WARN  sin log GSP — cerrar G1 y SOSO_QEMU_GPU=vfio:… cargo xtask run");
    }

    println!("\n=== Criterios G3 ===");
    print_criterion("G3a firmware + GEM staging (build)", fw_ok == 3 && lib.exists());
    print_criterion(
        "G3 ola2 ACR lx (firmware + módulos)",
        acr_ok == 2 && root.join("lxdde/ports/nouveau/acr_lx.c").exists(),
    );
    print_criterion(
        "G3b radix3 GSP-RM verificada (log)",
        log_contains(&log, "radix3 verificada"),
    );
    print_criterion(
        "G3b WPR meta verificado (log)",
        log_contains(&log, "WPR meta verificado"),
    );
    print_criterion(
        "G3b libos boot args + COT listo (log)",
        log_contains(&log, "libos verificado") && log_contains(&log, "COT listo"),
    );
    print_criterion(
        "G3b COT aceptado por el FSP (log)",
        log_contains(&log, "COT aceptado por el FSP"),
    );
    // Con la GPU caída del bus todo el MMIO se lee 0xffffffff, y el poll de tu102
    // lo tomaba por "listo": el 2026-07-25 esto dio un GO con la tarjeta muerta.
    // Se detecta por el mensaje nuevo y, además, por la huella en crudo: un
    // registro leído como 0xffffffff no es un registro, es silencio del bus.
    let gpu_gone = log_contains(&log, "fuera del bus")
        || log_contains(&log, "118128=0xffffffff")
        || log_contains(&log, "boot0=0xffffffff");
    if gpu_gone {
        println!("   AVISO la GPU se cayó del bus en esta ejecución — el arranque no cuenta");
    }
    print_criterion(
        "G3b hw boot (log: GSP booted sin soft)",
        gsp_log_ok && log_contains(&log, "GSP booted (hw") && !gpu_gone,
    );
    // El arranque del firmware de la GPU. En gb20x el indicador NO es el scratch de
    // la isla GC6 de Turing (que se lee 0 siempre) sino
    // `NV_THERM_I2CS_SCRATCH_FSP_BOOT_COMPLETE`, y es el que exige
    // `fsp_ready_to_send()` antes del COT. Confundirlos costó un ciclo el 2026-07-29.
    print_criterion(
        "firmware de la GPU arrancado (log: FSP boot complete = 0xff)",
        log_contains(&log, "FSP secure boot=0x000000ff") && !gpu_gone,
    );
    if log_contains(&log, "GFW_BOOT_PROGRESS") {
        println!(
            "   NOTA  GSP-RM asertó sobre GFW_BOOT_PROGRESS en un NOCAT. En gb20x ese\n\
             \x20        scratch (isla GC6, de Turing) se lee 0 y NO es el indicador de\n\
             \x20        este chip: el bueno es el FSP boot complete de arriba."
        );
    }
    // PTOP: la topología la publica el chip. No es un gate, es lo que evita seguir
    // adivinando índices de la tabla del FIFO de RM (que en GB205 dieron 0xbadf5040).
    print_criterion(
        "PTOP: topología de motores leída del chip (log)",
        log_contains(&log, "PTOP: ") && !gpu_gone,
    );
    print_criterion(
        "G4a SET_SYSTEM_INFO/SET_REGISTRY encolados (log)",
        log_contains(&log, "SET_SYSTEM_INFO") && log_contains(&log, "SET_REGISTRY"),
    );
    print_criterion(
        "G4a GSP_INIT_DONE por RPC (log)",
        log_contains(&log, "GSP_INIT_DONE recibido") && !gpu_gone,
    );
    print_criterion(
        "G4c objetos de RM cliente→device→subdevice (log)",
        log_contains(&log, "objetos RM listos") && !gpu_gone,
    );
    print_criterion(
        "G4d static info: VRAM utilizable de RM (log)",
        log_contains(&log, "GSP static info:") && !gpu_gone,
    );
    // Dos criterios distintos a propósito. Que RM acepte el directorio dice que
    // la raíz y el vaspace le cuadran; que la traducción se relea de las tablas
    // dice que las entradas están donde tienen que estar. Ninguno de los dos es
    // "la GPU traduce" — eso solo lo prueba el CE moviendo bytes, en G4e.
    print_criterion(
        "G4d vaspace + directorio aceptado por RM (log)",
        log_contains(&log, "vaspace 0x") && !gpu_gone,
    );
    print_criterion(
        "G4d traducción releída de las tablas (log)",
        log_contains(&log, "traducción verificada") && !gpu_gone,
    );
    // Qué clases dice tener el chip. No es un gate: es el dato que decide qué
    // clase de canal, de CE y de compute se piden, y hasta el 2026-07-28 se
    // suponía (mal: se pedían las de Ampere en una Blackwell).
    print_criterion(
        "G4e catálogo de clases del chip (log)",
        log_contains(&log, "catálogo de clases:") && !gpu_gone,
    );
    // G4e es el primer criterio que no se puede satisfacer leyendo registros:
    // exige que la GPU haya movido bytes por nuestras tablas de páginas. El
    // "canal armado" va aparte justamente para que no se confunda con el GO.
    print_criterion(
        "G4e canal GPFIFO + CE armados (log)",
        log_contains(&log, "CE listo cls=") && !gpu_gone,
    );
    // El doorbell es una escritura ciega: si el aperture de usermode no está donde
    // creemos, el trabajo se encola y nadie lo recoge, que es indistinguible de un
    // canal que no arranca. Lo único legible de ahí es su reloj.
    print_criterion(
        "G4e aperture de usermode responde (log: su reloj avanza)",
        log_contains(&log, "aperture de usermode vivo") && !gpu_gone,
    );
    print_criterion(
        "G4e GO: readback sysmem→VRAM→sysmem (log)",
        log_contains(&log, "CE readback verificado (G4e GO)") && !gpu_gone,
    );
    // Sin contexto promocionado el canal de GR existe y el QMD no puede correr, así
    // que esto va antes del criterio del SASS.
    print_criterion(
        "G4f contexto de GR promocionado (log)",
        log_contains(&log, "contexto de GR promocionado") && !gpu_gone,
    );
    print_criterion(
        "G4f SASS en VRAM + compute armado (log)",
        log_contains(&log, "SASS de ") && log_contains(&log, "compute listo cls=")
            && !gpu_gone,
    );
    print_criterion(
        "G3b nvkm ACR port (inventario stubs)",
        root.join("lxdde/ports/nouveau/nvkm_ola2.list").exists(),
    );

    println!("\n=== Fases GSP esperadas (serial) ===");
    println!("   fw_ready → fw_staged → rm_radix3");
    println!("     → [fmc_parse → fmc_ready → wpr_meta → libos_args → cot_ready → cot_sent → booted");
    println!("        | acr_load → acr_ahesasc → acr_asb → kick → poll → booted | booted_soft]");
    println!("   Inventario nvkm ola2: ./scripts/l6-g3-nvkm-inventory.sh nvkm_ola2.list");
    println!("   Tras booted: rm_ready si GSP-RM manda GSP_INIT_DONE por la cola");
    println!("   Pasos 3-6 sin GPU:    ./scripts/l6-g3-gsp-hostcheck.sh");

    println!("\n=== Resumen: {ok} OK, {warn} WARN, {fail} FAIL ===");
    if fail > 0 {
        exit(1);
    }
}

fn print_criterion(label: &str, pass: bool) {
    if pass {
        println!("   GO    {label}");
    } else {
        println!("   PEND  {label}");
    }
}

fn check_gsp_log(log: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(log) else {
        return false;
    };
    text.contains("nouveau-lx:") && text.contains("fw ")
}

fn log_contains(log: &Path, needle: &str) -> bool {
    std::fs::read_to_string(log)
        .map(|t| t.contains(needle))
        .unwrap_or(false)
}
