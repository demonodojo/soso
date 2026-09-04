//! Despacho GPU vía syscalls del kernel (G5).

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use libsoso::sys;
use soso_abi as abi;
use soso_llm_core::gpu::{GpuDispatch, GpuStats, MatvecOp};
use soso_llm_core::layer::TensorView;
use soso_llm_core::quant::{dequant_mxfp4, dequant_q4_k, dequant_q8_0};
use soso_llm_core::gemm::matvec_f32;
use soso_llm_core::sched::{tramo_idx, CostModel};
use sosomodel::dequant::row_bytes;
use sosomodel::layout::{DTYPE_F32, DTYPE_MXFP4, DTYPE_Q4_K, DTYPE_Q8_0};

/// Pesos ya residentes en el dispositivo, indexados por el nombre del tensor.
///
/// Por NOMBRE y no por dirección: los shards del modelo están mapeados y se
/// pueden desmapear, así que una dirección reutilizada por otro tensor devolvería
/// pesos ajenos con toda la pinta de estar bien. El nombre lo da el ejecutor de
/// capas, que es quien sabe cuál es.
struct Resident {
    key: String,
    bytes: u64,
    handle: u64,
    /// **Qué hay dentro del búfer**, que no es lo mismo que el dtype del tensor:
    /// un Q4_K con `cols` que no sea múltiplo de 256 se sube descuantizado y aquí
    /// pone `F32`.
    ///
    /// Sin este campo el fallo es mortal y mudo: subir bloques Q4_K en crudo y
    /// lanzar `MATVF` hace que el kernel lea nibbles como si fueran f32. Salen
    /// números finitos, sale texto plausible, y no hay ningún error. Por eso
    /// `matvec` despacha por esto y **nunca** vuelve a mirar `view.dtype`.
    fmt: Formato,
}

/// Lo que de verdad está en el búfer del dispositivo.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Formato {
    F32,
    Q4K,
    Q80,
}

/// Búfer de trabajo del que sí hace falta saber cuánto se reservó.
struct Scratch {
    handle: u64,
    bytes: u64,
}

impl Scratch {
    const NONE: Scratch = Scratch {
        handle: u64::MAX,
        bytes: 0,
    };
}

pub struct SysGpu {
    vram_free: u64,
    /// Nombre que dio el kernel. Se guarda para poder decir de qué dispositivo se
    /// habla: "GPU detectada" sobre el dispositivo software de pruebas sería
    /// mentira, y quien lea el log no tiene otra forma de saberlo.
    name: [u8; 32],
    /// Fase del bring-up del dispositivo (`rm_ce`, `rm_compute`…). Es lo que
    /// convierte un `on_gpu=0` en un diagnóstico: sin ella, saber dónde se quedó
    /// el silicio obliga a abrir el log de serie.
    phase: [u8; 16],
    on_gpu: bool,
    resident: Vec<Resident>,
    x: Scratch,
    y: Scratch,
    uploads: usize,
    calls: usize,
    /// Tensores que no caben en el dispositivo y se quedan en CPU. Es una cifra
    /// del offload híbrido, no un error: sin verla, "va lento" no se distingue de
    /// "no está usando la GPU".
    sin_sitio: usize,
    /// Último fallo duro del despacho (syscall, dimensiones…). No incluye el
    /// fallback a CPU por falta de VRAM.
    last_fail: Option<&'static str>,
    /// Tras un fallo de subida/submit (CE atascado, EIO…): no reintentar offload.
    /// Distinto de `sin_sitio`: eso es híbrido legítimo; esto es canal roto.
    offload_dead: bool,
    /// Ciclos gastados en descuantizar y en `gpu_map`, por separado.
    ///
    /// En ciclos y no en milisegundos porque el reloj de `SYS_UPTIME_MS` es el PIT
    /// y subcuenta durante el polling de disco: con él, mover trabajo de CPU a E/S
    /// (o al revés) cambia la cifra por razones que no son el cambio. Y separados
    /// porque son dos culpables muy distintos —el plano f32 en el heap y el camino
    /// a la VRAM— y hasta ahora no había forma de saber cuál dominaba.
    ciclos_dequant: u64,
    ciclos_map: u64,
    /// Tensores echados del dispositivo para hacer sitio. Era mudo: con desalojo en
    /// marcha, cada token resube matrices enteras y el aserto de la suite
    /// (`uploads < calls`) sigue en verde.
    evictions: usize,
    /// Subidas que fueron **en crudo** (bloques cuantizados, sin expandir a f32).
    ///
    /// Se cuenta porque el fallback es correcto y silencioso: si `row_bytes` dejara
    /// de aceptar un `cols`, todo seguiría dando los mismos tokens por el camino del
    /// plano f32 y se perderían el 8× de VRAM y de PCIe sin que nada se pusiera
    /// rojo. Es exactamente la forma del bug que costó tres meses en el camino DMA.
    crudas: usize,
    cost_model: CostModel,
    gpu_stats: GpuStats,
    last_ns: u64,
}

