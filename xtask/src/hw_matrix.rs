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

    #[allow(dead_code)]
    fn from_bool(ok: bool, ok_note: &str, fail_note: &str) -> Self {
        if ok {
            Self::ok(Some(ok_note))
        } else {
            Self::fail(fail_note)
        }
    }

    fn fail(nota: &str) -> Self {
        Self {
            status: "fail".into(),
            nota: Some(nota.into()),
            fecha: Some(today()),
        }
    }

    fn no_aplica(nota: &str) -> Self {
        Self {
            status: "no_aplica".into(),
            nota: Some(nota.into()),
            fecha: Some(today()),
        }
    }

    fn is_pendiente(&self) -> bool {
        self.status == "pendiente"
    }
}

/// Conserva el valor anterior solo para el historial agregado.
/// El estado *actual* de una ejecución nueva no usa esta fusión.
pub fn merge_stage(prev: &StageStatus, incoming: StageStatus) -> StageStatus {
    if incoming.is_pendiente() {
        prev.clone()
    } else {
        incoming
    }
}

fn merge_wifi(prev: &WifiStages, incoming: WifiStages) -> WifiStages {
    WifiStages {
        alive: merge_stage(&prev.alive, incoming.alive),
        init_complete: merge_stage(&prev.init_complete, incoming.init_complete),
        mvm_ready: merge_stage(&prev.mvm_ready, incoming.mvm_ready),
        scan: merge_stage(&prev.scan, incoming.scan),
        assoc_wpa2: merge_stage(&prev.assoc_wpa2, incoming.assoc_wpa2),
        dhcp: merge_stage(&prev.dhcp, incoming.dhcp),
        ssh: merge_stage(&prev.ssh, incoming.ssh),
        reconexion: merge_stage(&prev.reconexion, incoming.reconexion),
    }
}

#[allow(dead_code)]
fn merge_gpu(prev: &GpuStages, incoming: GpuStages) -> GpuStages {
    GpuStages {
        gsp_rpc: merge_stage(&prev.gsp_rpc, incoming.gsp_rpc),
        vram_pool: merge_stage(&prev.vram_pool, incoming.vram_pool),
        ce_readback: merge_stage(&prev.ce_readback, incoming.ce_readback),
        compute_cpu_gpu: merge_stage(&prev.compute_cpu_gpu, incoming.compute_cpu_gpu),
        carga_real: merge_stage(&prev.carga_real, incoming.carga_real),
        apagado_limpio: merge_stage(&prev.apagado_limpio, incoming.apagado_limpio),
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
    #[serde(default)]
    pub init_complete: StageStatus,
    #[serde(default)]
    pub mvm_ready: StageStatus,
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
            init_complete: StageStatus::pendiente(),
            mvm_ready: StageStatus::pendiente(),
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
    pub historial: Vec<RunEvidence>,
    #[serde(default)]
    pub notas: String,
    pub actualizado: String,
}

/// Una ejecución concreta: no se presenta como evidencia del kernel actual
/// si el log/hash ya no coincide.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RunEvidence {
    pub log_hash: String,
    #[serde(default)]
    pub kernel: String,
    #[serde(default)]
    pub pci: Vec<String>,
    #[serde(default)]
    pub wifi: WifiStages,
    #[serde(default)]
    pub gpu: GpuStages,
    #[serde(default)]
    pub fecha: String,
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
        run_capture_in(&root, "git", &["status", "--short"])
            .lines()
            .count()
            .to_string()
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
                vec!["10de:249c".into()],
                "live-usb + nouveau",
                &git,
                "PCI 10de:249c (ROG 3050 Mobile). Cadena Ampere pendiente de revalidar en placa",
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
        historial: Vec::new(),
        notas: notas.into(),
        actualizado: today(),
    }
}

