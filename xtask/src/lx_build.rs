//! Construcción de la biblioteca estática `liblxdde.a` con drivers Linux portados.
//!
//! Uso: `cargo xtask lx-build [--port spike|testdrv|e1000e|all]`

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, exit};

const LINUX_VERSION: &str = "6.6.32";
const LINUX_URL: &str =
    "https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-6.6.32.tar.xz";

pub fn run(args: &[String]) {
    let root = super::project_root();
    let port = args.first().map(|s| s.as_str()).unwrap_or("all");
    let out_dir = root.join("target/lxdde");
    fs::create_dir_all(&out_dir).expect("crear target/lxdde");

    let ports: Vec<&str> = match port {
        "spike" => vec!["spike"],
        "testdrv" => vec!["testdrv"],
        "e1000e" => vec!["e1000e"],
        "nouveau" => vec!["nouveau"],
        "all" => vec!["spike", "testdrv", "e1000e", "nouveau"],
        other => {
            eprintln!("lx-build: puerto desconocido {other}");
            exit(2);
        }
    };

    // Descargar Linux solo si algún source.list referencia el árbol pinneado.
    if port_needs_linux(&root, &ports) {
        ensure_linux(&root);
    }

    let mut objects = Vec::new();
    for p in &ports {
        objects.extend(compile_port(&root, p, &out_dir));
    }

    // Shim común + stubs generados
    let shim_obj = compile_c(&root, &out_dir, &root.join("lxdde/shim/src/shims.c"), &cc_flags(&root));
    objects.push(shim_obj);

    let stubs = generate_stubs(&root, &out_dir, &objects);
    if stubs.exists() {
        objects.push(compile_c(&root, &out_dir, &stubs, &cc_flags(&root)));
    }

    let archive = out_dir.join("liblxdde.a");
    let mut ar = Command::new("ar");
    ar.arg("rcs").arg(&archive);
    for o in &objects {
        ar.arg(o);
    }
    let st = ar.status().expect("ar");
    if !st.success() {
        exit(st.code().unwrap_or(1));
    }
    println!("lx-build: {}", archive.display());
}

fn linux_root(root: &Path) -> PathBuf {
    let nested = root.join("lxdde/linux");
    if nested.join("Makefile").exists() {
        return nested;
    }
    // Tarball extraído plano bajo lxdde/ (drivers/, include/, …)
    root.join("lxdde")
}

fn port_needs_linux(root: &Path, ports: &[&str]) -> bool {
    for p in ports {
        let list_path = root.join("lxdde/ports").join(p).join("source.list");
        if let Ok(list) = fs::read_to_string(&list_path) {
            for line in list.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') || line.starts_with("lxdde/") {
                    continue;
                }
                return true;
            }
        }
    }
    false
}

fn ensure_linux(root: &Path) {
    let linux_dir = linux_root(root);
    let marker = linux_dir.join("Makefile");
    if marker.exists() {
        return;
    }
    let tar = root.join("target/lxdde").join(format!("linux-{LINUX_VERSION}.tar.xz"));
    fs::create_dir_all(tar.parent().unwrap()).ok();
    if !tar.exists() {
        println!("lx-build: descargando Linux {LINUX_VERSION}…");
        let status = Command::new("curl")
            .args(["-fL", LINUX_URL, "-o"])
            .arg(&tar)
            .status()
            .expect("curl");
        if !status.success() {
            eprintln!("lx-build: fallo descargando {LINUX_URL}");
            exit(1);
        }
    }
    fs::create_dir_all(&linux_dir).ok();
    println!("lx-build: extrayendo tarball…");
    let status = Command::new("tar")
        .args(["-xJf"])
        .arg(&tar)
        .arg("-C")
        .arg(linux_dir.parent().unwrap())
        .arg("--strip-components=1")
        .arg(format!("linux-{LINUX_VERSION}/"))
        .status()
        .expect("tar");
    if !status.success() {
        // tar sin strip: extraer completo y renombrar
        let status = Command::new("tar")
            .args(["-xJf", tar.to_str().unwrap(), "-C", linux_dir.parent().unwrap().to_str().unwrap()])
            .status()
            .expect("tar fallback");
        if !status.success() {
            exit(1);
        }
        let extracted = linux_dir
            .parent()
            .unwrap()
            .join(format!("linux-{LINUX_VERSION}"));
        if extracted.exists() {
            let _ = fs::rename(extracted, &linux_dir);
        }
    }
}