impl SysGpu {
    pub fn new() -> Option<Self> {
        let mut info = abi::GpuInfo::default();
        if sys::gpu_info(&mut info) < 0 || info.present == 0 {
            return None;
        }
        if info.compute == 0 {
            // Presente pero incapaz de lanzar nada (iGPU Intel hoy). Enganchar el
            // despacho aquí sólo añadiría copias.
            return None;
        }
        if info.vram_bufs == 0 {
            // Sabe calcular pero no tiene dónde ponerle los pesos: `GPU_ALLOC_VRAM`
            // va a fallar en cada tensor. Se dice UNA vez y con la fase, porque es
            // la pregunta que costó una noche: la placa Ampere arranca el GSP pero
            // no llega a canal ni CE, así que no hay pool. Sin esta línea, lo único
            // que se veía era «offload desactivado — subida de pesos», que suena a
            // avería de la subida y no a que no había destino.
            libsoso::println!(
                "soso-llm: GPU presente sin pool de VRAM (fase={}) — inferencia en CPU",
                libsoso::str_hasta_nul(&info.phase)
            );
            return None;
        }
        Some(Self {
            vram_free: info.vram_free,
            name: info.name,
            phase: info.phase,
            on_gpu: false,
            resident: Vec::new(),
            x: Scratch::NONE,
            y: Scratch::NONE,
            uploads: 0,
            calls: 0,
            sin_sitio: 0,
            last_fail: None,
            offload_dead: false,
            ciclos_dequant: 0,
            ciclos_map: 0,
            evictions: 0,
            crudas: 0,
            cost_model: CostModel::default(),
            gpu_stats: GpuStats::default(),
            last_ns: 0,
        })
    }

    /// Calibra el modelo de coste con tres matvec sintéticos (~50 ms).
    pub fn calibrar(&mut self) {
        if self.offload_dead || !self.available() {
            return;
        }
        let shapes = [(384usize, 384usize), (1536, 384), (4096, 4096)];
        let mut gpu_fixed = 0f64;
        let mut gpu_mac = 0f64;
        let mut cpu_mac = 0f64;
        let mut n = 0u32;
        for &(rows, cols) in &shapes {
            let elems = rows * cols;
            let mut w = alloc::vec![0.01f32; elems];
            let x = alloc::vec![0.01f32; cols];
            let mut y_cpu = alloc::vec![0.0f32; rows];
            let mut y_gpu = alloc::vec![0.0f32; rows];
            for i in 0..w.len() {
                w[i] = (i as f32 * 0.001) % 0.1;
            }
            let v = TensorView {
                bytes: unsafe {
                    core::slice::from_raw_parts(w.as_ptr().cast::<u8>(), elems * 4)
                },
                dtype: DTYPE_F32,
                elems,
            };
            let t0 = libsoso::ciclos();
            matvec_f32(&w, rows, cols, &x, &mut y_cpu);
            let cpu_c = libsoso::ciclos().wrapping_sub(t0);
            cpu_mac += cpu_c as f64 / (rows * cols) as f64;
            let tg0 = libsoso::ciclos();
            let ok = self.matvec("_cal", &v, rows, cols, &x, &mut y_gpu).unwrap_or(false);
            let gpu_c = libsoso::ciclos().wrapping_sub(tg0);
            if ok {
                gpu_fixed += gpu_c as f64 * 0.3;
                gpu_mac += (gpu_c as f64 * 0.3) / (rows * cols) as f64;
                n += 1;
            }
        }
        if n > 0 {
            self.cost_model.gpu_fixed_ns = gpu_fixed / n as f64;
            self.cost_model.gpu_ns_per_mac = gpu_mac / n as f64;
            self.cost_model.cpu_ns_per_mac = cpu_mac / shapes.len() as f64 * 0.3;
        }
    }