fn init() {
    let path = matrix_path();
    if path.exists() {
        eprintln!(
            "hw-matrix: {} ya existe (usa collect/parse-logs para actualizar)",
            path.display()
        );
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
        if !pci_id_concreto(&pci) {
            eprintln!(
                "hw-matrix collect: PCI '{pci}' no es un id concreto (vvvv:dddd hex, sin ????)"
            );
            std::process::exit(2);
        }
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

fn firmware_files_for(id: &str) -> &'static [(&'static str, &'static str)] {
    match id {
        "ax211-wifi" => &[("iwlwifi-so-a0-gf-a0-89.ucode", "ax211_ucode")],
        "ax200-wifi" => &[("iwlwifi-cc-a0-77.ucode", "ax200_ucode")],
        "gb205-dgpu" => &[("nvidia/gb205/gsp/gsp-570.144.bin", "gsp_gb205")],
        // En árbol: fallback ga102, no blob nativo ga107.
        "ga107-igpu" => &[("nvidia/ga102/gsp/gsp-570.144.bin", "gsp_ga102_fallback")],
        _ => &[],
    }
}

fn collect_firmware_hashes(entry: &mut HwEntry) {
    let root = crate::project_root().join("rootfs/lib/firmware");
    for (rel, key) in firmware_files_for(&entry.id) {
        let p = root.join(rel);
        if p.is_file() {
            if let Ok(bytes) = std::fs::read(&p) {
                let hash = sha256_hex(&bytes);
                entry
                    .firmware
                    .insert((*key).into(), format!("{hash} {}B", bytes.len()));
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
    let runs = parse_log_runs(&blob);
    for run in &runs {
        if !entry
            .historial
            .iter()
            .any(|h| h.log_hash == run.log_hash && h.kernel == run.kernel)
        {
            entry.historial.push(run.clone());
        }
    }
    if let Some(last) = runs.last() {
        entry.wifi = last.wifi.clone();
        entry.gpu = last.gpu.clone();
        entry.sesion_sostenida = parse_sesion(last_boot_text(&blob));
        if boot_ok_criterio(last_boot_text(&blob)) {
            entry.arranques_consecutivos_ok = entry.arranques_consecutivos_ok.saturating_add(1);
        } else if last_boot_text(&blob).contains("boot: memtest") {
            entry.arranques_consecutivos_ok = 0;
        }
    }
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

fn line_lc(text: &str) -> Vec<String> {
    text.lines().map(|l| l.to_ascii_lowercase()).collect()
}

fn any_line(lines: &[String], pred: impl Fn(&str) -> bool) -> bool {
    lines.iter().any(|l| pred(l))
}

/// Parte logs concatenados por el marcador de arranque `boot: memtest`.
pub fn split_boots(text: &str) -> Vec<&str> {
    let mut idxs = Vec::new();
    let mut pos = 0;
    while let Some(rel) = text[pos..].find("boot: memtest") {
        idxs.push(pos + rel);
        pos += rel + 1;
    }
    if idxs.is_empty() {
        return vec![text];
    }
    let mut out = Vec::new();
    if idxs[0] > 0 {
        let prefix = text[..idxs[0]].trim();
        if !prefix.is_empty() {
            out.push(&text[..idxs[0]]);
        }
    }
    for (i, start) in idxs.iter().enumerate() {
        let end = idxs.get(i + 1).copied().unwrap_or(text.len());
        out.push(&text[*start..end]);
    }
    out
}

pub fn last_boot_text(text: &str) -> &str {
    split_boots(text).last().copied().unwrap_or(text)
}

fn extract_kernel_id(text: &str) -> String {
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with("soso ") || l.contains("SOSOKRN") || l.starts_with("version=") {
            return l.chars().take(80).collect();
        }
    }
    String::new()
}

fn extract_pci(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        for token in line.split(|c: char| !c.is_ascii_hexdigit() && c != ':') {
            if pci_id_concreto(token) && !out.iter().any(|p| p == token) {
                out.push(token.to_ascii_lowercase());
            }
        }
    }
    out
}

pub fn parse_log_runs(text: &str) -> Vec<RunEvidence> {
    split_boots(text)
        .into_iter()
        .map(|boot| RunEvidence {
            log_hash: sha256_hex(boot.as_bytes()),
            kernel: extract_kernel_id(boot),
            pci: extract_pci(boot),
            wifi: parse_wifi_stages(boot),
            gpu: parse_gpu_stages(boot),
            fecha: today(),
        })
        .collect()
}

fn boot_ok_criterio(text: &str) -> bool {
    let lines = line_lc(text);
    any_line(&lines, |l| l.contains("boot: task") || l.contains("sosh —"))
        && !any_line(&lines, |l| l.contains("panic") || l.contains("double fault"))
}

fn wifi_link_up(lines: &[String]) -> bool {
    any_line(lines, |l| {
        (l.contains("assoc=ok") || l.contains("associated=1") || l.contains("wifi asociado"))
            && !l.contains("no asociado")
    })
}

pub fn parse_wifi_stages(text: &str) -> WifiStages {
    let text = last_boot_text(text);
    let lines = line_lc(text);
    WifiStages {
        alive: if any_line(&lines, |l| {
            l.contains("alive degradado")
                || l.contains("alive=fallo")
                || l.contains("timeout alive")
                || l.contains("alive=false")
        }) {
            StageStatus::fail("ALIVE degradado o fallido")
        } else if any_line(&lines, |l| {
            l.contains("ucode_alive_ntfy") || l.contains("firmware alive")
        }) {
            StageStatus::ok(Some("UCODE_ALIVE_NTFY"))
        } else {
            StageStatus::pendiente()
        },
        init_complete: if any_line(&lines, |l| l.contains("init_complete_notif")) {
            StageStatus::ok(Some("INIT_COMPLETE_NOTIF"))
        } else {
            StageStatus::pendiente()
        },
        mvm_ready: if any_line(&lines, |l| l.contains("up mínimo listo") || l.contains("up minimo listo"))
        {
            StageStatus::ok(Some("MVM up"))
        } else {
            StageStatus::pendiente()
        },
        scan: if any_line(&lines, |l| {
            l.contains("scan=fallo")
                || l.contains("wifi scan fallo")
                || l.contains("wifi: ninguna red")
                || l.contains("scan_req_umac rechazado")
        }) {
            StageStatus::fail("scan fallido o sin BSS")
        } else if any_line(&lines, |l| {
            l.contains("scan fin") && (l.contains("end=1") || l.contains("complete=1"))
        }) {
            StageStatus::ok(Some("fin normal"))
        } else if any_line(&lines, |l| l.contains("wifi scan:") && l.contains("ssid")) {
            StageStatus::ok(None)
        } else {
            StageStatus::pendiente()
        },
        assoc_wpa2: if any_line(&lines, |l| {
            l.contains("assoc=fallo")
                || l.contains("wpa2 fallo")
                || l.contains("no asociado")
        }) {
            StageStatus::fail("asociación WPA2 fallida")
        } else if any_line(&lines, |l| l.contains("assoc=ok"))
            || any_line(&lines, |l| l.contains("4-way") && l.contains("ok"))
        {
            StageStatus::ok(None)
        } else {
            StageStatus::pendiente()
        },
        dhcp: {
            let wifi_dhcp_fail = any_line(&lines, |l| {
                l.contains("dhcp")
                    && l.contains("fallo")
                    && (l.contains("wifi") || l.contains("lxwifi") || l.contains("lx-wifi"))
                    && !l.contains("gsp")
            });
            let wifi_dhcp_ok = any_line(&lines, |l| {
                l.contains("net: dhcp ")
                    && (l.contains("192.") || l.contains("10.") || l.contains("172."))
                    && (l.contains("wifi")
                        || l.contains("lxwifi")
                        || l.contains("lx-wifi")
                        || l.contains("iwl"))
            }) || (wifi_link_up(&lines)
                && any_line(&lines, |l| {
                    l.contains("net: dhcp ")
                        && (l.contains("192.") || l.contains("10.") || l.contains("172."))
                }));
            if wifi_dhcp_fail {
                StageStatus::fail("DHCP fallido")
            } else if wifi_dhcp_ok {
                StageStatus::ok(None)
            } else {
                StageStatus::pendiente()
            }
        },
        ssh: if any_line(&lines, |l| {
            l.contains("ssh: sesión") || l.contains("ssh: sesion") || l.contains("ssh conectado")
        }) {
            StageStatus::ok(Some("SSH"))
        } else if any_line(&lines, |l| {
            l.contains("ssh:") && l.contains("ok") && !l.contains("sosh")
        }) {
            StageStatus::ok(Some("SSH"))
        } else if any_line(&lines, |l| {
            l.contains("ssh") && l.contains("fallo") && !l.contains("gsp=fallo")
        }) {
            StageStatus::fail("SSH fallido")
        } else {
            StageStatus::pendiente()
        },
        reconexion: if any_line(&lines, |l| {
            (l.contains("reconex") || l.contains("reconnect")) && l.contains("fallo")
        }) {
            StageStatus::fail("reconexión fallida")
        } else if any_line(&lines, |l| {
            (l.contains("reconex") || l.contains("reconnect"))
                && (l.contains("ok") || l.contains("éxito") || l.contains("exito"))
        }) {
            StageStatus::ok(None)
        } else {
            StageStatus::pendiente()
        },
    }
}

pub fn parse_gpu_stages(text: &str) -> GpuStages {
    let text = last_boot_text(text);
    let lines = line_lc(text);
    let gpu_backend = any_line(&lines, |l| {
        l.contains("pool vram=") && !l.contains("pool vram=no") && !l.contains("vram=no")
    }) || any_line(&lines, |l| l.contains("matvec") && l.contains("gpu"));
    GpuStages {
        gsp_rpc: if any_line(&lines, |l| {
            l.contains("gsp=fallo") || l.contains("gsp fallo") || l.contains("gsp: fallo")
        }) {
            StageStatus::fail("GSP fallido")
        } else if any_line(&lines, |l| {
            l.contains("gsp_init_done") || l.contains("gsp-rm listo")
        }) {
            StageStatus::ok(Some("GSP_INIT_DONE"))
        } else {
            StageStatus::pendiente()
        },
        vram_pool: if any_line(&lines, |l| l.contains("pool vram=no") || l.contains("vram=no")) {
            StageStatus::fail("pool VRAM=no")
        } else if any_line(&lines, |l| {
            l.contains("pool vram=") && !l.contains("pool vram=no")
        }) {
            StageStatus::ok(None)
        } else {
            StageStatus::pendiente()
        },
        ce_readback: if any_line(&lines, |l| l.contains("readback") && l.contains("fallo")) {
            StageStatus::fail("CE readback fallido")
        } else if any_line(&lines, |l| l.contains("readback") && l.contains("go")) {
            StageStatus::ok(Some("CE readback GO"))
        } else {
            StageStatus::pendiente()
        },
        compute_cpu_gpu: if any_line(&lines, |l| {
            l.contains("dispositivo «soft") || l.contains("dispositivo \"soft")
        }) {
            StageStatus::no_aplica("dispositivo software (QEMU)")
        } else if any_line(&lines, |l| l.contains("matvec") && l.contains("gpu") && l.contains("tok/s"))
        {
            StageStatus::ok(None)
        } else {
            StageStatus::pendiente()
        },
        carga_real: if any_line(&lines, |l| l.contains("soso-llm: generado")) && gpu_backend {
            StageStatus::ok(None)
        } else {
            StageStatus::pendiente()
        },
        apagado_limpio: if any_line(&lines, |l| l.contains("unload=fallo")) {
            StageStatus::fail("unload=fallo")
        } else if any_line(&lines, |l| l.contains("unload=ok"))
            && any_line(&lines, |l| l.contains("dma=off") || l.contains("bus master quitado"))
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
    entry.bench.seed = flag(args, "--seed")
        .and_then(|s| s.parse().ok())
        .or(Some(42));
    entry.bench.max_tokens = flag(args, "--max").and_then(|s| s.parse().ok()).or(Some(8));
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
    println!(
        "hw-matrix v{} — {} entradas\n",
        m.schema_version,
        m.entries.len()
    );
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
    print_stages(
        "  wifi",
        &[
            ("alive", &e.wifi.alive),
            ("init", &e.wifi.init_complete),
            ("mvm", &e.wifi.mvm_ready),
            ("scan", &e.wifi.scan),
            ("wpa2", &e.wifi.assoc_wpa2),
            ("dhcp", &e.wifi.dhcp),
            ("ssh", &e.wifi.ssh),
            ("recon", &e.wifi.reconexion),
        ],
    );
    print_stages(
        "  gpu ",
        &[
            ("gsp", &e.gpu.gsp_rpc),
            ("vram", &e.gpu.vram_pool),
            ("ce", &e.gpu.ce_readback),
            ("cpu/gpu", &e.gpu.compute_cpu_gpu),
            ("carga", &e.gpu.carga_real),
            ("apagado", &e.gpu.apagado_limpio),
        ],
    );
    if let Some(t) = e.bench.tok_s_mediana {
        println!(
            "  bench: {t:.2} tok/s ({})",
            e.bench.backend.as_deref().unwrap_or("?")
        );
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

/// PCI ID concreto `vvvv:dddd` en hex. Rechaza huecos tipo `10de:????`.
pub(crate) fn pci_id_concreto(id: &str) -> bool {
    let id = id.trim();
    let Some((vid, did)) = id.split_once(':') else {
        return false;
    };
    vid.len() == 4
        && did.len() == 4
        && vid.chars().all(|c| c.is_ascii_hexdigit())
        && did.chars().all(|c| c.is_ascii_hexdigit())
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
        let w = parse_wifi_stages("iwl: UCODE_ALIVE_NTFY net: dhcp 192.168.1.2/24 gw 192.168.1.1");
        assert_eq!(w.alive.status, "ok");
        assert_eq!(w.dhcp.status, "ok");
    }

    #[test]
    fn parse_gpu_shutdown() {
        let g = parse_gpu_stages("nouveau-lx: unload=ok halt=ok dma=off readback GO");
        assert_eq!(g.ce_readback.status, "ok");
        assert_eq!(g.apagado_limpio.status, "ok");
    }

    #[test]
    fn parse_gpu_rejects_false_positives() {
        let g = parse_gpu_stages("pool VRAM=no soso-llm run dispositivo «soft»");
        assert_eq!(g.vram_pool.status, "fail");
        assert_eq!(g.carga_real.status, "pendiente");
        assert_eq!(g.compute_cpu_gpu.status, "no_aplica");
        let g = parse_gpu_stages("unload=fallo dma=off");
        assert_eq!(g.apagado_limpio.status, "fail");
        let g = parse_gpu_stages("soso-llm: generado 16 tokens");
        assert_eq!(g.carga_real.status, "pendiente");
    }

    #[test]
    fn parse_c4_false_positives_and_boots() {
        let g = parse_gpu_stages("GSP firmware cargado\nmemtest OK\nnvkm device graph OK — subdev='gsp0'");
        assert_eq!(g.gsp_rpc.status, "pendiente");
        let g = parse_gpu_stages("VRAM 4096 MiB detectada");
        assert_eq!(g.vram_pool.status, "pendiente");
        let g = parse_gpu_stages("GSP_INIT_DONE recibido\npool VRAM=256MiB\nsoso-llm: generado 8 tokens");
        assert_eq!(g.gsp_rpc.status, "ok");
        assert_eq!(g.vram_pool.status, "ok");
        assert_eq!(g.carga_real.status, "ok");

        let w = parse_wifi_stages("wifi: no asociado\nnet: dhcp 192.168.1.10/24");
        assert_eq!(w.assoc_wpa2.status, "fail");
        assert_eq!(w.dhcp.status, "pendiente");

        let two = "\
boot: memtest\niwl: UCODE_ALIVE_NTFY\nINIT_COMPLETE_NOTIF\nup mínimo listo\n\
boot: memtest\niwl: start\n";
        let runs = parse_log_runs(two);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].wifi.alive.status, "ok");
        assert_eq!(runs[0].wifi.init_complete.status, "ok");
        assert_eq!(runs[0].wifi.mvm_ready.status, "ok");
        assert_eq!(runs[1].wifi.alive.status, "pendiente");
        let current = parse_wifi_stages(two);
        assert_eq!(current.alive.status, "pendiente");
        assert_ne!(runs[0].log_hash, runs[1].log_hash);
    }

    #[test]
    fn parse_truncated_after_green_is_current_pending() {
        let blob = "\
boot: memtest\nUCODE_ALIVE_NTFY\nINIT_COMPLETE_NOTIF\nup mínimo listo\nscan fin end=1 count=0\n\
boot: memtest\niwl: start\n";
        let w = parse_wifi_stages(blob);
        assert_eq!(w.alive.status, "pendiente");
        assert_eq!(w.scan.status, "pendiente");
        let hist = parse_log_runs(blob);
        assert_eq!(hist[0].wifi.scan.status, "ok");
    }

    #[test]
    fn parse_wifi_scan_empty_is_fail() {
        let w = parse_wifi_stages("wifi: ninguna red UCODE_ALIVE_NTFY");
        assert_eq!(w.alive.status, "ok");
        assert_eq!(w.scan.status, "fail");
    }

    #[test]
    fn parse_wifi_dhcp_ignores_gsp_fallo() {
        let w = parse_wifi_stages("GSP=fallo pool VRAM=no dhcp fallo");
        assert_eq!(w.dhcp.status, "pendiente");
    }

    #[test]
    fn parse_wifi_rejects_banner_and_words() {
        let w = parse_wifi_stages("sosh — escribe 'help' wpa2 reconnect wifi scan");
        assert_eq!(w.ssh.status, "pendiente");
        assert_eq!(w.assoc_wpa2.status, "pendiente");
        assert_eq!(w.reconexion.status, "pendiente");
        assert_eq!(w.scan.status, "pendiente");
        let w = parse_wifi_stages(
            "wifi scan: ssid Casa\nassoc=ok\nssh: sesión ok\nreconexion ok",
        );
        assert_eq!(w.scan.status, "ok");
        assert_eq!(w.assoc_wpa2.status, "ok");
        assert_eq!(w.ssh.status, "ok");
        assert_eq!(w.reconexion.status, "ok");
    }

    #[test]
    fn parse_truncated_log_stays_pending() {
        let g = parse_gpu_stages("gsp\nvram\n");
        assert_eq!(g.gsp_rpc.status, "pendiente");
        assert_eq!(g.vram_pool.status, "pendiente");
        let w = parse_wifi_stages("iwl: start");
        assert_eq!(w.alive.status, "pendiente");
    }

    #[test]
    fn merge_stage_keeps_ok_when_next_boot_is_truncated() {
        let prev = StageStatus::ok(Some("UCODE_ALIVE_NTFY"));
        let incoming = parse_wifi_stages("iwl: start").alive;
        assert_eq!(incoming.status, "pendiente");
        let merged = merge_stage(&prev, incoming);
        assert_eq!(merged.status, "ok");
        assert_eq!(merged.nota.as_deref(), Some("UCODE_ALIVE_NTFY"));
    }

    #[test]
    fn merge_stage_records_new_fail() {
        let prev = StageStatus::ok(Some("shell"));
        let incoming = StageStatus::fail("ALIVE degradado o fallido");
        let merged = merge_stage(&prev, incoming);
        assert_eq!(merged.status, "fail");
    }

    #[test]
    fn firmware_files_for_known_boards() {
        assert_eq!(firmware_files_for("ax211-wifi")[0].1, "ax211_ucode");
        assert_eq!(firmware_files_for("qemu-test").len(), 0);
        assert_eq!(firmware_files_for("ga107-igpu")[0].1, "gsp_ga102_fallback");
    }

    #[test]
    fn pci_id_rejects_placeholder() {
        assert!(!pci_id_concreto("10de:????"));
        assert!(!pci_id_concreto("????:249c"));
        assert!(!pci_id_concreto("10de:249"));
        assert!(!pci_id_concreto("10de249c"));
        assert!(pci_id_concreto("10de:249c"));
        assert!(pci_id_concreto("8086:7f70"));
    }

    #[test]
    fn default_and_committed_matrix_have_no_placeholder_pci() {
        let seed = default_matrix();
        for e in &seed.entries {
            for pci in &e.pci_ids {
                assert!(
                    pci_id_concreto(pci),
                    "semilla {}: PCI provisional {pci}",
                    e.id
                );
            }
        }
        let ga = seed
            .entries
            .iter()
            .find(|e| e.id == "ga107-igpu")
            .expect("ga107-igpu");
        assert_eq!(ga.pci_ids, vec!["10de:249c".to_string()]);

        let json = include_str!("../../docs/hw-matrix.json");
        let committed: HwMatrix = serde_json::from_str(json).expect("hw-matrix.json");
        for e in &committed.entries {
            for pci in &e.pci_ids {
                assert!(pci_id_concreto(pci), "{}: PCI provisional {pci}", e.id);
            }
        }
        let ga = committed
            .entries
            .iter()
            .find(|e| e.id == "ga107-igpu")
            .expect("ga107-igpu");
        assert_eq!(ga.pci_ids, vec!["10de:249c".to_string()]);
    }

    #[test]
    fn merge_wifi_truncated_does_not_wipe_scan() {
        let prev = WifiStages {
            scan: StageStatus::ok(Some("ssid")),
            ..WifiStages::default()
        };
        let incoming = parse_wifi_stages("sosh — wpa2 reconnect");
        let merged = merge_wifi(&prev, incoming);
        assert_eq!(merged.scan.status, "ok");
        assert_eq!(merged.assoc_wpa2.status, "pendiente");
    }
}
