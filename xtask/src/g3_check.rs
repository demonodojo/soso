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
    print_criterion("G3b hw boot (log: GSP booted sin soft)", gsp_log_ok && log_contains(&log, "GSP booted (hw"));
    print_criterion(
        "G3b nvkm ACR port (inventario stubs)",
        root.join("lxdde/ports/nouveau/nvkm_ola2.list").exists(),
    );

    println!("\n=== Fases GSP esperadas (serial) ===");
    println!("   fw_ready → fw_staged → rm_radix3");
    println!("     → [fmc_parse → fmc_ready → wpr_meta → libos_args → cot_ready → cot_sent → booted");
    println!("        | acr_load → acr_ahesasc → acr_asb → kick → poll → booted | booted_soft]");
    println!("   Inventario nvkm ola2: ./scripts/l6-g3-nvkm-inventory.sh nvkm_ola2.list");
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
