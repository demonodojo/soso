//! Offload GPU híbrido (G5): matvec vía syscall cuando hay VRAM.
//!
//! **Los pesos que importan están cuantizados.** Este despacho sólo admitía F32,
//! así que con un modelo Q4_K o Q8_0 —los que caben en una tarjeta de 12 GiB— el
//! offload no se usaba nunca y no lo decía: `try_gpu_matvec` se rendía por el
//! dtype y el matvec se iba a CPU en silencio. Ahora pasan también Q8_0 y Q4_K, y
//! el que implementa el rasgo decide cómo: el camino de soso es **descuantizar una
//! sola vez al subirlos** (el dispositivo calcula en f32 y los pesos son
//! residentes, así que se paga una vez por tensor, no por token). El precio es
//! VRAM —Q4_K a F32 son ~8×—, y ése es justo el criterio del offload híbrido: lo
//! que no cabe se queda en CPU.

use crate::layer::TensorView;
use crate::sched::{CostModel, N_TRAMOS};
use sosomodel::layout::{DTYPE_F32, DTYPE_MXFP4, DTYPE_Q4_K, DTYPE_Q8_0};

/// Estadísticas acumuladas del despacho GPU.
#[derive(Clone, Copy, Debug, Default)]
pub struct GpuStats {
    pub launches: u64,
    pub total_ns: u64,
    pub bytes_up: u64,
    pub bytes_down: u64,
    /// Expertos MoE servidos desde el pool de VRAM / traídos desde RAM (capas «en flujo»).
    pub flujo_aciertos: u64,
    pub flujo_fallos: u64,
    pub ewma_ns: [f64; N_TRAMOS],
    pub samples: [u64; N_TRAMOS],
}

/// Un experto de un bloque de prefill MoE: sus tres matrices y los tokens que lo eligen.
pub struct ExpertoBloque<'a> {
    /// Claves estables de `ffn_gate`, `ffn_up` y `ffn_down` (para tenerlas residentes).
    pub claves: [&'a str; 3],
    pub vistas: [&'a TensorView<'a>; 3],
    /// Índice (dentro del bloque) de cada token que elige este experto.
    pub tokens: &'a [u32],
    /// `tokens.len() × h` floats: salida de la proyección `down` de cada token.
    pub salida: &'a mut [f32],
}

/// Una operación matvec dentro de un lote.
pub struct MatvecOp<'a> {
    pub key: &'a str,
    pub view: &'a TensorView<'a>,
    pub rows: usize,
    pub cols: usize,
    pub x: &'a [f32],
    pub out: &'a mut [f32],
}

/// Una capa de Gemma entera para un despacho que la ejecute **en la GPU** (G9): del `hidden` de entrada al de
/// salida, con su KV residente en el dispositivo. Los pesos van por clave (el despacho ya los tiene subidos:
/// ver [`GpuDispatch::gemma_subir`]); las normas, que son pequeñas, viajan en cada llamada.
///
/// **Posición y KV.** `pos0` es la posición absoluta del primer token del bloque. El despacho guarda cuántas
/// posiciones tiene de cada capa: si `pos0` es menor que las que guarda, **rebobina** (reutilización de prefijo
/// o secuencia nueva); si es mayor, no puede seguir y devuelve `Ok(false)`.
pub struct GemmaCapa<'a> {
    pub layer: u32,
    pub pos0: usize,
    pub n: usize,
    /// `n` vectores de `h` floats; entra el estado antes de la capa y sale el de después.
    pub hidden: &'a mut [f32],
    pub h: usize,
    pub heads: usize,
    pub kv_heads: usize,
    pub head_dim: usize,
    pub ffn: usize,
    /// Ventana deslizante de la capa (0 = global).
    pub ventana: usize,
    /// Filas máximas del KV de una capa global.
    pub ctx_max: usize,
    pub eps: f32,
    pub theta: f32,
    pub escala: f32,
    pub v_de_k: bool,
    pub v_norm: bool,
    pub qk_norm: bool,
    pub geglu: bool,
    pub layer_scale: f32,
    pub clave_q: &'a str,
    pub clave_k: &'a str,
    pub clave_v: &'a str,
    pub clave_o: &'a str,
    pub clave_up: &'a str,
    pub clave_gate: &'a str,
    pub clave_down: &'a str,
    pub attn_norm: &'a [f32],
    pub q_norm: &'a [f32],
    pub k_norm: &'a [f32],
    pub post_attn_norm: &'a [f32],
    pub ffn_norm: &'a [f32],
    pub post_ffn_norm: &'a [f32],
    pub rope_freqs: Option<&'a [f32]>,
    /// Salida: filas expulsadas por delante del KV de esta capa.
    pub expulsadas: usize,
}