    /// Nombre del dispositivo tal y como lo dio el kernel.
    pub fn device_name(&self) -> &str {
        libsoso::str_hasta_nul(&self.name)
    }

    /// Fase del bring-up tal y como la dio el kernel; vacía si el dispositivo no
    /// tiene fases.
    pub fn phase(&self) -> &str {
        libsoso::str_hasta_nul(&self.phase)
    }

    /// `true` si el último `matvec_f32` lo calculó de verdad el silicio de la GPU
    /// (y no el bucle de CPU del kernel ni el dispositivo software).
    pub fn last_on_gpu(&self) -> bool {
        self.on_gpu
    }

    pub fn stats(&self) -> (usize, usize, usize, usize) {
        (self.calls, self.uploads, self.resident.len(), self.sin_sitio)
    }

    pub fn gpu_stats(&self) -> GpuStats {
        self.gpu_stats
    }

    pub fn last_fail(&self) -> Option<&'static str> {
        self.last_fail
    }

    fn note_fail(&mut self, reason: &'static str) {
        self.last_fail = Some(reason);
    }

    /// Corta el offload: el canal CE/compute no responde. Sin esto, cada matvec
    /// reintenta `gpu_map` de pesos y el log de serie se llena (2026-07-30).
    fn kill_offload(&mut self, reason: &'static str) {
        self.note_fail(reason);
        if !self.offload_dead {
            self.offload_dead = true;
            libsoso::println!(
                "soso-llm: offload GPU desactivado — {} (resto de la inferencia en CPU)",
                reason
            );
        }
    }

    /// Estadísticas del despacho GPU (éxito o fallo de inferencia).
    pub fn print_diagnostics(&self) {
        let (calls, uploads, resident, sin_sitio) = self.stats();
        libsoso::println!(
            "soso-llm: dispositivo «{}» — {} matvec, {} subidas de pesos, {} matrices residentes, {} sin sitio (a CPU), último on_gpu={}",
            self.device_name(),
            calls,
            uploads,
            resident,
            sin_sitio,
            self.last_on_gpu() as u8
        );
        // Ciclos, no ms: el reloj de `SYS_UPTIME_MS` es el PIT y subcuenta durante
        // el polling de disco, así que no sirve para repartir culpas entre CPU y
        // E/S — que es justo lo que hay que hacer aquí.
        if uploads > 0 {
            libsoso::println!(
                "soso-llm: subidas — {} de {} en crudo (sin expandir a f32), {} Mciclos descuantizando, {} Mciclos en gpu_map",
                self.crudas,
                uploads,
                self.ciclos_dequant / 1_000_000,
                self.ciclos_map / 1_000_000
            );
        }
        // El camino sin copias del CE llevaba desde el 2026-08-17 sin dispararse
        // NUNCA con pesos de un modelo (el payload de un shard empieza en +64 y el
        // kernel exigía alineación de página), y desde fuera no se veía: el rebote
        // da el mismo resultado, sólo cuesta una copia entera del tensor por la
        // CPU. Un `rebote` distinto de 0 es una regresión, no un detalle.
        let mut info = abi::GpuInfo::default();
        if sys::gpu_info(&mut info) == 0 && (info.uploads_dma | info.uploads_bounce) != 0 {
            libsoso::println!(
                "soso-llm: subidas a VRAM — {} por DMA del CE (sin copia), {} por rebote ({} MiB copiados)",
                info.uploads_dma,
                info.uploads_bounce,
                info.bounce_bytes >> 20
            );
        }
        if self.evictions > 0 {
            libsoso::println!(
                "soso-llm: {} desalojos de pesos — el modelo no cabe residente y se resube por token",
                self.evictions
            );
        }
        if let Some(r) = self.last_fail() {
            libsoso::println!("soso-llm: último fallo GPU — {}", r);
        }
        if self.offload_dead {
            libsoso::println!("soso-llm: offload estaba desactivado (fallo duro previo)");
        } else if !self.last_on_gpu() && calls > 0 {
            libsoso::println!(
                "soso-llm: el silicio no calculó nada — el GSP se quedó en la fase «{}»",
                self.phase()
            );
        }
    }

    /// Reserva o agranda un búfer de trabajo. Devolver el handle viejo cuando el
    /// nuevo tamaño no cabe es el bug 2 de la cabecera.
    fn ensure_scratch(
        s: &mut Scratch,
        bytes: u64,
        vram_free: &mut u64,
        on_fail: &mut Option<&'static str>,
    ) -> Result<u64, ()> {
        if s.handle != u64::MAX && s.bytes >= bytes {
            return Ok(s.handle);
        }
        if s.handle != u64::MAX {
            let freed = sys::gpu_free(s.handle);
            if freed > 0 {
                *vram_free = vram_free.saturating_add(freed as u64);
            }
            *s = Scratch::NONE;
        }
        if bytes > *vram_free {
            *on_fail = Some("scratch sin VRAM");
            return Err(());
        }
        let h = sys::gpu_alloc(bytes);
        if h < 0 {
            *on_fail = Some("gpu_alloc scratch");
            return Err(());
        }
        *vram_free = vram_free.saturating_sub(bytes);
        s.handle = h as u64;
        s.bytes = bytes;
        Ok(s.handle)
    }

    /// Handle de los pesos y **qué formato tienen dentro**, subiéndolos sólo la
    /// primera vez que se ven.
    ///
    /// Los cuantizados se suben EN CRUDO cuando la GPU sabe comerlos (`cols`
    /// múltiplo del bloque): los bytes viajan del page cache de sosomfs a la VRAM
    /// por DMA del CE sin que la CPU toque uno. Antes se descuantizaban a un plano
    /// f32 del heap —44 MiB por un `ffn_up` de TinyLlama que en disco son 5,9— y esa
    /// reserva costaba más que descuantizar: ~11 000 faltas de página anónimas (el
    /// mmap anónimo no admite huge pages), 44 MiB de puesta a cero y otro tanto de
    /// `munmap` al soltarla, **por tensor**.
    ///
    /// Lo que no puede ir en crudo (MXFP4, o `cols` que no sea bloque entero) sigue
    /// el camino de siempre. No es código muerto: es el fallback.
    fn resident_weights(
        &mut self,
        key: &str,
        view: &TensorView<'_>,
        elems: usize,
        cols: usize,
    ) -> Result<(u64, Formato), ()> {
        // Qué formato acabará en el dispositivo, y cuánto ocupa. `row_bytes` es lo
        // que decide si la GPU puede leer la matriz sin expandirla, y es la misma
        // función que usa el driver para acotar el búfer.
        let (fmt, bytes) = match view.dtype {
            DTYPE_Q4_K if row_bytes(DTYPE_Q4_K, cols).is_some() => {
                (Formato::Q4K, view.bytes.len() as u64)
            }
            DTYPE_Q8_0 if row_bytes(DTYPE_Q8_0, cols).is_some() => {
                (Formato::Q80, view.bytes.len() as u64)
            }
            _ => (Formato::F32, (elems * 4) as u64),
        };

        if let Some(r) = self
            .resident
            .iter()
            .find(|r| r.key == key && r.bytes == bytes && r.fmt == fmt)
        {
            return Ok((r.handle, r.fmt));
        }
        // Sitio: primero por número de entradas, luego por VRAM. Se echa la más
        // antigua, que con un recorrido de capas en orden es la que más tardará
        // en volver a hacer falta.
        while bytes > self.vram_free {
            let Some(old) = self.resident.first() else {
                // Ni vaciando el dispositivo cabe este tensor: se queda en CPU. Es
                // el caso normal de un modelo más grande que la VRAM, no un error.
                self.sin_sitio += 1;
                return Err(());
            };
            let freed = sys::gpu_free(old.handle);
            if freed > 0 {
                self.vram_free = self.vram_free.saturating_add(freed as u64);
            }
            self.resident.remove(0);
            // Se cuenta porque era mudo: con el desalojo en marcha, `uploads` sube
            // en cada token y el aserto de la suite (`uploads < calls`) pasa igual
            // con la mayoría de los matvec resubiendo la matriz entera.
            self.evictions += 1;
        }
        let h = sys::gpu_alloc_vram(bytes);
        if h < 0 {
            // Sin VRAM contable o pool G6 vacío: híbrido, no canal roto.
            self.sin_sitio += 1;
            return Err(());
        }
        let handle = h as u64;
        self.vram_free = self.vram_free.saturating_sub(bytes);
        let mut ciclos_deq = 0u64;
        let t_todo = libsoso::ciclos();
        let subido = match fmt {
            // Los bytes del shard tal cual: el kernel los pasa al CE, que los lee
            // del propio page cache. Cero copias de CPU.
            Formato::Q4K | Formato::Q80 => write_bytes(handle, view.bytes),
            Formato::F32 => match view.dtype {
                DTYPE_F32 => view.f32().ok_or(()).and_then(|w| write_f32(handle, w)),
                DTYPE_Q8_0 | DTYPE_Q4_K | DTYPE_MXFP4 => {
                    // Fallback: MXFP4 (sin kernel) o `cols` que no es bloque entero.
                    let t0 = libsoso::ciclos();
                    let mut plano = alloc::vec![0f32; elems];
                    let ok = match view.dtype {
                        DTYPE_Q8_0 => dequant_q8_0(view.bytes, &mut plano),
                        DTYPE_Q4_K => dequant_q4_k(view.bytes, &mut plano),
                        _ => dequant_mxfp4(view.bytes, &mut plano),
                    };
                    ciclos_deq = libsoso::ciclos().wrapping_sub(t0);
                    ok.and_then(|_| write_f32(handle, &plano))
                }
                _ => Err(()),
            },
        };
        let ciclos_todo = libsoso::ciclos().wrapping_sub(t_todo);
        self.ciclos_dequant = self.ciclos_dequant.wrapping_add(ciclos_deq);
        self.ciclos_map = self
            .ciclos_map
            .wrapping_add(ciclos_todo.saturating_sub(ciclos_deq));
        if subido.is_err() {
            sys::gpu_free(handle);
            self.vram_free = self.vram_free.saturating_add(bytes);
            // CE/DMA roto: cortar offload. Reintentar por cada proyección llenaba
            // la serie con "pushbuffer lleno" (~150 líneas/matvec).
            self.kill_offload("subida de pesos");
            return Err(());
        }
        self.uploads += 1;
        if fmt != Formato::F32 {
            self.crudas += 1;
        }
        self.resident.push(Resident {
            key: String::from(key),
            bytes,
            handle,
            fmt,
        });
        Ok((handle, fmt))
    }

    /// `MATVF` (matriz f32) o `MATVQ` (matriz cuantizada sin expandir), según lo que
    /// haya EN EL BÚFER. El despacho va por `Formato` y no por `view.dtype`: ver el
    /// comentario del campo `Resident::fmt`.
    fn submit_matv(
        fmt: Formato,
        w_h: u64,
        rows: u32,
        cols: u32,
        x_h: u64,
        y_h: u64,
    ) -> Result<u64, ()> {
        let mut cmd = [0u8; 38];
        let n = match fmt {
            Formato::F32 => {
                cmd[0..5].copy_from_slice(b"MATVF");
                37
            }
            Formato::Q4K | Formato::Q80 => {
                cmd[0..5].copy_from_slice(b"MATVQ");
                cmd[37] = if fmt == Formato::Q4K {
                    DTYPE_Q4_K
                } else {
                    DTYPE_Q8_0
                };
                38
            }
        };
        cmd[5..13].copy_from_slice(&w_h.to_le_bytes());
        cmd[13..17].copy_from_slice(&rows.to_le_bytes());
        cmd[17..21].copy_from_slice(&cols.to_le_bytes());
        cmd[21..29].copy_from_slice(&x_h.to_le_bytes());
        cmd[29..37].copy_from_slice(&y_h.to_le_bytes());
        Self::submit_batch(&cmd[..n])
    }

    fn note_launch(&mut self, ns: u64, bytes_up: u64, bytes_down: u64, macs: u64) {
        self.gpu_stats.launches += 1;
        self.gpu_stats.total_ns = self.gpu_stats.total_ns.saturating_add(ns);
        self.gpu_stats.bytes_up = self.gpu_stats.bytes_up.saturating_add(bytes_up);
        self.gpu_stats.bytes_down = self.gpu_stats.bytes_down.saturating_add(bytes_down);
        let tr = tramo_idx(macs);
        self.gpu_stats.samples[tr] = self.gpu_stats.samples[tr].wrapping_add(1);
        if self.gpu_stats.ewma_ns[tr] <= 0.0 {
            self.gpu_stats.ewma_ns[tr] = ns as f64;
        } else {
            self.gpu_stats.ewma_ns[tr] =
                0.125 * ns as f64 + 0.875 * self.gpu_stats.ewma_ns[tr];
        }
        self.last_ns = ns;
    }

    fn submit_matmf(
        w_h: u64,
        rows: u32,
        cols: u32,
        n: u32,
        x_h: u64,
        y_h: u64,
    ) -> Result<u64, ()> {
        let mut cmd = [0u8; 41];
        cmd[0..5].copy_from_slice(b"MATMF");
        cmd[5..13].copy_from_slice(&w_h.to_le_bytes());
        cmd[13..17].copy_from_slice(&rows.to_le_bytes());
        cmd[17..21].copy_from_slice(&cols.to_le_bytes());
        cmd[21..25].copy_from_slice(&n.to_le_bytes());
        cmd[25..33].copy_from_slice(&x_h.to_le_bytes());
        cmd[33..41].copy_from_slice(&y_h.to_le_bytes());
        Self::submit_batch(&cmd)
    }

    fn submit_batch(cmd: &[u8]) -> Result<u64, ()> {
        let rc = sys::gpu_submit(cmd);
        if rc < 0 {
            return Err(());
        }
        Ok(rc as u64)
    }
}

