//! `cargo xtask g1-check` — checklist host del gate L6 G1 (GPU NVIDIA / VFIO).
//!
//! Solo lectura salvo `--vfio-hint`, que imprime los comandos para bind manual.

use std::fs;
use std::path::Path;
use std::process::{Command, exit};

pub fn run(args: &[String]) {
    let show_hint = args.iter().any(|a| a == "--vfio-hint");
    let mut ok = 0u32;
    let mut warn = 0u32;
    let mut fail = 0u32;

    println!("=== L6 G1 — checklist host ===\n");

    // 1) PCI NVIDIA
    println!("1) lspci NVIDIA");
    let lspci = run_capture("lspci", &["-nn"]);
    let nvidia_lines: Vec<&str> = lspci
        .lines()
        .filter(|l| l.contains("NVIDIA") || l.contains("[10de:"))
        .collect();
    if nvidia_lines.is_empty() {
        fail += 1;
        println!("   FAIL  sin GPU NVIDIA en PCI");
    } else {
        ok += 1;
        for l in &nvidia_lines {
            println!("   OK    {l}");
        }
    }

    // 2) VT-d / DMAR + IOMMU + VFIO
    println!("\n2) VT-d / IOMMU / VFIO");
    let dmar = Path::new("/sys/firmware/acpi/tables/DMAR").exists();
    if dmar {
        ok += 1;
        println!("   OK    tabla DMAR presente (VT-d en firmware)");
    } else {
        fail += 1;
        println!("   FAIL  sin tabla DMAR — activar VT-d en BIOS antes de IOMMU");
        println!("         MSI Vector 16 HX: Advanced → Integrated Peripherals → VT-d → Enabled");
    }

    let cmdline = fs::read_to_string("/proc/cmdline").unwrap_or_default();
    let iommu_cmd = cmdline.contains("intel_iommu=on") || cmdline.contains("amd_iommu=on");
    if iommu_cmd {
        ok += 1;
        println!("   OK    IOMMU en cmdline del kernel");
    } else {
        warn += 1;
        println!("   WARN  falta intel_iommu=on — sudo ./scripts/l6-g1-enable-iommu.sh && reboot");
    }

    let iommu_groups = fs::read_dir("/sys/kernel/iommu_groups")
        .map(|d| d.count())
        .unwrap_or(0);
    if iommu_groups == 0 {
        fail += 1;
        println!("   FAIL  0 grupos IOMMU — activar VT-d/AMD-Vi en BIOS y añadir intel_iommu=on iommu=pt (o amd_iommu=on) al kernel");
    } else {
        ok += 1;
        println!("   OK    {iommu_groups} grupos IOMMU");
    }

    let vfio_mod = Path::new("/sys/module/vfio_pci").exists()
        || Path::new("/sys/module/vfio").exists();
    if vfio_mod {
        ok += 1;
        println!("   OK    módulo vfio cargado");
    } else {
        warn += 1;
        println!("   WARN  vfio no cargado (sudo modprobe vfio-pci)");
    }

    for line in &nvidia_lines {
        if let Some(bdf) = line.split_whitespace().next() {
            let sysfs = format!("/sys/bus/pci/devices/0000:{bdf}/driver");
            if let Ok(link) = fs::read_link(&sysfs) {
                let drv = link.file_name().and_then(|s| s.to_str()).unwrap_or("?");
                if drv == "vfio-pci" {
                    ok += 1;
                    println!("   OK    {bdf} → vfio-pci");
                } else {
                    warn += 1;
                    println!("   WARN  {bdf} → {drv} (passthrough requiere bind a vfio-pci)");
                }
            }
        }
    }

    if Path::new("/dev/vfio/vfio").exists() {
        ok += 1;
        println!("   OK    /dev/vfio/vfio presente");
    } else {
        warn += 1;
        println!("   WARN  /dev/vfio/vfio ausente");
    }

    // 3) Firmware GSP (linux-firmware)
    println!("\n3) Firmware GSP (linux-firmware)");
    let fw_root = Path::new("/lib/firmware/nvidia");
    if !fw_root.is_dir() {
        fail += 1;
        println!("   FAIL  /lib/firmware/nvidia no existe");
    } else {
        let blobs = collect_gsp_blobs(fw_root);
        if blobs.is_empty() {
            fail += 1;
            println!("   FAIL  sin blobs GSP bajo /lib/firmware/nvidia");
        } else {
            ok += 1;
            println!("   OK    {} blobs GSP", blobs.len());
            for b in blobs.iter().take(12) {
                println!("         {b}");
            }
            if blobs.len() > 12 {
                println!("         … (+{} más)", blobs.len() - 12);
            }
        }
    }

    // 3b) Firmware descomprimido en rootfs (G2)
    println!("\n3b) Firmware GSP en rootfs (soso)");
    let rootfs_fw = super::project_root().join("rootfs/lib/firmware/nvidia/gb205/gsp");
    let bins = [
        "bootloader-570.144.bin",
        "fmc-570.144.bin",
        "gsp-570.144.bin",
    ];
    let mut rootfs_ok = 0u32;
    for name in &bins {
        let p = rootfs_fw.join(name);
        if p.is_file() {
            ok += 1;
            rootfs_ok += 1;
            println!("   OK    rootfs/.../gb205/gsp/{name}");
        } else {
            warn += 1;
            println!("   WARN  falta rootfs/.../gb205/gsp/{name} — ./scripts/l6-pack-firmware.sh");
        }
    }

    // 4) nouveau / GSP en dmesg (solo informativo)
    println!("\n4) GSP en dmesg (nouveau, informativo)");
    let dmesg = run_capture("dmesg", &[]);
    let gsp_lines: Vec<&str> = dmesg
        .lines()
        .filter(|l| {
            let lower = l.to_ascii_lowercase();
            lower.contains("gsp") && (lower.contains("nvidia") || lower.contains("nouveau"))
        })
        .collect();
    if gsp_lines.is_empty() {
        warn += 1;
        println!("   WARN  sin líneas GSP en dmesg (normal si solo corre el driver propietario nvidia)");
    } else {
        ok += 1;
        for l in gsp_lines.iter().take(8) {
            println!("   OK    {l}");
        }
    }

    // 5) lx-build nouveau
    println!("\n5) Capa lxdde (nouveau stub)");
    let lib = super::project_root().join("target/lxdde/liblxdde.a");
    if lib.exists() {
        ok += 1;
        println!("   OK    {}", lib.display());
    } else {
        warn += 1;
        println!("   WARN  falta {} — ejecutar: cargo xtask lx-build nouveau", lib.display());
    }

    // Resumen go/no-go
    println!("\n=== Criterios go/no-go (automático parcial) ===");
    let fw_ok = fw_root.is_dir() && !collect_gsp_blobs(fw_root).is_empty();
    print_criterion("Tabla DMAR / VT-d en BIOS", dmar);
    print_criterion("Firmware GSP redistribuible (linux-firmware)", fw_ok);
    print_criterion("Firmware gb205 en rootfs (G2: boot + 2×ELF)", rootfs_ok == 3);
    print_criterion("IOMMU activo (prerrequisito VFIO)", iommu_groups > 0);
    let boot0_ok = check_nv_pmc_boot0_log();
    print_criterion(
        "NV_PMC_BOOT_0 desde soso (SOSO_QEMU_GPU=vfio:BDF cargo xtask run)",
        boot0_ok,
    );
    if !boot0_ok {
        println!("       ↑ pendiente tras bind VFIO — ver docs/L6-G1-gate.md");
        println!("       sudo ./scripts/l6-g1-vfio-test.sh");
    }

    if show_hint {
        println!("\n=== Bind VFIO (manual, requiere root; apaga la sesión gráfica) ===");
        if let Some(bdf) = nvidia_lines
            .first()
            .and_then(|l| l.split_whitespace().next())
        {
            let full = format!("0000:{bdf}");
            let pci_n = run_capture("lspci", &["-n", "-s", bdf]);
            let ids = pci_n
                .split_whitespace()
                .find(|t| t.contains("10de:"))
                .unwrap_or("10de:????");
            let (ven, dev) = ids.split_once(':').unwrap_or(("10de", "????"));
            println!(
                r#"
# Prerrequisito: IOMMU activo (cargo xtask g1-check debe mostrar grupos > 0)
sudo modprobe vfio-pci
echo "{full}" | sudo tee /sys/bus/pci/drivers/nvidia/unbind
echo "{ven} {dev}" | sudo tee /sys/bus/pci/drivers/vfio-pci/new_id
echo "{full}" | sudo tee /sys/bus/pci/drivers/vfio-pci/bind
SOSO_QEMU_GPU=vfio:{bdf} cargo xtask run
# Log serie esperado: nvidia: GPU 10de:.... NV_PMC_BOOT_0=0x........
"#
            );
        }
    } else {
        println!("\nTip: ./scripts/l6-g1-preflight.sh  → diagnóstico BIOS + GRUB");
        println!("     cargo xtask g1-check --vfio-hint  → comandos bind VFIO");
        println!("     sudo ./scripts/l6-g1-enable-iommu.sh  → GRUB + update-grub");
        println!("     sudo ./scripts/l6-g1-vfio-test.sh   → bind VFIO + prueba soso (TTY)");
        if !dmar {
            println!("     sudo ./scripts/l6-g1-vfio-noiommu.sh → BAR0 sin IOMMU (no cierra G1)");
        }
    }

    println!("\n=== Resumen: {ok} OK, {warn} WARN, {fail} FAIL ===");
    if fail > 0 {
        exit(1);
    }
}

fn check_nv_pmc_boot0_log() -> bool {
    let log = super::project_root().join("target/g1-vfio-serial.log");
    let Ok(text) = fs::read_to_string(&log) else {
        return false;
    };
    if !text.contains("NV_PMC_BOOT_0=0x") {
        return false;
    }
    println!("\n   NV_PMC_BOOT_0 (desde {})", log.display());
    for line in text.lines().filter(|l| l.contains("nvidia:")) {
        println!("   OK    {line}");
    }
    true
}

fn print_criterion(label: &str, pass: bool) {
    if pass {
        println!("   GO    {label}");
    } else {
        println!("   BLOCK {label}");
    }
}

fn collect_gsp_blobs(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    walk_gsp(root, root, &mut out);
    out.sort();
    out.dedup();
    out
}

fn walk_gsp(base: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for ent in entries.flatten() {
        let path = ent.path();
        if path.is_dir() {
            walk_gsp(base, &path, out);
        } else if path
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|n| n.contains("gsp"))
        {
            out.push(path.strip_prefix(base).unwrap_or(&path).display().to_string());
        }
    }
}

fn run_capture(bin: &str, args: &[&str]) -> String {
    Command::new(bin)
        .args(args)
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default()
}
