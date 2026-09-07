//! `cargo xtask hw-matrix` — matriz versionada de validación por equipo (A8).

use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::drivers::{parse_hwscan_file, ScanLine};

const MATRIX_PATH: &str = "docs/hw-matrix.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StageStatus {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nota: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fecha: Option<String>,
}

impl StageStatus {
    fn pendiente() -> Self {
        Self {
            status: "pendiente".into(),
            nota: None,
            fecha: None,
        }
    }

    fn ok(nota: Option<&str>) -> Self {
        Self {
            status: "ok".into(),
            nota: nota.map(String::from),
            fecha: Some(today()),
        }
    }

    fn from_bool(ok: bool, ok_note: &str, fail_note: &str) -> Self {
        if ok {
            Self::ok(Some(ok_note))
        } else {
            Self {
                status: "fail".into(),
                nota: Some(fail_note.into()),
                fecha: Some(today()),
            }
        }
    }
}

impl Default for StageStatus {
    fn default() -> Self {
        Self::pendiente()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WifiStages {
    pub alive: StageStatus,
    pub scan: StageStatus,
    pub assoc_wpa2: StageStatus,
    pub dhcp: StageStatus,
    pub ssh: StageStatus,
    pub reconexion: StageStatus,
}

impl Default for WifiStages {
    fn default() -> Self {
        Self {
            alive: StageStatus::pendiente(),
            scan: StageStatus::pendiente(),
            assoc_wpa2: StageStatus::pendiente(),
            dhcp: StageStatus::pendiente(),
            ssh: StageStatus::pendiente(),
            reconexion: StageStatus::pendiente(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GpuStages {
    pub gsp_rpc: StageStatus,
    pub vram_pool: StageStatus,
    pub ce_readback: StageStatus,
    pub compute_cpu_gpu: StageStatus,
    pub carga_real: StageStatus,
    pub apagado_limpio: StageStatus,
}

impl Default for GpuStages {
    fn default() -> Self {
        Self {
            gsp_rpc: StageStatus::pendiente(),
            vram_pool: StageStatus::pendiente(),
            ce_readback: StageStatus::pendiente(),
            compute_cpu_gpu: StageStatus::pendiente(),
            carga_real: StageStatus::pendiente(),
            apagado_limpio: StageStatus::pendiente(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BenchRecord {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modelo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tok_s_mediana: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tok_s_frio: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tok_s_caliente: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nota: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GitInfo {
    pub commit: String,
    pub commit_corto: String,
    pub version: String,
    pub dirty: bool,
    #[serde(default)]
    pub cambios_locales: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct HwEntry {
    pub id: String,
    pub equipo: String,
    #[serde(default)]
    pub pci_ids: Vec<String>,
    #[serde(default)]
    pub firmware: std::collections::BTreeMap<String, String>,
    pub git: GitInfo,
    #[serde(default)]
    pub perfil_drivers: String,
    #[serde(default)]
    pub arranques_consecutivos_ok: u8,
    #[serde(default)]
    pub sesion_sostenida: StageStatus,
    #[serde(default)]
    pub wifi: WifiStages,
    #[serde(default)]
    pub gpu: GpuStages,
    #[serde(default)]
    pub bench: BenchRecord,
    #[serde(default)]
    pub logs: Vec<String>,
    #[serde(default)]
    pub notas: String,
    pub actualizado: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct HwMatrix {
    pub schema_version: u32,
    pub entries: Vec<HwEntry>,
}

pub fn run(args: &[String]) {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("show");
    match sub {
        "show" => show(),
        "init" => init(),
        "collect" => collect(&args[1..]),
        "parse-logs" => parse_logs_cmd(&args[1..]),
        "record-boot" => record_boot(&args[1..]),
        "record-bench" => record_bench(&args[1..]),
        "host-check" => host_check(),
        "help" | "--help" | "-h" => usage(),
        other => {
            eprintln!("hw-matrix: subcomando desconocido {other}");
            usage();
            std::process::exit(2);
        }
    }
}

fn usage() {
    eprintln!(
        "uso:\n\
         cargo xtask hw-matrix show\n\
         cargo xtask hw-matrix init\n\
         cargo xtask hw-matrix collect --id ID [--equipo N] [--pci 8086:2723] [--perfil live-usb]\n\
         cargo xtask hw-matrix parse-logs --id ID [--sosolog F] [--sosodrv F] [--serial F]\n\
         cargo xtask hw-matrix record-boot --id ID [--fail]\n\
         cargo xtask hw-matrix record-bench --id ID --tok-s 1.23 [--frio 1.1] [--caliente 1.3]\n\
         cargo xtask hw-matrix host-check"
    );
}

fn matrix_path() -> PathBuf {
    crate::project_root().join(MATRIX_PATH)
}

fn load_matrix() -> HwMatrix {
    let path = matrix_path();
    if !path.exists() {
        return default_matrix();
    }
    let text = std::fs::read_to_string(&path).expect("leer hw-matrix.json");
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("hw-matrix.json inválido: {e}"))
}

fn save_matrix(m: &HwMatrix) {
    let path = matrix_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("mkdir docs");
    }
    let text = serde_json::to_string_pretty(m).expect("serializar matriz");
    std::fs::write(&path, text).expect("escribir hw-matrix.json");
}

fn today() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}

fn git_info() -> GitInfo {
    let root = crate::project_root();
    let commit = run_capture_in(&root, "git", &["rev-parse", "HEAD"]);
    let commit_corto = run_capture_in(&root, "git", &["rev-parse", "--short", "HEAD"]);
    let dirty = !run_capture_in(&root, "git", &["status", "--porcelain"]).is_empty();
    let version = crate::version::read_version(&root);
    let cambios = if dirty {
        run_capture_in(&root, "git", &["status", "--short"]).lines().count().to_string()
            + " ficheros modificados"
    } else {
        String::new()
    };
    GitInfo {
        commit: commit.trim().into(),
        commit_corto: commit_corto.trim().into(),
        version,
        dirty,
        cambios_locales: cambios,
    }
}

fn run_capture_in(cwd: &Path, bin: &str, args: &[&str]) -> String {
    Command::new(bin)
        .current_dir(cwd)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

fn default_matrix() -> HwMatrix {
    let git = git_info();
    HwMatrix {
        schema_version: 1,
        entries: vec![
            seed_entry(
                "qemu-test",
                "QEMU q35 (cargo xtask test)",
                vec![],
                "test-shards",
                &git,
                "Suite automatizada llm-dense/sys; WiFi/GPU reales pendientes",
            ),
            seed_entry(
                "gb205-dgpu",
                "NVIDIA GB205 (RTX 5070 Ti Mobile)",
                vec!["10de:2f18".into()],
                "live-usb + nouveau",
                &git,
                "G1–G5 en placa; validar tras cambios FWSEC/falcon",
            ),
            seed_entry(
                "ga107-igpu",
                "NVIDIA GA107 (iGPU / Ampere)",
                vec!["10de:????".into()],
                "live-usb + nouveau",
                &git,
                "Cadena Ampere pendiente de cierre",
            ),
            seed_entry(
                "ax211-wifi",
                "Intel AX211 (CNVi gen3)",
                vec!["8086:7f70".into(), "8086:51f0".into(), "8086:54f0".into()],
                "live-usb + iwlwifi",
                &git,
                "Secuencia: ALIVE → scan → WPA2 → DHCP → SSH",
            ),
            seed_entry(
                "ax200-wifi",
                "Intel AX200 (gen2)",
                vec!["8086:2723".into()],
                "live-usb + iwlwifi",
                &git,
                "Context-info gen2; sin PNVM",
            ),
        ],
    }
}

fn seed_entry(
    id: &str,
    equipo: &str,
    pci: Vec<String>,
    perfil: &str,
    git: &GitInfo,
    notas: &str,
) -> HwEntry {
    HwEntry {
        id: id.into(),
        equipo: equipo.into(),
        pci_ids: pci,
        firmware: std::collections::BTreeMap::new(),
        git: git.clone(),
        perfil_drivers: perfil.into(),
        arranques_consecutivos_ok: 0,
        sesion_sostenida: StageStatus::pendiente(),
        wifi: WifiStages::default(),
        gpu: GpuStages::default(),
        bench: BenchRecord::default(),
        logs: Vec::new(),
        notas: notas.into(),
        actualizado: today(),
    }
}

fn init() {
    let path = matrix_path();
    if path.exists() {
        eprintln!("hw-matrix: {} ya existe (usa collect/parse-logs para actualizar)", path.display());
        return;
    }
    save_matrix(&default_matrix());
    println!("hw-matrix: creado {}", path.display());
}

fn find_entry<'a>(m: &'a mut HwMatrix, id: &str) -> Option<&'a mut HwEntry> {
    m.entries.iter_mut().find(|e| e.id == id)
}

fn collect(args: &[String]) {
    let id = flag(args, "--id").unwrap_or_else(|| {
        eprintln!("hw-matrix collect: falta --id");
        std::process::exit(2);
    });
    let mut m = load_matrix();
    if !m.entries.iter().any(|e| e.id == id) {
        m.entries.push(seed_entry(
            &id,
            &flag(args, "--equipo").unwrap_or_else(|| id.clone()),
            flag(args, "--pci").map(|p| vec![p]).unwrap_or_default(),
            &flag(args, "--perfil").unwrap_or_else(|| "?".into()),
            &git_info(),
            "",
        ));
    }
    let git = git_info();
    let entry = find_entry(&mut m, &id).expect("entrada");
    entry.git = git;
    if let Some(eq) = flag(args, "--equipo") {
        entry.equipo = eq;
    }
    if let Some(pci) = flag(args, "--pci") {
        if !entry.pci_ids.contains(&pci) {
            entry.pci_ids.push(pci);
        }
    }
    if let Some(perfil) = flag(args, "--perfil") {
        entry.perfil_drivers = perfil;
    }
    if let Some(nota) = flag(args, "--notas") {
        entry.notas = nota;
    }
    collect_firmware_hashes(entry);
    entry.actualizado = today();
    let equipo = entry.equipo.clone();
    save_matrix(&m);
    println!("hw-matrix: actualizado {id} ({equipo})");
}

fn collect_firmware_hashes(entry: &mut HwEntry) {
    let root = crate::project_root().join("rootfs/lib/firmware");
    let patterns = [
        ("iwlwifi-so-a0-gf-a0-89.ucode", "ax211_ucode"),
        ("iwlwifi-cc-a0-77.ucode", "ax200_ucode"),
        ("nvidia/ad103/gsp/gsp.bin", "gsp_ad103"),
    ];
    for (rel, key) in patterns {
        let p = root.join(rel);
        if p.is_file() {
            if let Ok(bytes) = std::fs::read(&p) {
                let hash = sha256_hex(&bytes);
                entry.firmware.insert(key.into(), format!("{hash} {}B", bytes.len()));
            }
        }
    }
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(data);
    format!("{:x}", h.finalize())
}

fn parse_logs_cmd(args: &[String]) {
    let id = flag(args, "--id").unwrap_or_else(|| {
        eprintln!("hw-matrix parse-logs: falta --id");
        std::process::exit(2);
    });
    let mut m = load_matrix();
    if !m.entries.iter().any(|e| e.id == id) {
        eprintln!("hw-matrix parse-logs: entrada {id} desconocida; ejecuta collect primero");
        std::process::exit(2);
    }
    let mut blob = String::new();
    let mut log_paths: Vec<PathBuf> = Vec::new();
    for (flag, label) in [
        ("--sosolog", "SOSOLOG"),
        ("--sosodrv", "SOSODRV"),
        ("--sosowifi", "SOSOWIFI"),
        ("--serial", "serial"),
    ] {
        if let Some(p) = flag_path(args, flag) {
            if let Ok(t) = std::fs::read_to_string(&p) {
                blob.push_str(&format!("\n--- {label} ---\n{t}"));
                log_paths.push(p);
            }
        }
    }
    if blob.trim().is_empty() {
        eprintln!("hw-matrix parse-logs: indica al menos un --sosolog|--sosodrv|--serial");
        std::process::exit(2);
    }
    let entry = find_entry(&mut m, &id).expect("entrada");
    for p in log_paths {
        let s = p.display().to_string();
        if !entry.logs.contains(&s) {
            entry.logs.push(s);
        }
    }
    entry.wifi = parse_wifi_stages(&blob);
    entry.gpu = parse_gpu_stages(&blob);
    entry.sesion_sostenida = parse_sesion(&blob);
    if let Some(drv) = flag_path(args, "--sosodrv") {
        apply_hwscan(entry, &drv);
    }
    entry.actualizado = today();
    let summary = entry.clone();
    save_matrix(&m);
    println!("hw-matrix: etapas parseadas para {id}");
    show_entry_summary(&summary);
}

fn apply_hwscan(entry: &mut HwEntry, sosodrv: &Path) {
    let lines: Vec<ScanLine> = parse_hwscan_file(sosodrv);
    let mut parts: Vec<String> = lines
        .iter()
        .map(|l| format!("{}={}", l.driver, l.status))
        .collect();
    parts.sort();
    if !parts.is_empty() {
        entry.notas = format!("hwscan: {}", parts.join(", "));
    }
}

pub fn parse_wifi_stages(text: &str) -> WifiStages {
    let t = text.to_ascii_lowercase();
    WifiStages {
        alive: if t.contains("ucode_alive_ntfy") || t.contains("firmware alive") {
            StageStatus::ok(Some("UCODE_ALIVE_NTFY"))
        } else if t.contains("alive degradado") {
            StageStatus::from_bool(false, "", "ALIVE degradado no válido")
        } else {
            StageStatus::pendiente()
        },
        scan: if t.contains("wifi scan") || t.contains("scan:") && t.contains("ssid") {
            StageStatus::ok(None)
        } else {
            StageStatus::pendiente()
        },
        assoc_wpa2: if t.contains("asociad") || t.contains("wpa2") || t.contains("4-way") {
            StageStatus::ok(None)
        } else {
            StageStatus::pendiente()
        },
        dhcp: if t.contains("dhcp") && (t.contains("lease") || t.contains("ok") || t.contains("192."))
        {
            StageStatus::ok(None)
        } else {
            StageStatus::pendiente()
        },
        ssh: if t.contains("ssh") && t.contains("ok") || t.contains("sosh —") {
            StageStatus::ok(Some("consola/SSH"))
        } else {
            StageStatus::pendiente()
        },
        reconexion: if t.contains("reconex") || t.contains("reconnect") {
            StageStatus::ok(None)
        } else {
            StageStatus::pendiente()
        },
    }
}

pub fn parse_gpu_stages(text: &str) -> GpuStages {
    let t = text.to_ascii_lowercase();
    GpuStages {
        gsp_rpc: if (t.contains("gsp") && t.contains("ok")) || t.contains("gsp-rm") {
            StageStatus::ok(None)
        } else {
            StageStatus::pendiente()
        },
        vram_pool: if t.contains("vram") && (t.contains("pool") || t.contains("libre")) {
            StageStatus::ok(None)
        } else {
            StageStatus::pendiente()
        },
        ce_readback: if t.contains("readback") && t.contains("go") {
            StageStatus::ok(Some("CE readback GO"))
        } else {
            StageStatus::pendiente()
        },
        compute_cpu_gpu: if t.contains("matvec") && t.contains("gpu") {
            StageStatus::ok(None)
        } else if t.contains("dispositivo «soft") {
            StageStatus::ok(Some("dispositivo software (QEMU)"))
        } else {
            StageStatus::pendiente()
        },
        carga_real: if t.contains("soso-llm: generado") || t.contains("soso-llm run") {
            StageStatus::ok(None)
        } else {
            StageStatus::pendiente()
        },
        apagado_limpio: if (t.contains("unload=ok") || t.contains("unload=fallo"))
            && (t.contains("dma=off") || t.contains("bus master quitado"))
        {
            StageStatus::ok(Some("unload/halt/dma"))
        } else {
            StageStatus::pendiente()
        },
    }
}

fn parse_sesion(text: &str) -> StageStatus {
    let t = text.to_ascii_lowercase();
    if t.contains("sosh —") && (t.contains("net: dhcp") || t.contains("dhcp")) {
        StageStatus::ok(Some("shell + red"))
    } else if t.contains("sosh —") {
        StageStatus::ok(Some("shell"))
    } else {
        StageStatus::pendiente()
    }
}

fn record_boot(args: &[String]) {
    let id = flag(args, "--id").unwrap_or_else(|| {
        eprintln!("hw-matrix record-boot: falta --id");
        std::process::exit(2);
    });
    let fail = args.iter().any(|a| a == "--fail");
    let mut m = load_matrix();
    let entry = find_entry(&mut m, &id).expect("entrada desconocida");
    if fail {
        entry.arranques_consecutivos_ok = 0;
        entry.notas = format!("{}; arranque fallido {}", entry.notas, today());
    } else {
        entry.arranques_consecutivos_ok = entry.arranques_consecutivos_ok.saturating_add(1);
    }
    entry.actualizado = today();
    let n = entry.arranques_consecutivos_ok;
    save_matrix(&m);
    println!("hw-matrix: {id} arranques_consecutivos_ok={n}");
}

fn record_bench(args: &[String]) {
    let id = flag(args, "--id").unwrap_or_else(|| {
        eprintln!("hw-matrix record-bench: falta --id");
        std::process::exit(2);
    });
    let tok_s: f64 = flag(args, "--tok-s")
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            eprintln!("hw-matrix record-bench: falta --tok-s");
            std::process::exit(2);
        });
    let mut m = load_matrix();
    let entry = find_entry(&mut m, &id).expect("entrada desconocida");
    entry.bench.modelo = flag(args, "--modelo").or(Some("tiny".into()));
    entry.bench.prompt = flag(args, "--prompt").or(Some("test".into()));
    entry.bench.seed = flag(args, "--seed").and_then(|s| s.parse().ok()).or(Some(42));
    entry.bench.max_tokens = flag(args, "--max")
        .and_then(|s| s.parse().ok())
        .or(Some(8));
    entry.bench.tok_s_mediana = Some(tok_s);
    entry.bench.tok_s_frio = flag(args, "--frio").and_then(|s| s.parse().ok());
    entry.bench.tok_s_caliente = flag(args, "--caliente").and_then(|s| s.parse().ok());
    entry.bench.backend = flag(args, "--backend");
    entry.bench.nota = flag(args, "--nota");
    entry.actualizado = today();
    save_matrix(&m);
    println!("hw-matrix: bench {id} mediana {tok_s:.2} tok/s");
}

fn host_check() {
    println!("=== A8 host-check (parser + simulación, no ALIVE en placa) ===\n");
    let root = crate::project_root();
    let scripts = [
        ("iwl-fw", "scripts/l6-iwl-fw-hostcheck.sh"),
        ("gsp-host", "scripts/l6-g3-gsp-hostcheck.sh"),
    ];
    let mut ok = 0u32;
    let mut fail = 0u32;
    for (name, rel) in scripts {
        let path = root.join(rel);
        print!("{name}: ");
        if !path.is_file() {
            println!("SKIP (no existe {rel})");
            continue;
        }
        let st = Command::new("bash").arg(&path).current_dir(&root).status();
        match st {
            Ok(s) if s.success() => {
                ok += 1;
                println!("OK");
            }
            Ok(s) => {
                fail += 1;
                println!("FAIL (exit {:?})", s.code());
            }
            Err(e) => {
                fail += 1;
                println!("FAIL ({e})");
            }
        }
    }
    println!("\nhost-check: {ok} OK, {fail} FAIL");
    if fail > 0 {
        std::process::exit(1);
    }
}

fn show() {
    let m = load_matrix();
    println!("hw-matrix v{} — {} entradas\n", m.schema_version, m.entries.len());
    for e in &m.entries {
        show_entry_summary(e);
        println!();
    }
}

fn show_entry_summary(e: &HwEntry) {
    println!(
        "[{}] {} — {} ({}{})",
        e.id,
        e.equipo,
        e.git.version,
        e.git.commit_corto,
        if e.git.dirty { " dirty" } else { "" }
    );
    println!(
        "  perfil={} pci={:?} arranques_ok={}/3",
        e.perfil_drivers, e.pci_ids, e.arranques_consecutivos_ok
    );
    print_stages("  wifi", &[
        ("alive", &e.wifi.alive),
        ("scan", &e.wifi.scan),
        ("wpa2", &e.wifi.assoc_wpa2),
        ("dhcp", &e.wifi.dhcp),
        ("ssh", &e.wifi.ssh),
        ("recon", &e.wifi.reconexion),
    ]);
    print_stages("  gpu ", &[
        ("gsp", &e.gpu.gsp_rpc),
        ("vram", &e.gpu.vram_pool),
        ("ce", &e.gpu.ce_readback),
        ("cpu/gpu", &e.gpu.compute_cpu_gpu),
        ("carga", &e.gpu.carga_real),
        ("apagado", &e.gpu.apagado_limpio),
    ]);
    if let Some(t) = e.bench.tok_s_mediana {
        println!("  bench: {t:.2} tok/s ({})", e.bench.backend.as_deref().unwrap_or("?"));
    }
    if !e.notas.is_empty() {
        println!("  notas: {}", e.notas.trim());
    }
}

fn print_stages(prefix: &str, stages: &[(&str, &StageStatus)]) {
    let s: Vec<String> = stages
        .iter()
        .map(|(n, st)| format!("{n}={}", st.status))
        .collect();
    println!("{prefix}: {}", s.join(" "));
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn flag_path(args: &[String], name: &str) -> Option<PathBuf> {
    flag(args, name).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_wifi_alive_ok() {
        let w = parse_wifi_stages("iwl: UCODE_ALIVE_NTFY dhcp lease 192.168.1.2");
        assert_eq!(w.alive.status, "ok");
        assert_eq!(w.dhcp.status, "ok");
    }

    #[test]
    fn parse_gpu_shutdown() {
        let g = parse_gpu_stages("nouveau-lx: unload=ok halt=ok dma=off readback GO");
        assert_eq!(g.ce_readback.status, "ok");
        assert_eq!(g.apagado_limpio.status, "ok");
    }
}