/// Memoria que necesita un modelo MoE en la GPU, para decidir **qué capas llevan sus expertos residentes** y cuáles los
/// traen por demanda («flujo»). Todos los tamaños son bytes en VRAM.
#[derive(Clone, Copy, Debug)]
pub struct MoePlan {
    pub n_layers: usize,
    pub n_exp: usize,
    /// Pesos de atención de todas las capas.
    pub attn_total: usize,
    /// Los tres tensores de los `n_exp` expertos de una capa (con el `down` más grande).
    pub expertos_capa: usize,
    /// KV de todas las capas.
    pub kv_total: usize,
    /// Resto fijo: cabeza de logits, etc.
    pub fijo: usize,
    /// Bytes de gate, up y down (el mayor) de UN experto.
    pub bytes_experto: [usize; 3],
}

/// Cómo se ejecuta una capa MoE en el dispositivo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModoMoe {
    /// Todos sus expertos viven en VRAM.
    Residente,
    /// Atención y router en VRAM; los expertos que elige el router se traen desde RAM a un pool con LRU.
    Flujo,
}

/// Fuente de los expertos de una capa para el modo [`ModoMoe::Flujo`].
pub trait ProveedorExpertos {
    /// Entrega los tres tensores del experto `e` —`destino(0, dtype, gate)`, `destino(1, …, up)`, `destino(2, …, down)`—.
    fn experto(&mut self, e: usize, destino: &mut dyn FnMut(usize, u8, &[u8]) -> Result<(), ()>) -> Result<(), ()>;
}

/// Lo que de una capa MoE es pequeño y fijo: se envía una sola vez (la primera llamada de la capa).
pub struct MoeNormas<'a> {
    pub attn_norm: &'a [f32],
    pub q_norm: &'a [f32],
    pub k_norm: &'a [f32],
    pub ffn_norm: &'a [f32],
    /// Pesos del router, `n_exp × h` f32.
    pub router: &'a [f32],
    /// Factores de frecuencia del RoPE (YaRN): la frecuencia se **divide** por ellos. `None` = RoPE plano.
    pub rope_factores: Option<&'a [f32]>,
}

/// Una capa MoE (Qwen3-MoE / Mellum: GQA con QK-norm, RoPE NeoX, router top-k, expertos SwiGLU Q4_K/Q8_0)
/// entera en el dispositivo, como [`GemmaCapa`]: atención con KV residente, router, expertos elegidos en la GPU.
/// Los expertos son los tensores `{prefijo}.E{nn}.ffn_{gate,up,down}`, ya subidos con `gemma_subir`.
pub struct MoeCapa<'a> {
    pub layer: u32,
    pub pos0: usize,
    pub n: usize,
    pub hidden: &'a mut [f32],
    pub h: usize,
    pub heads: usize,
    pub kv_heads: usize,
    pub head_dim: usize,
    /// Ancho de la FFN de cada experto.
    pub ffn: usize,
    pub n_exp: usize,
    pub top_k: usize,
    pub ventana: usize,
    pub ctx_max: usize,
    pub eps: f32,
    pub theta: f32,
    pub escala: f32,
    pub qk_norm: bool,
    /// Factor que multiplica cos/sin del RoPE (YaRN: `1 + 0.1·ln(factor)`; 1 si no hay).
    pub rope_mscale: f32,
    /// El `hidden` de entrada ya está en el dispositivo (salida de la capa anterior, ver [`GpuDispatch::moe_pendiente`]):
    /// no se sube el de `hidden`.
    pub continuar: bool,
    /// La capa siguiente también irá en la GPU: no se descarga la salida (queda pendiente en el dispositivo) y no se
    /// sincroniza.
    pub encadenar: bool,
    pub clave_q: &'a str,
    pub clave_k: &'a str,
    pub clave_v: &'a str,
    pub clave_o: &'a str,
    pub prefijo: &'a str,
    /// `Some` sólo si el despacho aún no tiene las normas de esta capa (`moe_lista` = false).
    pub normas: Option<MoeNormas<'a>>,
    /// Fuente de los expertos por si la capa va «en flujo» (el despacho decide; si es residente no se usa).
    pub flujo: Option<&'a mut dyn ProveedorExpertos>,
    /// `ffn_down` de los expertos de esta capa en Q4_K (si no, Q8_0). Sólo lo usa el modo «en flujo»; en residente lo da el dtype subido.
    pub down_q4k: bool,
    /// Bytes de gate, up y down (el mayor) de un experto: el tamaño de los huecos del pool.
    pub bytes_experto: [usize; 3],
    pub expulsadas: usize,
}

