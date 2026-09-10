//! `cargo xtask hw-matrix` — matriz versionada de validación por equipo (A8).

use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::drivers::{parse_hwscan_file, ScanLine};

const MATRIX_PATH: &str = "docs/hw-matrix.json";
/// v2: identidad de arranque por banner, artefactos y campañas separadas (R2).
const SCHEMA_VERSION: u32 = 2;

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
    /// Hashes de lo realmente desplegado: kernel, binarios del rootfs, perfil.
    #[serde(default)]
    pub artefactos: std::collections::BTreeMap<String, String>,
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
///
/// `log_hash` identifica los bytes del arranque (integridad del fichero); la
/// identidad de *lo desplegado* son `kernel` + `artefactos`. Sin banner de
/// versión la identidad es explícitamente desconocida.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RunEvidence {
    pub log_hash: String,
    #[serde(default)]
    pub kernel: String,
    #[serde(default)]
    pub identidad_conocida: bool,
    #[serde(default)]
    pub artefactos: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub pci: Vec<String>,
    #[serde(default)]
    pub wifi: WifiStages,
    #[serde(default)]
    pub gpu: GpuStages,
    /// Llegó a userspace (init/sosh vivos, sin panic).
    #[serde(default)]
    pub userspace: bool,
    /// Superó la campaña WiFi completa (ALIVE→scan→assoc→DHCP atribuido).
    #[serde(default)]
    pub campana_wifi: bool,
    /// Superó la campaña GPU completa (GSP→VRAM→CE→compute comparado).
    #[serde(default)]
    pub campana_gpu: bool,
    /// Instantáneas del log dentro de este mismo arranque.
    #[serde(default)]
    pub flushes: u32,
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
        "artefactos" => record_artefactos(&args[1..]),
        "migrar" => migrar(&args[1..]),
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
                                            [--sosohash SOSOHASH.TXT]\n\
         cargo xtask hw-matrix migrar [--id ID] [--seco]\n\
         cargo xtask hw-matrix artefactos --id ID [--kernel F] [--rootfs D] [--bin F] [--perfil P]\n\
         cargo xtask hw-matrix record-boot --id ID [--fail] [--nota T]\n\
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
        schema_version: SCHEMA_VERSION,
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
        artefactos: std::collections::BTreeMap::new(),
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

/// Hash agregado de un directorio: cada fichero aporta ruta + sha256, en
/// orden estable, y el resultado se vuelve a hashear. Detecta cambios de
/// contenido, altas y bajas.
pub fn hash_directorio(dir: &Path) -> Option<(String, usize)> {
    let mut ficheros: Vec<PathBuf> = Vec::new();
    recorrer(dir, &mut ficheros)?;
    ficheros.sort();
    let mut acc = String::new();
    for f in &ficheros {
        let rel = f.strip_prefix(dir).unwrap_or(f);
        let bytes = std::fs::read(f).ok()?;
        acc.push_str(&format!("{} {}\n", rel.display(), sha256_hex(&bytes)));
    }
    Some((sha256_hex(acc.as_bytes()), ficheros.len()))
}

fn recorrer(dir: &Path, out: &mut Vec<PathBuf>) -> Option<()> {
    for e in std::fs::read_dir(dir).ok()? {
        let e = e.ok()?;
        let p = e.path();
        if p.is_dir() {
            recorrer(&p, out)?;
        } else if p.is_file() {
            out.push(p);
        }
    }
    Some(())
}

/// `hw-matrix artefactos`: registra la identidad de lo desplegado.
///
/// El hash del log acredita la integridad del fichero, no qué binarios
/// corrieron. Estos hashes son lo que permite decir «esta ejecución es de
/// este kernel y este rootfs» en vez de deducirlo de la versión anunciada.
fn record_artefactos(args: &[String]) {
    let id = flag(args, "--id").unwrap_or_else(|| {
        eprintln!("hw-matrix artefactos: falta --id");
        std::process::exit(2);
    });
    let root = crate::project_root();
    let mut m = load_matrix();
    if !m.entries.iter().any(|e| e.id == id) {
        eprintln!("hw-matrix artefactos: entrada {id} desconocida; ejecuta collect primero");
        std::process::exit(2);
    }
    let entry = find_entry(&mut m, &id).expect("entrada");
    let mut registrados = 0u32;

    let kernel = flag_path(args, "--kernel").unwrap_or_else(|| {
        root.join("target/kernel/x86_64-soso/debug/kernel")
    });
    if kernel.is_file() {
        if let Ok(bytes) = std::fs::read(&kernel) {
            entry.artefactos.insert(
                "kernel".into(),
                format!("{} {}B {}", sha256_hex(&bytes), bytes.len(),
                        kernel.file_name().unwrap_or_default().to_string_lossy()),
            );
            registrados += 1;
        }
    } else {
        eprintln!("hw-matrix artefactos: sin kernel en {}", kernel.display());
    }

    for bin in flags(args, "--bin") {
        let p = PathBuf::from(&bin);
        if let Ok(bytes) = std::fs::read(&p) {
            let nombre = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
            entry.artefactos.insert(
                format!("bin:{nombre}"),
                format!("{} {}B", sha256_hex(&bytes), bytes.len()),
            );
            registrados += 1;
        } else {
            eprintln!("hw-matrix artefactos: no pude leer {bin}");
        }
    }

    let rootfs = flag_path(args, "--rootfs").unwrap_or_else(|| root.join("rootfs"));
    if rootfs.is_dir() {
        if let Some((hash, n)) = hash_directorio(&rootfs) {
            entry
                .artefactos
                .insert("rootfs".into(), format!("{hash} {n} ficheros"));
            registrados += 1;
        }
    }

    if let Some(perfil) = flag(args, "--perfil") {
        entry.perfil_drivers = perfil.clone();
        entry.artefactos.insert("perfil".into(), perfil);
        registrados += 1;
    } else if !entry.perfil_drivers.is_empty() {
        entry
            .artefactos
            .insert("perfil".into(), entry.perfil_drivers.clone());
    }

    collect_firmware_hashes(entry);
    for (k, v) in entry.firmware.clone() {
        entry.artefactos.insert(format!("fw:{k}"), v);
    }
    entry.actualizado = today();
    let resumen = entry.artefactos.clone();
    save_matrix(&m);
    println!("hw-matrix: {registrados} artefacto(s) registrados en {id}");
    for (k, v) in resumen {
        println!("  {k} = {v}");
    }
}