fn cc_flags(root: &Path) -> Vec<String> {
    let linux = linux_root(root);
    let shim = root.join("lxdde/shim/include");
    let nouveau = linux.join("drivers/gpu/drm/nouveau");
    let mut inc = vec![
        format!("-I{}", shim.display()),
        format!("-I{}", linux.join("arch/x86/include").display()),
        format!("-I{}", linux.join("arch/x86/include/uapi").display()),
        format!("-I{}", linux.join("include").display()),
        format!("-I{}", linux.join("include/uapi").display()),
        // Include dirs privados de nouveau (equivalentes al Kbuild del driver):
        // resuelven "priv.h" relativo, <core/*.h>, <subdev/*.h> de nvkm.
        // Solo son rutas -I: inocuos para los demás ports.
        format!("-I{}", nouveau.join("include").display()),
        format!("-I{}", nouveau.join("include/nvkm").display()),
        format!("-I{}", nouveau.join("nvkm").display()),
        format!("-I{}", nouveau.display()),
    ];
    let mut flags = vec![
        "-target".into(),
        "x86_64-unknown-none-elf".into(),
        "-ffreestanding".into(),
        "-fno-stack-protector".into(),
        "-fno-strict-aliasing".into(),
        "-fno-common".into(),
        "-fno-builtin".into(),
        "-nostdinc".into(),
        "-fno-asynchronous-unwind-tables".into(),
        "-mno-red-zone".into(),
        "-mno-sse".into(),
        "-mno-mmx".into(),
        "-fPIE".into(),
        "-mcmodel=kernel".into(),
        "-O2".into(),
        "-Wall".into(),
        "-Wno-unused-parameter".into(),
        "-Wno-macro-redefined".into(),
        "-D__KERNEL__".into(),
        "-include".into(),
        format!("{}/autoconf.h", shim.display()),
        "-include".into(),
        format!("{}/linux/compat.h", shim.display()),
        // El build real del kernel fuerza kconfig.h globalmente (IS_ENABLED, etc.).
        "-include".into(),
        format!("{}/linux/kconfig.h", shim.display()),
    ];
    flags.extend(inc);
    flags
}

fn compile_port(root: &Path, port: &str, out_dir: &Path) -> Vec<PathBuf> {
    let list_path = root.join("lxdde/ports").join(port).join("source.list");
    let list = fs::read_to_string(&list_path).unwrap_or_else(|e| {
        panic!("{}: {e}", list_path.display());
    });
    let flags = cc_flags(root);
    let mut objs = Vec::new();
    for line in list.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let src = if line.starts_with("lxdde/") {
            root.join(line)
        } else {
            linux_root(root).join(line)
        };
        if !src.exists() {
            eprintln!("lx-build: omitiendo (no existe): {}", src.display());
            continue;
        }
        objs.push(compile_c(root, out_dir, &src, &flags));
    }
    objs
}

fn compile_c(root: &Path, out_dir: &Path, src: &Path, flags: &[String]) -> PathBuf {
    let stem = src.file_stem().unwrap().to_str().unwrap();
    let hash = format!("{:x}", md5_simple(src));
    let obj = out_dir.join(format!("{stem}-{hash}.o"));
    if obj.exists() {
        if let Ok(meta_src) = fs::metadata(src) {
            if let Ok(meta_obj) = fs::metadata(&obj) {
                if let (Ok(t_src), Ok(t_obj)) = (meta_src.modified(), meta_obj.modified()) {
                    if t_obj >= t_src {
                        return obj;
                    }
                }
            }
        }
    }
    let compiler = std::env::var("LX_CC").unwrap_or_else(|_| "clang".into());
    let mut cmd = Command::new(&compiler);
    cmd.args(flags).arg("-c").arg(src).arg("-o").arg(&obj);
    let st = cmd.status().unwrap_or_else(|e| panic!("{compiler}: {e}"));
    if !st.success() {
        eprintln!("lx-build: fallo compilando {}", src.display());
        exit(st.code().unwrap_or(1));
    }
    let _ = root;
    obj
}

fn md5_simple(path: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    h.finish()
}