/// Despacho opcional hacia la GPU del kernel (implementado en userspace).
pub trait GpuDispatch {
    fn available(&self) -> bool;

    /// ¿Sabe este despacho presentar tensores de ese dtype? Por defecto, los de [`dtype_ofrecible`]; un
    /// backend con más núcleos (el CUDA del host acepta Q4_0) lo amplía sin tocar a los demás.
    fn dtype_soportado(&self, dtype: u8) -> bool {
        dtype_ofrecible(dtype)
    }

    fn stats(&self) -> GpuStats {
        GpuStats::default()
    }

    fn cost_model(&self) -> CostModel {
        CostModel::default()
    }

    /// ¿Puede este despacho intentar la capa `layer` de Gemma entera? `false` si ya la rechazó.
    fn gemma_puede(&self, layer: u32) -> bool {
        let _ = layer;
        false
    }

    /// ¿Está ya subido ese tensor para [`Self::gemma_capa`]?
    fn gemma_residente(&self, key: &str) -> bool {
        let _ = key;
        false
    }

    /// Sube un tensor para [`Self::gemma_capa`]; `Ok(false)` si no lo acepta (dtype, VRAM).
    fn gemma_subir(&mut self, key: &str, view: &TensorView<'_>, rows: usize, cols: usize) -> Result<bool, ()> {
        let _ = (key, view, rows, cols);
        Ok(false)
    }

    /// Ejecuta una capa de Gemma entera en el dispositivo; `Ok(false)` si no puede (la CPU la hace).
    fn gemma_capa(&mut self, capa: &mut GemmaCapa<'_>) -> Result<bool, ()> {
        let _ = capa;
        Ok(false)
    }

    /// ¿Puede intentar la capa MoE `layer` entera? `false` si ya la rechazó (o no sabe hacerlo).
    fn moe_puede(&self, layer: u32) -> bool {
        let _ = layer;
        false
    }

    /// ¿Tiene ya las normas, el router y las tablas de expertos de esa capa MoE?
    fn moe_lista(&self, layer: u32) -> bool {
        let _ = layer;
        false
    }

    /// ¿Tiene el dispositivo pendiente (sin descargar) la salida de la capa `layer` para `n` tokens desde `pos0`?
    fn moe_pendiente(&self, layer: u32, pos0: usize, n: usize) -> bool {
        let _ = (layer, pos0, n);
        false
    }

    /// Descarga a `hidden` la salida pendiente y la da por entregada (no hace nada si no hay).
    fn moe_volcar(&mut self, hidden: &mut [f32]) -> Result<(), ()> {
        let _ = hidden;
        Ok(())
    }

    /// Modo de la capa MoE `layer`: la primera vez planifica la VRAM con `plan`. Por defecto, todo residente.
    fn moe_modo(&mut self, layer: u32, plan: &MoePlan) -> ModoMoe {
        let _ = (layer, plan);
        ModoMoe::Residente
    }