/// `hw-matrix migrar`: rehace el historial desde los logs originales.
///
/// Las entradas importadas con el parser anterior tienen una ejecución por la
/// cabecera y otra por el cuerpo del mismo arranque, y como «kernel» la línea
/// `updslot: SOSOKRN.BIN…`. Se vuelven a derivar de los ficheros que siguen
/// existiendo; las ejecuciones cuyo log ya no está se conservan con la
/// identidad marcada como desconocida, para no perder la traza.
fn migrar(args: &[String]) {
    let solo = flag(args, "--id");
    let seco = args.iter().any(|a| a == "--seco");
    let mut m = load_matrix();
    let mut total_antes = 0usize;
    let mut total_despues = 0usize;

    for entry in m.entries.iter_mut() {
        if let Some(id) = &solo {
            if &entry.id != id {
                continue;
            }
        }
        let antes = entry.historial.len();
        total_antes += antes;
        let grupos = agrupar_logs(&entry.logs);
        let mut nuevo: Vec<RunEvidence> = Vec::new();
        for (logs, informes) in &grupos {
            let logs: Vec<(String, String)> = logs
                .iter()
                .filter_map(|(et, p)| {
                    std::fs::read_to_string(p).ok().map(|t| (et.clone(), t))
                })
                .collect();
            let informes: Vec<(String, String)> = informes
                .iter()
                .filter_map(|(et, p)| {
                    std::fs::read_to_string(p).ok().map(|t| (et.clone(), t))
                })
                .collect();
            if logs.is_empty() && informes.is_empty() {
                continue;
            }
            let (runs, avisos) = correlacionar_fuentes(&logs, &informes, &entry.artefactos);
            for a in &avisos {
                eprintln!("hw-matrix migrar [{}]: {a}", entry.id);
            }
            for r in &runs {
                anadir_o_fusionar(&mut nuevo, r);
            }
        }
        // Ejecuciones sin log recuperable: se mantienen, sin identidad falsa.
        let heredadas: Vec<RunEvidence> = entry
            .historial
            .iter()
            .filter(|h| !nuevo.iter().any(|n| n.log_hash == h.log_hash))
            .filter(|h| banner_version(&h.kernel).is_some())
            .map(|h| {
                let mut h = h.clone();
                h.identidad_conocida = banner_version(&h.kernel).is_some();
                h
            })
            .collect();
        if grupos.is_empty() {
            nuevo = heredadas;
        }
        entry.historial = nuevo;
        recalcular_racha(entry);
        if let Some(last) = entry.historial.last().cloned() {
            entry.wifi = last.wifi.clone();
            entry.gpu = last.gpu.clone();
        }
        total_despues += entry.historial.len();
        if antes != entry.historial.len() {
            entry.notas = format!(
                "{} ; historial remigrado {}: {antes}→{} ejecuciones (identidad por banner)",
                entry.notas.trim_end_matches(' '),
                today(),
                entry.historial.len()
            )
            .trim_start_matches(" ; ")
            .to_string();
            entry.actualizado = today();
        }
        println!(
            "hw-matrix migrar: {} {antes} → {} ejecuciones (racha={})",
            entry.id,
            entry.historial.len(),
            entry.arranques_consecutivos_ok
        );
    }
    if seco {
        println!("hw-matrix migrar: --seco, no se escribe ({total_antes} → {total_despues})");
        return;
    }
    m.schema_version = SCHEMA_VERSION;
    save_matrix(&m);
    println!("hw-matrix migrar: {total_antes} → {total_despues} ejecuciones; schema v{SCHEMA_VERSION}");
}