/// Símbolos que la capa Rust/C shim implementa (no generar dummy).
fn provided_symbols() -> HashSet<&'static str> {
    [
        "lx_spike_run", "lx_testdrv_run", "lx_e1000e_init_module", "lx_e1000e_exit_module",
        "lx_e1000e_adapter", "lx_e1000_poll",
        "lx_kmalloc", "lx_kzalloc", "lx_krealloc", "lx_kfree", "lx_vmalloc", "lx_vfree",
        "lx_puts", "lx_putchar", "lx_vprintk", "lx_printk",
        "lx_emul_trace_and_stop", "lx_emul_trace",
        "lx_jiffies", "lx_msecs_to_jiffies", "lx_jiffies_to_msecs", "lx_udelay", "lx_mdelay",
        "lx_ktime_get_ns", "lx_schedule", "lx_yield", "lx_msleep",
        "lx_init_completion", "lx_complete", "lx_wait_for_completion", "lx_wait_for_completion_timeout",
        "lx_init_work", "lx_schedule_work", "lx_flush_work",
        "lx_init_delayed_work", "lx_schedule_delayed_work", "lx_cancel_delayed_work",
        "lx_request_irq", "lx_free_irq", "lx_irq_wake",
        "lx_pci_register_driver", "lx_pci_iomap", "lx_pci_iounmap", "lx_pci_enable_device",
        "lx_pci_disable_device", "lx_pci_set_master", "lx_pci_read_config", "lx_pci_write_config",
        "lx_pci_alloc_irq_vectors", "lx_pci_free_irq_vectors", "lx_pci_irq_vector",
        "lx_pci_get_drvdata", "lx_pci_set_drvdata", "lx_pci_device_id",
        "lx_dma_alloc_coherent", "lx_dma_free_coherent", "lx_dma_map_single", "lx_dma_unmap_single",
        "lx_register_initcall",
        "lx_alloc_etherdev", "lx_register_netdev", "lx_unregister_netdev", "lx_netdev_priv",
        "lx_netif_rx", "lx_alloc_skb", "lx_kfree_skb", "lx_skb_put", "lx_skb_reserve",
        "lx_dev_queue_xmit", "lx_netif_start_queue", "lx_netif_stop_queue", "lx_netif_queue_stopped",
        "lx_netif_carrier_on", "lx_eth_hw_addr_random", "lx_eth_mac_addr",
        "lx_skb_len", "lx_skb_data", "lx_set_netdev_ops", "lx_set_netdev_mac", "lx_tx_head",
        "lx_request_firmware", "lx_release_firmware",
        "lx_drm_dev_alloc", "lx_drm_gem_create", "lx_drm_gem_vmap", "lx_map_wc",
        "lx_nouveau_init_module", "lx_nouveau_gsp_is_ready", "lx_nouveau_gsp_phase",
        "lx_nouveau_compute_saxpy", "lx_nouveau_compute_matvec_f32", "lx_nouveau_vram_total",
        "lx_nouveau_set_boot0",
        "memcpy", "memset", "memmove", "strlen", "strcmp", "strncmp", "strncpy", "strnlen",
    ]
    .into_iter()
    .collect()
}

fn generate_stubs(root: &Path, out_dir: &Path, objects: &[PathBuf]) -> PathBuf {
    let stubs_path = root.join("lxdde/shim/src/generated_dummies.c");
    let provided = provided_symbols();
    // Recolectar por separado símbolos definidos y no definidos en TODO el
    // conjunto de objetos: un dummy solo hace falta para lo undefined que
    // ningún objeto define (evita definiciones duplicadas, p.ej. nvkm_gsp_new_
    // definido en base.o pero U en ga102.o).
    let mut undefined = HashSet::new();
    let mut defined = HashSet::new();

    for obj in objects {
        let output = Command::new("nm")
            .arg(obj)
            .output()
            .expect("nm");
        if !output.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("nm:") {
                continue;
            }
            // Formato nm: "<addr> <T> <name>" (definido) o "<U> <name>" (no def).
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 2 {
                continue;
            }
            let name = parts.last().copied().unwrap_or("");
            // El tipo es el token anterior al nombre (U para undefined).
            let sym_type = parts[parts.len() - 2];
            if sym_type == "U" {
                // Los `__`-prefijados suelen ser builtins del compilador (no
                // stub), salvo la API interna de nvkm (`__nvkm_*`).
                let skip_underscore = name.starts_with("__") && !name.starts_with("__nvkm");
                if !provided.contains(name) && !skip_underscore {
                    undefined.insert(name.to_string());
                }
            } else if sym_type != "w" && sym_type != "v" {
                // Cualquier definición real (T/t/D/B/R/…) satisface el símbolo.
                defined.insert(name.to_string());
            }
        }
    }

    // Solo dummy para lo que nadie define.
    undefined.retain(|n| !defined.contains(n));

    if undefined.is_empty() {
        let _ = fs::write(&stubs_path, "/* sin stubs */\n");
        return stubs_path;
    }

    let mut f = fs::File::create(&stubs_path).expect("stubs");
    writeln!(f, "/* Generado por xtask lx-build — NO EDITAR */").unwrap();
    writeln!(f, "#include \"lx_emul.h\"").unwrap();
    let mut names: Vec<_> = undefined.into_iter().collect();
    names.sort();
    for name in &names {
        writeln!(
            f,
            "void {name}(void) {{ lx_emul_trace_and_stop(\"{name}\"); }}"
        )
        .unwrap();
    }
    let _ = out_dir;
    stubs_path
}