/// Sube `data` al búfer del dispositivo **desde donde está**.
///
/// Antes esto pedía un `mmap`, copiaba los datos ahí, llamaba a `gpu_map` y
/// soltaba el `mmap`: cuatro syscalls y una copia entera de la matriz por subida,
/// más una falta de página por cada página nueva del mapeo. Todo eso existía porque
/// `SYS_GPU_MAP` exigía permiso de escritura en el búfer de origen, y los pesos de
/// un modelo viven en un mapeo de fichero de sólo lectura. Corregido eso en el
/// kernel, la dirección del propio tensor sirve tal cual.
fn write_bytes(handle: u64, data: &[u8]) -> Result<(), ()> {
    if sys::gpu_map(handle, data.as_ptr() as u64, data.len() as u64) < 0 {
        return Err(());
    }
    Ok(())
}

/// Igual, con el tipo del llamante. El kernel **no interpreta** lo que se sube: son
/// bytes, y quien sabe qué son es quien luego elige `MATVF` o `MATVQ`.
fn write_f32(handle: u64, data: &[f32]) -> Result<(), ()> {
    write_bytes(handle, unsafe {
        core::slice::from_raw_parts(data.as_ptr().cast::<u8>(), data.len() * 4)
    })
}

/// Lee el resultado **directamente sobre el destino** del llamante.
fn read_f32_into(handle: u64, out: &mut [f32]) -> Result<(), ()> {
    let bytes = (out.len() * 4) as u64;
    if sys::gpu_read(handle, out.as_mut_ptr() as u64, bytes) < 0 {
        return Err(());
    }
    Ok(())
}