    /// ¿Hay capas MoE «en flujo»? (Entonces conviene procesar el prefill en bloques grandes: cada capa en flujo se lee una vez por bloque.)
    fn moe_flujo_activo(&self) -> bool {
        false
    }

    /// ¿Caben `pesos` bytes de pesos y `kv` bytes de KV en lo que queda de VRAM? (Se pregunta antes de subir una capa MoE.)
    fn moe_cabe(&self, pesos: usize, kv: usize) -> bool {
        let _ = (pesos, kv);
        true
    }

    /// Da la capa MoE por no apta para la GPU (no cabe, dtypes…): no se vuelve a intentar.
    fn moe_descartar(&mut self, layer: u32) {
        let _ = layer;
    }

    /// Ejecuta una capa MoE entera en el dispositivo; `Ok(false)` si no puede (la CPU la hace).
    fn moe_capa(&mut self, capa: &mut MoeCapa<'_>) -> Result<bool, ()> {
        let _ = capa;
        Ok(false)
    }

    /// matvec de una fila por elemento de `out`.
    fn matvec(
        &mut self,
        key: &str,
        view: &TensorView<'_>,
        rows: usize,
        cols: usize,
        x: &[f32],
        out: &mut [f32],
    ) -> Result<bool, ()>;

    /// Y[n×rows] = X[n×cols] · W[rows×cols]^T (W row-major).
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
        let _ = (key, view, rows, cols, n, x, out);
        Ok(false)
    }

    /// Expertos de un bloque de `n` tokens (`xs`: `n` vectores de `h`) con los tokens agrupados por experto: gate, up, SwiGLU y down de
    /// cada uno en el dispositivo, con una sola espera. `hecho[i]` queda a `true` para los resueltos (`exps[i].salida` rellena); el resto
    /// los hace quien llama. Por defecto no resuelve ninguno.
    fn expertos_bloque(
        &mut self,
        xs: &[f32],
        n: usize,
        h: usize,
        ffn: usize,
        exps: &mut [ExpertoBloque<'_>],
        hecho: &mut [bool],
    ) -> Result<(), ()> {
        let _ = (xs, n, h, ffn, exps, hecho);
        Ok(())
    }

    /// Atención causal de un bloque de `n` tokens en el dispositivo (KV f16 espejado por capa). `q`: `n` vectores de `heads * hd`; `out`: lo
    /// mismo; `k`/`v`: el KV del host con `seq_antes + n` filas ya añadidas; `filas_validas`: filas del principio que el dispositivo ya tiene
    /// (el despacho sube las que falten y la actualiza). `Ok(false)` = hacerlo en CPU.
    #[allow(clippy::too_many_arguments)]
    fn atencion_prefill(
        &mut self,
        capa: u32,
        q: &[f32],
        out: &mut [f32],
        n: usize,
        seq_antes: usize,
        heads: usize,
        kv_heads: usize,
        hd: usize,
        k: &[u16],
        v: &[u16],
        filas_validas: &mut usize,
    ) -> Result<bool, ()> {
        let _ = (capa, q, out, n, seq_antes, heads, kv_heads, hd, k, v, filas_validas);
        Ok(false)
    }

    /// Varios matvec de **un** vector con una sola espera al final. `hecho[i]` queda a `true` para los que el
    /// despacho ha calculado en `ops[i].out`; los demás los hace quien llama. Por defecto, uno a uno.
    fn matvec_lote(&mut self, ops: &mut [MatvecOp<'_>], hecho: &mut [bool]) -> Result<(), ()> {
        for (i, op) in ops.iter_mut().enumerate() {
            if self.dtype_soportado(op.view.dtype) && self.matvec(op.key, op.view, op.rows, op.cols, op.x, op.out)? {
                hecho[i] = true;
            }
        }
        Ok(())
    }

    /// Varios matvec encolados; espera única al final.
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
        let _ = (x, rows, cols);
        Ok(false)
    }

    fn layernorm_rows(
        &mut self,
        x: &mut [f32],
        weight: &[f32],
        bias: &[f32],
        rows: usize,
        cols: usize,
        eps: f32,
    ) -> Result<bool, ()> {
        let _ = (x, weight, bias, rows, cols, eps);
        Ok(false)
    }

    /// RMSNorm in-place (1 fila): x *= rsqrt(mean(x²)+eps) * weight.
    fn rmsnorm(&mut self, x: &mut [f32], weight: &[f32], eps: f32) -> Result<bool, ()> {
        let _ = (x, weight, eps);
        Ok(false)
    }
}

