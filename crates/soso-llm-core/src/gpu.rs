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

/// Capa Gated DeltaNet de Qwen3.5/3.6 para un despacho que la haga entera en el dispositivo (proyecciones, conv, recurrencia, norma con
/// puerta y proyección de salida), sin pasar las activaciones intermedias por el bus.
pub struct GdnCapa<'a> {
    pub claves: [&'a str; 3], // qkv, z (attn_gate), salida (ssm_out)
    pub vistas: [&'a TensorView<'a>; 3],
    /// `ssm_beta` y `ssm_alpha` (`n_v × h` f32), pesos de la conv (`qkv_dim × kernel`), `ssm_dt`, `ssm_a` (`n_v`) y `ssm_norm` (`d`).
    pub beta_w: &'a [f32],
    pub alpha_w: &'a [f32],
    pub conv_w: &'a [f32],
    pub dt: &'a [f32],
    pub a: &'a [f32],
    pub norma: &'a [f32],
    pub n_k: usize,
    pub n_v: usize,
    pub d: usize,
    pub kernel: usize,
    pub eps: f32,
}

/// Capa de atención con puerta (Qwen3.5/3.6 Gated) para un despacho que la haga entera en el dispositivo (q/k/v, qk-norm, rope, escritura del KV,
/// atención, puerta sigmoide y proyección de salida).
pub struct AtencionCapa<'a> {
    pub claves: [&'a str; 4], // attn_q, attn_k, attn_v, attn_output
    pub vistas: [&'a TensorView<'a>; 4],
    pub qnorm: &'a [f32],
    pub knorm: &'a [f32],
    pub heads: usize,
    pub kv_heads: usize,
    pub hd: usize,
    pub rope_n: usize,
    pub theta: f32,
    pub eps: f32,
}

/// Vista cruda de un tensor: puntero al mapa de la fuente (vive mientras viva el modelo), sin tomar prestada la fuente.
#[derive(Clone, Copy, Debug)]
pub struct VistaCruda {
    pub ptr: usize,
    pub len: usize,
    pub dtype: u8,
    pub elems: usize,
}

impl VistaCruda {
    pub fn de(v: &TensorView<'_>) -> Self {
        VistaCruda { ptr: v.bytes.as_ptr() as usize, len: v.bytes.len(), dtype: v.dtype, elems: v.elems }
    }

    /// # Safety
    /// El tensor al que apunta debe seguir mapeado.
    pub unsafe fn vista(&self) -> TensorView<'static> {
        TensorView { bytes: unsafe { core::slice::from_raw_parts(self.ptr as *const u8, self.len) }, dtype: self.dtype, elems: self.elems }
    }
}

/// Una matriz del modelo para el decode en GPU.
#[derive(Clone, Debug)]
pub struct MatDec {
    pub clave: alloc::string::String,
    pub v: VistaCruda,
    pub rows: usize,
    pub cols: usize,
}