impl GpuDispatch for SysGpu {
    fn available(&self) -> bool {
        !self.offload_dead
    }

    fn stats(&self) -> GpuStats {
        self.gpu_stats
    }

    fn cost_model(&self) -> CostModel {
        self.cost_model
    }

    fn matvec(
        &mut self,
        key: &str,
        view: &TensorView<'_>,
        rows: usize,
        cols: usize,
        x: &[f32],
        out: &mut [f32],
    ) -> Result<bool, ()> {
        if self.offload_dead {
            return Ok(false);
        }
        if view.elems != rows * cols || x.len() != cols || out.len() != rows {
            self.note_fail("dimensiones matvec");
            return Err(());
        }
        // Que no quepa NO es un error: devolver Err aquí abortaría la inferencia
        // entera en vez de calcular esa capa en CPU, que es lo que hay que hacer
        // con un modelo más grande que la VRAM. Subida CE fallida sí corta offload
        // (`kill_offload` dentro de `resident_weights`).
        let Ok((w_handle, fmt)) = self.resident_weights(key, view, rows * cols, cols) else {
            return Ok(false);
        };
        let x_bytes = (cols * 4) as u64;
        let y_bytes = (rows * 4) as u64;
        let Ok(x_handle) = Self::ensure_scratch(
            &mut self.x,
            x_bytes,
            &mut self.vram_free,
            &mut self.last_fail,
        ) else {
            return Ok(false);
        };
        let Ok(y_handle) = Self::ensure_scratch(
            &mut self.y,
            y_bytes,
            &mut self.vram_free,
            &mut self.last_fail,
        ) else {
            return Ok(false);
        };

        if write_f32(x_handle, x).is_err() {
            self.kill_offload("gpu_map vector x");
            return Ok(false);
        }
        let t0 = libsoso::ciclos();
        let bits = match Self::submit_matv(
            fmt,
            w_handle,
            rows as u32,
            cols as u32,
            x_handle,
            y_handle,
        ) {
            Ok(b) => b,
            Err(()) => {
                self.kill_offload("gpu_submit matvec");
                return Ok(false);
            }
        };
        self.calls += 1;
        self.on_gpu = bits & abi::GPU_SUBMIT_ON_GPU != 0;
        if bits & abi::GPU_SUBMIT_COMPUTED == 0 {
            // El dispositivo no calculó nada: que lo haga la CPU de quien llama.
            // Si el canal está muerto el kernel ya habrá contado fallos; aquí no
            // cortamos aún — un COMPUTED=0 puntual (soft) no es CE stuck.
            return Ok(false);
        }
        if read_f32_into(y_handle, out).is_err() {
            self.kill_offload("gpu_read resultado");
            return Ok(false);
        }
        let ns = libsoso::ciclos().wrapping_sub(t0) as u64 * 3 / 10;
        self.note_launch(
            ns,
            x_bytes,
            y_bytes,
            (rows as u64).saturating_mul(cols as u64),
        );
        Ok(true)
    }