/// Sin GPU: siempre CPU.
pub struct NoGpu;

impl GpuDispatch for NoGpu {
    fn available(&self) -> bool {
        false
    }

    fn matvec(
        &mut self,
        _key: &str,
        _view: &TensorView<'_>,
        _rows: usize,
        _cols: usize,
        _x: &[f32],
        _out: &mut [f32],
    ) -> Result<bool, ()> {
        Ok(false)
    }
}

/// Dtypes que el despacho sabe presentar.
pub fn dtype_ofrecible(dtype: u8) -> bool {
    matches!(dtype, DTYPE_F32 | DTYPE_Q8_0 | DTYPE_Q4_K | DTYPE_MXFP4)
}

/// Intenta el offload; `true` si el resultado ya está en `out`.
pub fn try_gpu_matvec(
    gpu: &mut dyn GpuDispatch,
    key: &str,
    view: &TensorView<'_>,
    rows: usize,
    cols: usize,
    x: &[f32],
    out: &mut [f32],
) -> Result<bool, ()> {
    if !gpu.available() || !gpu.dtype_soportado(view.dtype) {
        return Ok(false);
    }
    if view.dtype == DTYPE_F32 && view.f32().is_none() {
        return Ok(false);
    }
    if view.elems != rows * cols {
        return Ok(false);
    }
    match gpu.matvec(key, view, rows, cols, x, out) {
        Ok(v) => Ok(v),
        Err(()) => Ok(false),
    }
}

/// Intenta varios matvec en un solo submit BATCH; `true` si todos en GPU.
pub fn try_gpu_matvec_batch(
    gpu: &mut dyn GpuDispatch,
    ops: &mut [MatvecOp<'_>],
) -> Result<bool, ()> {
    if !gpu.available() || ops.is_empty() {
        return Ok(false);
    }
    for op in ops.iter() {
        if !gpu.dtype_soportado(op.view.dtype) {
            return Ok(false);
        }
        if op.view.dtype == DTYPE_F32 && op.view.f32().is_none() {
            return Ok(false);
        }
    }
    match gpu.matvec_batch(ops) {
        Ok(v) => Ok(v),
        Err(()) => Ok(false),
    }
}

/// Intenta RMSNorm en GPU; `true` si `x` ya está normalizado.
pub fn try_gpu_rmsnorm(
    gpu: &mut dyn GpuDispatch,
    x: &mut [f32],
    weight: &[f32],
    eps: f32,
) -> Result<bool, ()> {
    if !gpu.available() || x.len() != weight.len() || x.is_empty() {
        return Ok(false);
    }
    match gpu.rmsnorm(x, weight, eps) {
        Ok(v) => Ok(v),
        Err(()) => Ok(false),
    }
}

/// Intenta matmul batched; `true` si el resultado ya está en `out`.
pub fn try_gpu_matmul(
    gpu: &mut dyn GpuDispatch,
    key: &str,
    view: &TensorView<'_>,
    rows: usize,
    cols: usize,
    n: usize,
    x: &[f32],
    out: &mut [f32],
) -> Result<bool, ()> {
    if !gpu.available() || !gpu.dtype_soportado(view.dtype) {
        return Ok(false);
    }
    if view.dtype == DTYPE_F32 && view.f32().is_none() {
        return Ok(false);
    }
    if view.elems != rows * cols || x.len() != cols * n || out.len() != rows * n {
        return Ok(false);
    }
    match gpu.matmul(key, view, rows, cols, n, x, out) {
        Ok(v) => Ok(v),
        Err(()) => Ok(false),
    }
}