/// Agrupa las rutas registradas por directorio: cada carpeta de diagnóstico
/// es una sesión con su SOSOLOG y sus informes.
fn agrupar_logs(
    logs: &[String],
) -> Vec<(Vec<(String, PathBuf)>, Vec<(String, PathBuf)>)> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for l in logs {
        let p = PathBuf::from(l);
        let dir = p.parent().map(|d| d.to_path_buf()).unwrap_or_default();
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    let mut out = Vec::new();
    for dir in dirs {
        let mut arranque = Vec::new();
        let mut informe = Vec::new();
        for l in logs {
            let p = PathBuf::from(l);
            if p.parent().map(|d| d.to_path_buf()).unwrap_or_default() != dir || !p.is_file() {
                continue;
            }
            let nombre = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_ascii_uppercase();
            if nombre.starts_with("SOSOLOG") {
                arranque.push(("SOSOLOG".to_string(), p));
            } else if nombre.contains("SERIAL") {
                arranque.push(("serial".to_string(), p));
            } else if nombre.starts_with("SOSODRV") {
                informe.push(("SOSODRV".to_string(), p));
            } else if nombre.starts_with("SOSOWIFI") {
                informe.push(("SOSOWIFI".to_string(), p));
            }
        }
        if !arranque.is_empty() || !informe.is_empty() {
            out.push((arranque, informe));
        }
    }
    out
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
    // Logs de arranque frente a informes: solo los primeros definen
    // ejecuciones. Antes iban todos a un único blob y cada sección con banner
    // se convertía en un arranque más.
    let mut logs_arranque: Vec<(String, String)> = Vec::new();
    let mut informes: Vec<(String, String)> = Vec::new();
    let mut log_paths: Vec<PathBuf> = Vec::new();
    for (flag, label, es_log) in [
        ("--sosolog", "SOSOLOG", true),
        ("--serial", "serial", true),
        ("--sosodrv", "SOSODRV", false),
        ("--sosowifi", "SOSOWIFI", false),
    ] {
        if let Some(p) = flag_path(args, flag) {
            match std::fs::read_to_string(&p) {
                Ok(t) => {
                    if es_log {
                        logs_arranque.push((label.into(), t));
                    } else {
                        informes.push((label.into(), t));
                    }
                    log_paths.push(p);
                }
                Err(e) => {
                    eprintln!("hw-matrix parse-logs: no pude leer {}: {e}", p.display());
                    std::process::exit(2);
                }
            }
        }
    }
    if logs_arranque.iter().all(|(_, t)| t.trim().is_empty())
        && informes.iter().all(|(_, t)| t.trim().is_empty())
    {
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
    // R3: si el USB trae su manifiesto, la identidad de la ejecución sale de
    // ahí (lo que se flasheó) y no de lo que haya en el árbol ahora.
    let mut artefactos = entry.artefactos.clone();
    if let Some(p) = flag_path(args, "--sosohash") {
        match leer_manifiesto_esp(&p) {
            Ok(m) => {
                for (k, v) in m {
                    artefactos.insert(k, v);
                }
                println!("hw-matrix parse-logs: artefactos desde {}", p.display());
            }
            Err(e) => {
                eprintln!("hw-matrix parse-logs: {} ilegible: {e}", p.display());
                std::process::exit(2);
            }
        }
    }
    let (runs, avisos) = correlacionar_fuentes(&logs_arranque, &informes, &artefactos);
    for aviso in &avisos {
        eprintln!("hw-matrix parse-logs: {aviso}");
    }
    for run in &runs {
        anadir_o_fusionar(&mut entry.historial, run);
    }
    if let Some(last) = runs.last() {
        let last = entry
            .historial
            .iter()
            .find(|h| h.log_hash == last.log_hash)
            .cloned()
            .unwrap_or_else(|| last.clone());
        entry.wifi = last.wifi.clone();
        entry.gpu = last.gpu.clone();
        entry.sesion_sostenida = if last.userspace {
            parse_sesion(&last_boot_de_fuentes(&logs_arranque))
        } else {
            StageStatus::pendiente()
        };
        if !last.identidad_conocida {
            eprintln!(
                "hw-matrix parse-logs: el último arranque no trae banner de versión;                  identidad desconocida"
            );
        }
    }
    // Contadores derivados del historial, no incrementales.
    recalcular_racha(entry);
    if let Some(drv) = flag_path(args, "--sosodrv") {
        apply_hwscan(entry, &drv);
    }
    entry.actualizado = today();
    let summary = entry.clone();
    save_matrix(&m);
    println!("hw-matrix: etapas parseadas para {id}");
    show_entry_summary(&summary);
}

/// Lee `SOSOHASH.TXT` de la ESP: `clave=valor` por línea, `#` es comentario.
///
/// Las claves de artefacto (kernel/esp/rootfs/modelos) llevan `hash bytes`; el
/// resto (version/build/perfil/features/fecha) se guardan como texto para
/// poder decir con qué se construyó.
pub fn leer_manifiesto_esp(
    path: &Path,
) -> Result<std::collections::BTreeMap<String, String>, String> {
    let texto = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut out = std::collections::BTreeMap::new();
    for linea in texto.lines() {
        let l = linea.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let Some((k, v)) = l.split_once('=') else {
            continue;
        };
        let (k, v) = (k.trim(), v.trim());
        if k.is_empty() || v.is_empty() || v == "desconocido" {
            continue;
        }
        out.insert(k.to_string(), v.to_string());
    }
    if !out.contains_key("kernel") && !out.contains_key("version") {
        return Err("no parece un SOSOHASH.TXT (sin kernel ni version)".into());
    }
    Ok(out)
}

fn last_boot_de_fuentes(logs: &[(String, String)]) -> String {
    logs.first()
        .map(|(_, t)| last_boot_text(t).to_string())
        .unwrap_or_default()
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

/// Reconoce el banner de versión que imprime el kernel al arrancar
/// (`kernel/src/main.rs`: `soso {version} ({build})`).
///
/// Es la **identidad del arranque**. Cualquier otra mención a SOSOKRN —
/// `updslot: SOSOKRN.BIN LBA 417`, por ejemplo — habla del slot de la ESP, no
/// de lo que se está ejecutando.
pub fn banner_version(line: &str) -> Option<&str> {
    let l = line.trim();
    let resto = l.strip_prefix("soso ")?;
    let (ver, resto) = resto.split_once(' ')?;
    if ver.is_empty() || !ver.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    if !ver
        .chars()
        .all(|c| c.is_ascii_digit() || c == '.' || c == '-' || c.is_ascii_alphabetic())
    {
        return None;
    }
    if !resto.starts_with('(') || !resto.trim_end().ends_with(')') {
        return None;
    }
    Some(l)
}

/// Marcador de instantánea del log en la ESP: no es un arranque nuevo.
fn es_flush(line: &str) -> bool {
    let l = line.trim();
    l.starts_with("=== soso log flush") || l.starts_with("=== soso log")
}

fn es_memtest(line: &str) -> bool {
    line.contains("boot: memtest")
}

/// Offsets donde empieza cada arranque.
///
/// Un arranque empieza en su banner de versión; si no hay banner, en
/// `boot: memtest`. Así la cabecera y el cuerpo del mismo arranque quedan
/// juntos: antes el corte en `boot: memtest` dejaba la versión —seis líneas
/// más arriba en run15— en una «ejecución» aparte.
fn inicios_de_arranque(text: &str) -> Vec<usize> {
    let mut inicios: Vec<usize> = Vec::new();
    let mut memtest_en_curso = false;
    let mut off = 0usize;
    for line in text.split_inclusive('\n') {
        if banner_version(line).is_some() {
            inicios.push(off);
            memtest_en_curso = false;
        } else if es_memtest(line) {
            if inicios.is_empty() || memtest_en_curso {
                inicios.push(off);
            }
            memtest_en_curso = true;
        }
        off += line.len();
    }
    inicios
}

/// Parte un log concatenado en arranques. El texto anterior al primer inicio
/// es la cola de un arranque anterior sin cabecera: no se presenta como
/// ejecución propia (era el origen de las entradas duplicadas).
pub fn split_boots(text: &str) -> Vec<&str> {
    let inicios = inicios_de_arranque(text);
    if inicios.is_empty() {
        return vec![text];
    }
    let mut out = Vec::new();
    for (i, start) in inicios.iter().enumerate() {
        let mut start = *start;
        // Una cabecera de instantánea (y líneas en blanco) delante del banner
        // es del mismo arranque: el volcado de la ESP escribe el marcador y
        // después el buffer. Solo se descarta la cola con contenido real.
        if i == 0 && start > 0 && solo_cabeceras(&text[..start]) {
            start = 0;
        }
        let end = inicios.get(i + 1).copied().unwrap_or(text.len());
        out.push(&text[start..end]);
    }
    out
}

fn solo_cabeceras(prefijo: &str) -> bool {
    prefijo
        .lines()
        .all(|l| l.trim().is_empty() || es_flush(l))
}

pub fn last_boot_text(text: &str) -> &str {
    split_boots(text).last().copied().unwrap_or(text)
}

/// Identidad del arranque: banner de versión, o `None` si el log no la trae.
fn extract_kernel_id(text: &str) -> Option<String> {
    for line in text.lines() {
        if let Some(b) = banner_version(line) {
            return Some(b.chars().take(80).collect());
        }
        let l = line.trim();
        if let Some(v) = l.strip_prefix("version=") {
            if !v.is_empty() {
                return Some(l.chars().take(80).collect());
            }
        }
    }
    None
}

fn contar_flushes(text: &str) -> u32 {
    text.lines().filter(|l| es_flush(l)).count() as u32
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

#[allow(dead_code)]
pub fn parse_log_runs(text: &str) -> Vec<RunEvidence> {
    parse_log_runs_con_artefactos(text, &std::collections::BTreeMap::new())
}

/// Cada arranque del log se convierte en una ejecución, con la identidad de
/// los artefactos desplegados que se conozca en ese momento.
pub fn parse_log_runs_con_artefactos(
    text: &str,
    artefactos: &std::collections::BTreeMap<String, String>,
) -> Vec<RunEvidence> {
    split_boots(text)
        .into_iter()
        .map(|boot| {
            let ident = extract_kernel_id(boot);
            let wifi = parse_wifi_stages(boot);
            let gpu = parse_gpu_stages(boot);
            RunEvidence {
                log_hash: sha256_hex(boot.as_bytes()),
                identidad_conocida: ident.is_some(),
                kernel: ident.unwrap_or_else(|| "desconocida".into()),
                artefactos: artefactos.clone(),
                pci: extract_pci(boot),
                userspace: boot_ok_criterio(boot),
                campana_wifi: campana_wifi_ok(&wifi),
                campana_gpu: campana_gpu_ok(&gpu),
                flushes: contar_flushes(boot),
                wifi,
                gpu,
                fecha: today(),
            }
        })
        .collect()
}

/// Fusiona en `dst` lo que aporte `src` del **mismo** arranque: una etapa
/// pendiente nunca borra una acreditada, y la identidad conocida gana.
pub fn fusionar_run(dst: &mut RunEvidence, src: &RunEvidence) {
    dst.wifi = merge_wifi(&dst.wifi, src.wifi.clone());
    dst.gpu = merge_gpu(&dst.gpu, src.gpu.clone());
    if !dst.identidad_conocida && src.identidad_conocida {
        dst.kernel = src.kernel.clone();
        dst.identidad_conocida = true;
    }
    for pci in &src.pci {
        if !dst.pci.contains(pci) {
            dst.pci.push(pci.clone());
        }
    }
    for (k, v) in &src.artefactos {
        dst.artefactos.entry(k.clone()).or_insert_with(|| v.clone());
    }
    dst.userspace |= src.userspace;
    dst.flushes = dst.flushes.max(src.flushes);
    // Las campañas se recalculan sobre las etapas ya fusionadas.
    dst.campana_wifi = campana_wifi_ok(&dst.wifi);
    dst.campana_gpu = campana_gpu_ok(&dst.gpu);
}

/// Añade la ejecución al historial o la fusiona con la que ya tenga su hash.
/// Reimportar no crea entradas nuevas ni mueve contadores.
fn anadir_o_fusionar(historial: &mut Vec<RunEvidence>, run: &RunEvidence) {
    if let Some(prev) = historial.iter_mut().find(|h| h.log_hash == run.log_hash) {
        fusionar_run(prev, run);
        return;
    }
    historial.push(run.clone());
}

/// Correlaciona las fuentes de una importación en ejecuciones.
///
/// SOSOLOG y el log de serie son *logs de arranque*: se parten en arranques y
/// se emparejan por posición cuando traen el mismo número de arranques con
/// identidades compatibles (son el mismo arranque visto por otro canal).
/// SOSODRV/SOSOWIFI son *informes* del último arranque: no son ejecuciones
/// nuevas, se fusionan en el último.
pub fn correlacionar_fuentes(
    logs_arranque: &[(String, String)],
    informes: &[(String, String)],
    artefactos: &std::collections::BTreeMap<String, String>,
) -> (Vec<RunEvidence>, Vec<String>) {
    let mut runs: Vec<RunEvidence> = Vec::new();
    let mut avisos: Vec<String> = Vec::new();

    for (etiqueta, texto) in logs_arranque {
        let nuevos = parse_log_runs_con_artefactos(texto, artefactos);
        if runs.is_empty() {
            runs = nuevos;
            continue;
        }
        let emparejable = nuevos.len() == runs.len()
            && runs.iter().zip(nuevos.iter()).all(|(a, b)| {
                !a.identidad_conocida || !b.identidad_conocida || a.kernel == b.kernel
            });
        if emparejable {
            for (dst, src) in runs.iter_mut().zip(nuevos.iter()) {
                fusionar_run(dst, src);
            }
        } else {
            avisos.push(format!(
                "{etiqueta}: {} arranque(s) no correlacionables con los {} de la fuente previa; \
                 se registran aparte",
                nuevos.len(),
                runs.len()
            ));
            for r in &nuevos {
                anadir_o_fusionar(&mut runs, r);
            }
        }
    }

    if runs.is_empty() && !informes.is_empty() {
        runs.push(RunEvidence {
            log_hash: sha256_hex(informes[0].1.as_bytes()),
            kernel: "desconocida".into(),
            identidad_conocida: false,
            artefactos: artefactos.clone(),
            fecha: today(),
            ..Default::default()
        });
    }
    for (etiqueta, texto) in informes {
        let Some(ultimo) = runs.last_mut() else {
            continue;
        };
        let informe = RunEvidence {
            log_hash: ultimo.log_hash.clone(),
            kernel: String::new(),
            identidad_conocida: false,
            wifi: parse_wifi_stages(texto),
            gpu: parse_gpu_stages(texto),
            pci: extract_pci(texto),
            fecha: today(),
            ..Default::default()
        };
        let _ = etiqueta;
        fusionar_run(ultimo, &informe);
    }
    (runs, avisos)
}

/// «Llegó a userspace» ≠ «superó la campaña»: son criterios distintos y se
/// registran por separado.
fn boot_ok_criterio(text: &str) -> bool {
    let lines = line_lc(text);
    any_line(&lines, |l| l.contains("boot: task") || l.contains("sosh —"))
        && !any_line(&lines, |l| l.contains("panic") || l.contains("double fault"))
}

fn campana_wifi_ok(w: &WifiStages) -> bool {
    ["ok"].contains(&w.alive.status.as_str())
        && w.init_complete.status == "ok"
        && w.mvm_ready.status == "ok"
        && w.scan.status == "ok"
        && w.assoc_wpa2.status == "ok"
        && w.dhcp.status == "ok"
}

fn campana_gpu_ok(g: &GpuStages) -> bool {
    g.gsp_rpc.status == "ok"
        && g.vram_pool.status == "ok"
        && g.ce_readback.status == "ok"
        && g.compute_cpu_gpu.status == "ok"
}

/// Racha derivada del historial ordenado: cuenta arranques únicos con
/// userspace al final de la lista. Reimportar el mismo log no la mueve porque
/// no añade ejecuciones nuevas.
fn recalcular_racha(entry: &mut HwEntry) {
    let mut racha: u8 = 0;
    for run in entry.historial.iter().rev() {
        if run.userspace {
            racha = racha.saturating_add(1);
        } else {
            break;
        }
    }
    entry.arranques_consecutivos_ok = racha;
}

/// Atribución por interfaz: una etapa WiFi necesita una línea de WiFi.
fn es_iface_wifi(l: &str) -> bool {
    l.contains("wifi") || l.contains("lxwifi") || l.contains("lx-wifi") || l.contains("iwl")
}

fn es_iface_ethernet(l: &str) -> bool {
    l.contains("eth") || l.contains("e1000") || l.contains("cable")
}

fn tiene_ip_privada(l: &str) -> bool {
    l.contains("192.") || l.contains("10.") || l.contains("172.")
}

fn iface_de_linea(l: &str) -> String {
    for tok in ["lxwifi", "lx-wifi", "iwlwifi", "iwl", "wifi"] {
        if l.contains(tok) {
            return tok.into();
        }
    }
    "?".into()
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
                    && es_iface_wifi(l)
                    && !l.contains("gsp")
            });
            // Un lease atribuido explícitamente al WiFi acredita WiFi. Con el
            // enlace arriba se acepta un lease sin interfaz nombrada, pero
            // nunca uno que diga Ethernet: eso es la otra interfaz.
            let explicito = lines.iter().find(|l| {
                l.contains("net: dhcp ") && tiene_ip_privada(l) && es_iface_wifi(l)
            });
            let implicito = wifi_link_up(&lines)
                && any_line(&lines, |l| {
                    l.contains("net: dhcp ") && tiene_ip_privada(l) && !es_iface_ethernet(l)
                });
            if wifi_dhcp_fail {
                StageStatus::fail("DHCP fallido")
            } else if let Some(l) = explicito {
                StageStatus::ok(Some(&format!("lease en {}", iface_de_linea(l))))
            } else if implicito {
                StageStatus::ok(Some("lease con enlace WiFi arriba (interfaz no nombrada)"))
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
        } else if any_line(&lines, |l| {
            // Solo una comparación ejecutada en GPU acredita cálculo: ni el
            // pool de VRAM, ni GSP_INIT_DONE, ni una ruta CPU con «gpu» en la
            // línea. Han de aparecer el kernel medido y su resultado.
            l.contains("matvec")
                && l.contains("gpu")
                && l.contains("tok/s")
                && !l.contains("cpu")
                && !l.contains("soft")
        }) || any_line(&lines, |l| {
            (l.contains("saxpy") || l.contains("matvec"))
                && l.contains("gpu")
                && (l.contains("coincide") || l.contains("ok vs cpu") || l.contains("max_err"))
        }) {
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
    let nota = flag(args, "--nota").unwrap_or_else(|| "arranque anotado a mano".into());
    let mut m = load_matrix();
    let entry = find_entry(&mut m, &id).expect("entrada desconocida");
    // Un arranque anotado a mano también es una ejecución del historial: la
    // racha sigue derivándose de él, no de un contador incremental aparte.
    let sello = format!("manual:{id}:{}:{}:{}", today(), entry.historial.len(), nota);
    let run = RunEvidence {
        log_hash: sha256_hex(sello.as_bytes()),
        kernel: "desconocida (anotado a mano)".into(),
        identidad_conocida: false,
        artefactos: entry.artefactos.clone(),
        userspace: !fail,
        fecha: today(),
        ..Default::default()
    };
    entry.historial.push(run);
    if fail {
        entry.notas = format!("{}; arranque fallido {}", entry.notas, today());
    }
    recalcular_racha(entry);
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

/// Todas las apariciones de una bandera repetible.
fn flags(args: &[String], name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == name {
            if let Some(v) = args.get(i + 1) {
                out.push(v.clone());
                i += 1;
            }
        }
        i += 1;
    }
    out
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

    // --- R2: identidad de arranque e importación fiable -------------------

    const RUN15: &str = include_str!("../fixtures/run15-reducido.log");

    #[test]
    fn r2_run15_es_un_solo_arranque_con_su_version() {
        let boots = split_boots(RUN15);
        assert_eq!(boots.len(), 1, "run15 debe ser un único arranque");
        let runs = parse_log_runs(RUN15);
        assert_eq!(runs.len(), 1);
        let r = &runs[0];
        assert!(r.identidad_conocida, "la versión está en el mismo arranque");
        assert_eq!(r.kernel, "soso 0.2.2 (276404696-dirty)");
        assert_eq!(r.flushes, 2, "dos instantáneas del mismo arranque");
        // Etapas del mismo arranque, no repartidas entre cabecera y cuerpo.
        assert_eq!(r.wifi.alive.status, "ok");
        assert_eq!(r.wifi.init_complete.status, "ok");
        assert_eq!(r.gpu.gsp_rpc.status, "ok");
        assert_eq!(r.gpu.ce_readback.status, "ok");
        assert_eq!(r.gpu.vram_pool.status, "fail");
        assert!(r.userspace, "llegó a sosh sin panic");
        assert!(!r.campana_wifi, "TX_ANT falló: la campaña WiFi no se supera");
        assert!(!r.campana_gpu, "sin compute comparado no hay campaña GPU");
        assert!(r.pci.contains(&"10de:249c".to_string()));
    }

    #[test]
    fn r2_updslot_no_es_identidad_de_kernel() {
        // El slot de la ESP habla del fichero, no de lo que se ejecuta.
        let solo_updslot = "boot: memtest\nupdslot: SOSOKRN.BIN LBA 417 (64 MiB)\nboot: task\n";
        let runs = parse_log_runs(solo_updslot);
        assert_eq!(runs.len(), 1);
        assert!(!runs[0].identidad_conocida);
        assert_eq!(runs[0].kernel, "desconocida");
        assert!(banner_version("updslot: SOSOKRN.BIN LBA 417 (64 MiB)").is_none());
        assert!(banner_version("soso 0.2.2 (276404696-dirty)").is_some());
        assert!(banner_version("sosh — escribe 'help' para la ayuda").is_none());
        assert!(banner_version("=== soso log flush #45 uptime=1071560ms ===").is_none());
    }

    #[test]
    fn r2_dos_arranques_conservan_su_cabecera() {
        let dos = "\
soso 0.2.2 (aaaaaaaaa)\nboot: memtest\nUCODE_ALIVE_NTFY\nboot: task\nsosh — hola\n\
soso 0.2.3 (bbbbbbbbb)\nboot: memtest\niwl: start\n";
        let runs = parse_log_runs(dos);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].kernel, "soso 0.2.2 (aaaaaaaaa)");
        assert_eq!(runs[1].kernel, "soso 0.2.3 (bbbbbbbbb)");
        assert!(runs[0].userspace);
        assert!(!runs[1].userspace);
        assert_ne!(runs[0].log_hash, runs[1].log_hash);
    }

    #[test]
    fn r2_reimportar_no_infla_la_racha() {
        let mut entry = seed_entry("t", "t", vec![], "p", &vacia_git(), "");
        let runs = parse_log_runs(RUN15);
        for _ in 0..3 {
            for r in &runs {
                anadir_o_fusionar(&mut entry.historial, r);
            }
            recalcular_racha(&mut entry);
        }
        assert_eq!(entry.historial.len(), 1, "tres importaciones, un arranque");
        assert_eq!(entry.arranques_consecutivos_ok, 1);

        // Un intento fallido posterior corta la racha.
        let fallido = parse_log_runs("soso 0.2.2 (276404696-dirty)\nboot: memtest\nKERNEL panic\n");
        for r in &fallido {
            anadir_o_fusionar(&mut entry.historial, r);
        }
        recalcular_racha(&mut entry);
        assert_eq!(entry.historial.len(), 2);
        assert_eq!(entry.arranques_consecutivos_ok, 0);
    }

    #[test]
    fn r2_flushes_repetidos_no_crean_ejecuciones() {
        let mut blob = String::from("soso 0.2.2 (ccc)\nboot: memtest\nUCODE_ALIVE_NTFY\n");
        for i in 0..5 {
            blob.push_str(&format!("=== soso log flush #{i} uptime={}ms ===\n", i * 1000));
            blob.push_str("iwl_mvm: INIT_COMPLETE_NOTIF\n");
        }
        blob.push_str("boot: task\nsosh — hola\n");
        let runs = parse_log_runs(&blob);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].flushes, 5);
        assert!(runs[0].userspace);
    }

    #[test]
    fn r2_truncamiento_no_acredita_ni_borra() {
        // Log cortado a mitad: identidad desconocida y etapas pendientes.
        let cortado = "INIT_COMPLETE_NOTIF a medias";
        let runs = parse_log_runs(cortado);
        assert_eq!(runs.len(), 1);
        assert!(!runs[0].identidad_conocida);
        assert!(!runs[0].userspace);

        // La cola de un arranque anterior (sin cabecera) no es ejecución nueva.
        let con_cola = "…iwl: bytes de un arranque previo\nsoso 0.2.2 (ddd)\nboot: memtest\nboot: task\nsosh — hola\n";
        let runs = parse_log_runs(con_cola);
        assert_eq!(runs.len(), 1, "la cola no cuenta como arranque");
        assert_eq!(runs[0].kernel, "soso 0.2.2 (ddd)");
    }

    #[test]
    fn r2_correlaciona_serial_y_sosodrv_sin_duplicar() {
        let sosolog = "soso 0.2.2 (eee)\nboot: memtest\nUCODE_ALIVE_NTFY\nboot: task\nsosh — hola\n"
            .to_string();
        // El serial ve el mismo arranque y aporta una etapa más.
        let serial = "soso 0.2.2 (eee)\nboot: memtest\nINIT_COMPLETE_NOTIF\nup mínimo listo\n"
            .to_string();
        // SOSODRV es un informe, no un arranque.
        let sosodrv = "driver=iwlwifi status=ok\nGSP_INIT_DONE recibido\n".to_string();
        let arts = std::collections::BTreeMap::from([("kernel".to_string(), "abc123 1B".to_string())]);
        let (runs, avisos) = correlacionar_fuentes(
            &[("SOSOLOG".into(), sosolog), ("serial".into(), serial)],
            &[("SOSODRV".into(), sosodrv)],
            &arts,
        );
        assert!(avisos.is_empty(), "mismo arranque en ambos logs: {avisos:?}");
        assert_eq!(runs.len(), 1, "una ejecución, tres fuentes");
        assert_eq!(runs[0].wifi.alive.status, "ok");
        assert_eq!(runs[0].wifi.init_complete.status, "ok");
        assert_eq!(runs[0].wifi.mvm_ready.status, "ok");
        assert_eq!(runs[0].gpu.gsp_rpc.status, "ok");
        assert_eq!(runs[0].artefactos.get("kernel").map(String::as_str), Some("abc123 1B"));
    }

    #[test]
    fn r2_serial_no_correlacionable_avisa_y_no_pisa() {
        let sosolog = "soso 0.2.2 (fff)\nboot: memtest\nUCODE_ALIVE_NTFY\nboot: task\nsosh — hola\n"
            .to_string();
        let serial = "soso 0.2.2 (fff)\nboot: memtest\niwl: start\nsoso 0.2.2 (fff)\nboot: memtest\niwl: start\n"
            .to_string();
        let (runs, avisos) = correlacionar_fuentes(
            &[("SOSOLOG".into(), sosolog), ("serial".into(), serial)],
            &[],
            &std::collections::BTreeMap::new(),
        );
        assert_eq!(avisos.len(), 1, "debe avisar de que no cuadran");
        assert!(runs.len() >= 2);
        assert_eq!(runs[0].wifi.alive.status, "ok", "el primero conserva su etapa");
    }

    #[test]
    fn r2_dhcp_ethernet_no_acredita_wifi() {
        let w = parse_wifi_stages(
            "assoc=ok\nnet: dhcp 192.168.1.44/24 gw 192.168.1.1 (eth0 e1000e)\n",
        );
        assert_eq!(w.assoc_wpa2.status, "ok");
        assert_eq!(w.dhcp.status, "pendiente", "el lease es de Ethernet");
        let w = parse_wifi_stages("net: dhcp 192.168.1.44/24 lxwifi0\n");
        assert_eq!(w.dhcp.status, "ok");
        assert!(w.dhcp.nota.as_deref().unwrap_or("").contains("lxwifi"));
    }

    #[test]
    fn r2_pool_y_rpc_no_acreditan_compute() {
        let g = parse_gpu_stages(
            "GSP_INIT_DONE recibido\npool VRAM=256MiB\nreadback GO\n\
             soso-llm: matvec cpu 3.2 tok/s (gpu ausente)\n",
        );
        assert_eq!(g.gsp_rpc.status, "ok");
        assert_eq!(g.vram_pool.status, "ok");
        assert_eq!(g.compute_cpu_gpu.status, "pendiente");
        let g = parse_gpu_stages(
            "GSP_INIT_DONE recibido\npool VRAM=256MiB\nreadback GO\n\
             soso-llm: matvec gpu 9.1 tok/s\n",
        );
        assert_eq!(g.compute_cpu_gpu.status, "ok");
        assert!(campana_gpu_ok(&g));
    }

    #[test]
    fn r2_artefactos_hashean_el_arbol() {
        let dir = std::env::temp_dir().join(format!("soso-r2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(dir.join("bin/init"), b"uno").unwrap();
        let (h1, n1) = hash_directorio(&dir).expect("hash");
        assert_eq!(n1, 1);
        std::fs::write(dir.join("bin/init"), b"dos").unwrap();
        let (h2, _) = hash_directorio(&dir).expect("hash");
        assert_ne!(h1, h2, "cambiar contenido cambia el hash");
        std::fs::write(dir.join("bin/sosh"), b"uno").unwrap();
        let (h3, n3) = hash_directorio(&dir).expect("hash");
        assert_ne!(h2, h3, "añadir un fichero cambia el hash");
        assert_eq!(n3, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn r3_manifiesto_esp_identifica_lo_flasheado() {
        let dir = std::env::temp_dir().join(format!("soso-r3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("SOSOHASH.TXT");
        std::fs::write(
            &p,
            "# soso: artefactos flasheados\n\
             version=0.2.2\n\
             build=abc1234-dirty\n\
             perfil=nouveau,iwlwifi\n\
             kernel=deadbeef 32080696\n\
             rootfs=cafe1234 402653184\n\
             modelos=desconocido\n",
        )
        .unwrap();
        let m = leer_manifiesto_esp(&p).expect("manifiesto");
        assert_eq!(m.get("kernel").map(String::as_str), Some("deadbeef 32080696"));
        assert_eq!(m.get("perfil").map(String::as_str), Some("nouveau,iwlwifi"));
        assert!(
            !m.contains_key("modelos"),
            "un artefacto desconocido no se registra como conocido"
        );

        // La ejecución importada se queda con esos hashes, no con los del árbol.
        let runs = parse_log_runs_con_artefactos(RUN15, &m);
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].artefactos.get("kernel").map(String::as_str),
            Some("deadbeef 32080696")
        );

        std::fs::write(&p, "cualquier cosa\n").unwrap();
        assert!(leer_manifiesto_esp(&p).is_err(), "sin claves no es manifiesto");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn vacia_git() -> GitInfo {
        GitInfo {
            commit: "0".into(),
            commit_corto: "0".into(),
            version: "0".into(),
            dirty: false,
            cambios_locales: String::new(),
        }
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