    fn matmul(
        &mut self,
        key: &str,
        view: &TensorView<'_>,
        rows: usize,
        cols: usize,
        n: usize,
        x: &[f32],
        out: &mut [f32],
    ) -> Result<bool, ()> {
        if self.offload_dead {
            return Ok(false);
        }
        if view.elems != rows * cols || x.len() != cols * n || out.len() != rows * n {
            return Err(());
        }
        let Ok((w_handle, fmt)) = self.resident_weights(key, view, rows * cols, cols) else {
            return Ok(false);
        };
        if fmt != Formato::F32 {
            return Ok(false);
        }
        let x_bytes = (cols * n * 4) as u64;
        let y_bytes = (rows * n * 4) as u64;
        let Ok(x_handle) = Self::ensure_scratch(
            &mut self.x,
            x_bytes,
            &mut self.vram_free,
            &mut self.last_fail,
        ) else {
            return Ok(false);
        };
        let Ok(y_handle) = Self::ensure_scratch(
            &mut self.y,
            y_bytes,
            &mut self.vram_free,
            &mut self.last_fail,
        ) else {
            return Ok(false);
        };
        if write_f32(x_handle, x).is_err() {
            self.kill_offload("gpu_map matmul x");
            return Ok(false);
        }
        let t0 = libsoso::ciclos();
        let bits = match Self::submit_matmf(
            w_handle,
            rows as u32,
            cols as u32,
            n as u32,
            x_handle,
            y_handle,
        ) {
            Ok(b) => b,
            Err(()) => {
                self.kill_offload("gpu_submit matmul");
                return Ok(false);
            }
        };
        self.calls += 1;
        self.on_gpu = bits & abi::GPU_SUBMIT_ON_GPU != 0;
        if bits & abi::GPU_SUBMIT_COMPUTED == 0 {
            return Ok(false);
        }
        if read_f32_into(y_handle, out).is_err() {
            self.kill_offload("gpu_read matmul");
            return Ok(false);
        }
        let ns = libsoso::ciclos().wrapping_sub(t0) as u64 * 3 / 10;
        self.note_launch(
            ns,
            x_bytes,
            y_bytes,
            (rows as u64).saturating_mul(cols as u64).saturating_mul(n as u64),
        );
        Ok(true)
    }