#[derive(Clone, Debug)]
pub struct GdnDec {
    pub n_k: usize,
    pub n_v: usize,
    pub d: usize,
    pub kernel: usize,
    pub beta_w: VistaCruda,
    pub alpha_w: VistaCruda,
    pub conv_w: VistaCruda,
    pub dt: alloc::vec::Vec<f32>,
    pub a: alloc::vec::Vec<f32>,
    pub norma: alloc::vec::Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct GatedDec {
    pub heads: usize,
    pub kv_heads: usize,
    pub hd: usize,
    pub rope_n: usize,
    pub theta: f32,
    pub q_norm: alloc::vec::Vec<f32>,
    pub k_norm: alloc::vec::Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct CompartidoDec {
    pub mats: [MatDec; 3],
    pub pg: VistaCruda,
}

#[derive(Clone, Debug)]
pub struct MoeDec {
    pub router: VistaCruda,
    pub n_exp: usize,
    pub top_k: usize,
    pub ffn: usize,
    pub expertos: alloc::vec::Vec<[MatDec; 3]>,
    pub compartido: Option<CompartidoDec>,
}

#[derive(Clone, Debug)]
pub struct CapaDec {
    pub layer: u32,
    pub gdn: Option<GdnDec>,
    pub gated: Option<GatedDec>,
    pub norma_attn: alloc::vec::Vec<f32>,
    pub norma_ffn: alloc::vec::Vec<f32>,
    /// GDN: qkv, z, salida. Atención con puerta: q, k, v, salida.
    pub mats: alloc::vec::Vec<MatDec>,
    pub moe: MoeDec,
}

/// Todo lo que el decode de un token en GPU necesita saber del modelo.
#[derive(Clone, Debug)]
pub struct DecodePlan {
    pub capas: alloc::vec::Vec<CapaDec>,
    pub h: usize,
    pub eps: f32,
    pub norma_final: alloc::vec::Vec<f32>,
    pub lm_head: MatDec,
    pub vocab: usize,
}

/// Estado de una capa en el host (entra al empezar una sesión de decode y sale al terminarla).
pub struct EstadoCapaDec<'a> {
    pub gdn_s: &'a mut [f32],
    pub gdn_conv: &'a mut [f32],
    pub k: &'a [u16],
    pub v: &'a [u16],
    pub filas_validas: &'a mut usize,
}

/// Un experto de un bloque de prefill MoE: sus tres matrices y los tokens que lo eligen.
pub struct ExpertoBloque<'a> {
    /// Claves estables de `ffn_gate`, `ffn_up` y `ffn_down` (para tenerlas residentes).
    pub claves: [&'a str; 3],
    pub vistas: [&'a TensorView<'a>; 3],
    /// Índice (dentro del bloque) de cada token que elige este experto.
    pub tokens: &'a [u32],
    /// Es un experto compartido (no cuenta para el tope de VRAM de los expertos).
    pub compartido: bool,
    /// Índice `token * top_k + posición en su ranking` de cada uno de esos tokens.
    pub destinos: &'a [u32],
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
    /// cada uno en el dispositivo, con una sola espera. `pesos` (`n × top_k`) son los pesos del ranking; en `suma` (`n × h`) queda, para cada
    /// token, la suma ponderada de los expertos resueltos aquí (en el orden del ranking). `hecho[i]` queda a `true` para los resueltos; el resto
    /// los hace quien llama. Por defecto no resuelve ninguno.
    fn expertos_bloque(
        &mut self,
        xs: &[f32],
        n: usize,
        h: usize,
        ffn: usize,
        top_k: usize,
        pesos: &[f32],
        exps: &mut [ExpertoBloque<'_>],
        suma: &mut [f32],
        hecho: &mut [bool],
    ) -> Result<(), ()> {
        let _ = (xs, n, h, ffn, top_k, pesos, exps, suma, hecho);
        Ok(())
    }

    /// Router de un bloque de `n` tokens (`xs`: `n × h`; `w`: `n_exp × h` f32 con la clave `key`): logits en f32, softmax y top-`top_k` por token en el
    /// dispositivo. `idx` y `pesos` (`n × top_k`) salen en orden de ranking, con los pesos ya renormalizados. `Ok(false)` = hacerlo en CPU.
    #[allow(clippy::too_many_arguments)]
    fn router_topk(
        &mut self,
        key: &str,
        w: &[f32],
        xs: &[f32],
        n: usize,
        h: usize,
        n_exp: usize,
        top_k: usize,
        idx: &mut [i32],
        pesos: &mut [f32],
    ) -> Result<bool, ()> {
        let _ = (key, w, xs, n, h, n_exp, top_k, idx, pesos);
        Ok(false)
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

    /// Capa de atención con puerta entera de un bloque de `n` tokens (`xs`: `n` vectores de `h` ya normalizados) que empieza en la fila
    /// `seq_antes` del KV (= posición absoluta). `salida` (`n × h`) es la proyección de salida (sin el residual). `k_host`/`v_host`: el KV del host
    /// (`seq_antes` filas); `filas_validas`: filas del principio que el dispositivo ya espeja (se actualiza). Devuelve en `k_nuevo`/`v_nuevo`
    /// (`n × kv_dim`, f16) las filas que el host debe añadir a su KV. `Ok(false)` = hacerlo en CPU.
    #[allow(clippy::too_many_arguments)]
    fn atencion_capa(
        &mut self,
        layer: u32,
        capa: &AtencionCapa<'_>,
        xs: &[f32],
        n: usize,
        h: usize,
        seq_antes: usize,
        filas_validas: &mut usize,
        k_host: &[u16],
        v_host: &[u16],
        k_nuevo: &mut [u16],
        v_nuevo: &mut [u16],
        salida: &mut [f32],
    ) -> Result<bool, ()> {
        let _ = (layer, capa, xs, n, h, seq_antes, filas_validas, k_host, v_host, k_nuevo, v_nuevo, salida);
        Ok(false)
    }

    /// Capa GDN entera de un bloque de `n` tokens (`xs`: `n` vectores de `h` ya normalizados): `salida` (`n × h`) es la proyección de salida
    /// (sin el residual). `conv_estado` (`(kernel - 1) × qkv_dim`) y `estado` (`n_v × d × d`) entran y salen. `Ok(false)` = hacerlo en CPU.
    #[allow(clippy::too_many_arguments)]
    fn gdn_capa(
        &mut self,
        capa: &GdnCapa<'_>,
        xs: &[f32],
        n: usize,
        h: usize,
        conv_estado: &mut [f32],
        estado: &mut [f32],
        salida: &mut [f32],
    ) -> Result<bool, ()> {
        let _ = (capa, xs, n, h, conv_estado, estado, salida);
        Ok(false)
    }

    /// Gated DeltaNet de un bloque de `n` tokens: conv causal + SiLU, normas, recurrencia por cabeza y puerta. `hist`: `(kernel - 1 + n)`
    /// filas de `qkv_dim` (estado de la conv seguido de las entradas del bloque); `z`: `n × v_dim`; `decay`/`beta`: `n × n_v`; `estado`: `n_v × d × d`
    /// (entra y sale); `salida`: `n × v_dim`. Sólo `d == 128`. `Ok(false)` = hacerlo en CPU.
    #[allow(clippy::too_many_arguments)]
    fn gdn_bloque(
        &mut self,
        hist: &[f32],
        conv: &[f32],
        z: &[f32],
        decay: &[f32],
        beta: &[f32],
        norma: &[f32],
        estado: &mut [f32],
        salida: &mut [f32],
        n: usize,
        n_k: usize,
        n_v: usize,
        d: usize,
        kernel: usize,
        eps: f32,
    ) -> Result<bool, ()> {
        let _ = (hist, conv, z, decay, beta, norma, estado, salida, n, n_k, n_v, d, kernel, eps);
        Ok(false)
    }

    /// Tokens por bloque que prefiere el despacho para el prefill de un MoE (p. ej. cuando sus expertos pasan por el bus).
    fn bloque_prefill_moe(&self) -> Option<usize> {
        None
    }

    /// Prepara el decode de un token entero en el dispositivo (pesos residentes, tablas de expertos, búferes). `Ok(false)` = no se puede.
    fn decode_preparar(&mut self, plan: &DecodePlan) -> Result<bool, ()> {
        let _ = plan;
        Ok(false)
    }

    /// ¿Está preparado el decode en GPU?
    fn decode_listo(&self) -> bool {
        false
    }

    /// Empieza una sesión de decode: sube el estado de las capas (GDN y filas del KV que falten) a la posición `pos`.
    fn decode_subir_estado(&mut self, capas: &mut [EstadoCapaDec<'_>], pos: usize) -> Result<(), ()> {
        let _ = (capas, pos);
        Err(())
    }

    /// Un token en el dispositivo: `embed` es la fila de embedding; `cpu_moe(capa, xn, índices, pesos, salida)` calcula en CPU los expertos
    /// elegidos de las capas cuyos expertos no están en la tarjeta. Deja los logits en `logits` y las filas K/V nuevas de las capas de atención
    /// (en orden de capa, `[k..., v...]` en f16) en `kv_rows`.
    #[allow(clippy::type_complexity)]
    fn decode_token(
        &mut self,
        embed: &[f32],
        pos: usize,
        cpu_moe: &mut dyn FnMut(usize, &[f32], &[u32], &[f32], &mut [f32]) -> Result<(), ()>,
        logits: &mut [f32],
        kv_rows: &mut [u16],
    ) -> Result<bool, ()> {
        let _ = (embed, pos, cpu_moe, logits, kv_rows);
        Ok(false)
    }

    /// Termina la sesión: baja el estado de las capas GDN al host.
    fn decode_bajar_estado(&mut self, capas: &mut [EstadoCapaDec<'_>]) -> Result<(), ()> {
        let _ = capas;
        Ok(())
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