    fn matvec_batch(&mut self, ops: &mut [MatvecOp<'_>]) -> Result<bool, ()> {
        let mut any = false;
        for op in ops.iter_mut() {
            if self.matvec(op.key, op.view, op.rows, op.cols, op.x, op.out)? {
                any = true;
            }
        }
        Ok(any)
    }

    fn softmax_rows(&mut self, x: &mut [f32], rows: usize, cols: usize) -> Result<bool, ()> {
        if self.offload_dead || x.len() != rows * cols {
            return Ok(false);
        }
        let bytes = (x.len() * 4) as u64;
        let Ok(x_handle) = Self::ensure_scratch(
            &mut self.x,
            bytes,
            &mut self.vram_free,
            &mut self.last_fail,
        ) else {
            return Ok(false);
        };
        if write_f32(x_handle, x).is_err() {
            return Ok(false);
        }
        let mut cmd = [0u8; 21];
        cmd[0..5].copy_from_slice(b"SOFTM");
        cmd[5..13].copy_from_slice(&x_handle.to_le_bytes());
        cmd[13..17].copy_from_slice(&(rows as u32).to_le_bytes());
        cmd[17..21].copy_from_slice(&(cols as u32).to_le_bytes());
        let t0 = libsoso::ciclos();
        let Ok(bits) = Self::submit_batch(&cmd) else {
            return Ok(false);
        };
        if bits & abi::GPU_SUBMIT_COMPUTED == 0 {
            return Ok(false);
        }
        if read_f32_into(x_handle, x).is_err() {
            return Ok(false);
        }
        let ns = libsoso::ciclos().wrapping_sub(t0) as u64 * 3 / 10;
        self.note_launch(ns, bytes, bytes, (rows * cols) as u64);
        Ok(true)
    }

    fn layernorm_rows(
        &mut self,
        _x: &mut [f32],
        _weight: &[f32],
        _bias: &[f32],
        _rows: usize,
        _cols: usize,
        _eps: f32,
    ) -> Result<bool, ()> {
        Ok(false)
    }
}

impl Drop for SysGpu {
    fn drop(&mut self) {
        for r in self.resident.drain(..) {
            let freed = sys::gpu_free(r.handle);
            if freed > 0 {
                self.vram_free = self.vram_free.saturating_add(freed as u64);
            }
        }
        for s in [&mut self.x, &mut self.y] {
            if s.handle != u64::MAX {
                let freed = sys::gpu_free(s.handle);
                if freed > 0 {
                    self.vram_free = self.vram_free.saturating_add(freed as u64);
                }
                *s = Scratch::NONE;
            }
        }
    }
}
