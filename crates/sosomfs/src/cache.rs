//! Caché de bloques con políticas NORMAL / STREAM / PIN y readahead.

use alloc::vec::Vec;
use block_dev::{Block, BlockError};
use crate::layout::{CACHE_NORMAL, CACHE_PIN, CACHE_STREAM};
use crate::volume_set::VolumeSet;

/// Suelo de la ventana de streaming, para cachés diminutas.
const STREAM_WINDOW_MIN: usize = 32;
/// Recuerdo de prefetch: cuántos destinos recientes se dan por hechos.
const PREFETCH_RECIENTES: usize = 16;

struct Entry {
    lba: u64,
    data: Block,
    age: u32,
    policy: u8,
    /// Bit de referencia del reloj: lo pone `touch`, lo consume la manecilla.
    usado: bool,
}

/// Ranura libre del índice.
const VACIO: u32 = u32::MAX;
/// Lápida: hubo una entrada aquí y se desalojó.
///
/// Hace falta porque con sondeo lineal no se puede dejar un hueco a media
/// cadena: la búsqueda para en el primer `VACIO` y dejaría de encontrar lo que
/// hay detrás. El primer intento de este índice resolvía las bajas
/// reconstruyendo la tabla entera en cada desalojo, que es **exactamente el
/// O(n) por bloque que se venía a quitar**: con la caché llena, poblar un
/// racimo de 32 bloques costaba 32 reconstrucciones.
const LAPIDA: u32 = u32::MAX - 1;

pub struct BlockCache<V: VolumeSet> {
    vol: V,
    entries: Vec<Entry>,
    capacity: usize,
    tick: u32,
    pin_count: usize,
    /// Techo de entradas para los shards (`CACHE_STREAM`), para que un modelo
    /// grande no se coma la caché entera.
    ///
    /// AVERÍA (2026-08-02): esto era una constante de 32 bloques —128 KiB— y
    /// convertía el prefetch en un generador de trabajo inútil: `prefetch` leía
    /// el shard siguiente entero del disco y la ventana sólo podía quedarse con
    /// los 32 últimos bloques, así que lo tiraba y las faltas de página que
    /// venían detrás lo releían. Medido con el modelo `tiny` (577 bloques):
    /// **54 272 lecturas de 4 KiB a 177 us**, 94 bloques leídos por cada bloque
    /// del modelo, 9,2 s de disco. La ventana tiene que ser proporcional a la
    /// caché, no un número fijo más pequeño que un solo prefetch.
    stream_cap: usize,
    /// Destinos de prefetch ya servidos. Era **una sola casilla**, y eso sólo
    /// frena repeticiones consecutivas del mismo destino: en cuanto el acceso
    /// alterna entre dos shards (el runtime mapea la capa N y la N+1 a la vez)
    /// cada lectura de 4 KiB volvía a prefetchear desde cero.
    prefetch_hechos: [u64; PREFETCH_RECIENTES],
    prefetch_siguiente: usize,
    aciertos: u64,
    fallos: u64,
    /// Índice abierto `lba → posición en entries`, con sondeo lineal.
    ///
    /// Vacío si no hubo memoria para reservarlo: entonces se cae a búsqueda
    /// lineal, que es lento pero correcto. En la máquina de 48 MiB del test de
    /// reclaim nada se reserva sin comprobarlo.
    indice: Vec<u32>,
    mascara: usize,
    lapidas: usize,
    /// Manecilla del reloj de evicción.
    mano: usize,
    /// Entradas desalojables (todas menos las `CACHE_PIN`), al vuelo.
    desalojables: usize,
    /// Sondeos acumulados del índice. Sólo en host: es la ÚNICA forma de
    /// afirmar que el índice hace su trabajo. Medir aciertos no vale — una
    /// tabla saturada de ranuras obsoletas sigue acertando, barriéndola entera.
    #[cfg(any(test, feature = "std"))]
    sondeos: core::cell::Cell<u64>,
}

impl<V: VolumeSet> BlockCache<V> {
    pub fn new(vol: V, capacity: usize) -> Self {
        let capacity = capacity.max(8);
        Self {
            vol,
            entries: Vec::new(),
            capacity,
            tick: 0,
            pin_count: 0,
            stream_cap: (capacity / 2).max(STREAM_WINDOW_MIN).min(capacity),
            prefetch_hechos: [u64::MAX; PREFETCH_RECIENTES],
            prefetch_siguiente: 0,
            aciertos: 0,
            fallos: 0,
            indice: Vec::new(),
            mascara: 0,
            lapidas: 0,
            mano: 0,
            desalojables: 0,
            #[cfg(any(test, feature = "std"))]
            sondeos: core::cell::Cell::new(0),
        }
        .con_indice()
    }

    /// Reserva el índice: el doble de ranuras que entradas (por encima del 50 %
    /// de ocupación el sondeo lineal se degrada) y 4 B por ranura — 16 KiB con
    /// capacidad 2048. Con `try_reserve_exact`, porque en la máquina de 48 MiB
    /// del test de reclaim nada se reserva sin comprobarlo; si no cabe, se sigue
    /// sin índice y `buscar` cae a la búsqueda lineal de siempre.
    fn con_indice(mut self) -> Self {
        let ranuras = (self.capacity * 2).next_power_of_two();
        if self.indice.try_reserve_exact(ranuras).is_ok() {
            self.indice.resize(ranuras, VACIO);
            self.mascara = ranuras - 1;
        }
        self
    }

    pub fn volume_mut(&mut self) -> &mut V {
        &mut self.vol
    }

    /// (aciertos, fallos) desde el montaje. El kernel los publica por
    /// `SYS_IOSTAT`; sin esto, la amplificación de lectura sólo se puede medir
    /// reinstrumentando a mano, que es como se perdieron las cifras anteriores.
    pub fn estadisticas(&self) -> (u64, u64) {
        (self.aciertos, self.fallos)
    }

    /// Comprobación de invariantes, para los tests. Un índice que se desincroniza
    /// de `entries` no da error: da **el bloque equivocado**, que es la avería
    /// más cara de diagnosticar (se manifiesta lejos, como datos corruptos).
    #[cfg(any(test, feature = "std"))]
    pub fn comprobar_invariantes(&self) -> Result<(), alloc::string::String> {
        use alloc::format;
        if self.indice.is_empty() {
            return Ok(());
        }
        // Toda entrada viva se encuentra por el índice, y en su propia posición.
        for (i, e) in self.entries.iter().enumerate() {
            match self.buscar(e.lba) {
                Some(j) if j == i => {}
                otro => {
                    return Err(format!(
                        "entrada {i} (lba {}) se indexa como {otro:?}",
                        e.lba
                    ))
                }
            }
        }
        // Ninguna ranura apunta fuera de rango.
        for (r, &s) in self.indice.iter().enumerate() {
            if s != VACIO && s != LAPIDA && s as usize >= self.entries.len() {
                return Err(format!("ranura {r} apunta a {s}, fuera de {} entradas", self.entries.len()));
            }
        }
        // Las lápidas contadas son las que hay.
        let reales = self.indice.iter().filter(|&&s| s == LAPIDA).count();
        if reales != self.lapidas {
            return Err(format!("lápidas: contadas {} reales {reales}", self.lapidas));
        }
        // Y los desalojables también.
        let no_pin = self.entries.iter().filter(|e| e.policy != CACHE_PIN).count();
        if no_pin != self.desalojables {
            return Err(format!(
                "desalojables: contados {} reales {no_pin}",
                self.desalojables
            ));
        }
        Ok(())
    }

    fn touch(&mut self, idx: usize) {
        self.tick = self.tick.wrapping_add(1);
        self.entries[idx].age = self.tick;
        self.entries[idx].usado = true;
    }

    /// Mezcla de Fibonacci. Los LBA de un shard son **consecutivos**, y el
    /// módulo directo los amontonaría en ranuras contiguas: justo el caso que
    /// convierte el sondeo lineal en una lista.
    fn ranura(&self, lba: u64) -> usize {
        (lba.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 32) as usize & self.mascara
    }

    fn buscar(&self, lba: u64) -> Option<usize> {
        if self.indice.is_empty() {
            return self.entries.iter().position(|e| e.lba == lba);
        }
        let mut r = self.ranura(lba);
        for _ in 0..=self.mascara {
            #[cfg(any(test, feature = "std"))]
            self.sondeos.set(self.sondeos.get() + 1);
            match self.indice[r] {
                VACIO => return None,
                LAPIDA => {}
                v => {
                    if self.entries[v as usize].lba == lba {
                        return Some(v as usize);
                    }
                }
            }
            r = (r + 1) & self.mascara;
        }
        None
    }

    /// Sondeos acumulados desde el último `reiniciar_sondeos`.
    #[cfg(any(test, feature = "std"))]
    pub fn sondeos(&self) -> u64 {
        self.sondeos.get()
    }

    #[cfg(any(test, feature = "std"))]
    pub fn reiniciar_sondeos(&self) {
        self.sondeos.set(0);
    }

    fn indexar(&mut self, lba: u64, idx: usize) {
        if self.indice.is_empty() {
            return;
        }
        let mut r = self.ranura(lba);
        for _ in 0..=self.mascara {
            let s = self.indice[r];
            if s == VACIO || s == LAPIDA {
                if s == LAPIDA {
                    self.lapidas -= 1;
                }
                self.indice[r] = idx as u32;
                return;
            }
            r = (r + 1) & self.mascara;
        }
    }

    /// Da de baja un LBA dejando lápida. Si las lápidas se comen la tabla, se
    /// recompacta una vez — amortizado sobre `ranuras/4` bajas, no una por baja.
    fn desindexar(&mut self, lba: u64) {
        if self.indice.is_empty() {
            return;
        }
        let mut r = self.ranura(lba);
        for _ in 0..=self.mascara {
            match self.indice[r] {
                VACIO => return,
                LAPIDA => {}
                v => {
                    if self.entries[v as usize].lba == lba {
                        self.indice[r] = LAPIDA;
                        self.lapidas += 1;
                        if self.lapidas > self.indice.len() / 4 {
                            self.recompactar();
                        }
                        return;
                    }
                }
            }
            r = (r + 1) & self.mascara;
        }
    }

    fn recompactar(&mut self) {
        self.indice.iter_mut().for_each(|s| *s = VACIO);
        self.lapidas = 0;
        for i in 0..self.entries.len() {
            let lba = self.entries[i].lba;
            self.indexar(lba, i);
        }
    }

    /// Víctima por reloj de segunda oportunidad: O(1) amortizado.
    ///
    /// Mismos criterios que el LRU exacto al que sustituye —nunca una `PIN`, y
    /// una petición `STREAM` sólo desaloja `STREAM`—, incluido devolver `None`
    /// cuando no hay candidato: ahí el llamante crece la tabla en vez de robarle
    /// el sitio a otra política.
    fn victima(&mut self, policy: u8) -> Option<usize> {
        let n = self.entries.len();
        if n == 0 {
            return None;
        }
        if self.mano >= n {
            self.mano = 0;
        }
        // Dos vueltas: en la primera se consumen los bits de referencia, en la
        // segunda ya hay víctima segura.
        for _ in 0..(2 * n) {
            let i = self.mano;
            self.mano = (self.mano + 1) % n;
            let e = &mut self.entries[i];
            if e.policy == CACHE_PIN || (policy == CACHE_STREAM && e.policy != CACHE_STREAM) {
                continue;
            }
            if e.usado {
                e.usado = false;
                continue;
            }
            return Some(i);
        }
        None
    }

    /// Reemplaza la entrada `idx`, manteniendo índice y contadores cuadrados.
    fn reemplazar(&mut self, idx: usize, lba: u64, data: Block, policy: u8) {
        let vieja_lba = self.entries[idx].lba;
        let vieja_pol = self.entries[idx].policy;
        self.desindexar(vieja_lba);
        if vieja_pol == CACHE_PIN {
            self.pin_count = self.pin_count.saturating_sub(1);
            self.desalojables += 1;
        }
        if policy == CACHE_PIN {
            self.pin_count += 1;
            self.desalojables -= 1;
        }
        self.entries[idx] = Entry { lba, data, age: self.tick, policy, usado: true };
        self.indexar(lba, idx);
    }

    pub fn read_lba(&mut self, lba: u64, policy: u8, buf: &mut Block) -> Result<(), BlockError> {
        if let Some(idx) = self.buscar(lba) {
            buf.copy_from_slice(&self.entries[idx].data);
            self.touch(idx);
            self.aciertos += 1;
            return Ok(());
        }
        self.fallos += 1;
        self.vol.read_lba(lba, buf)?;
        self.insert(lba, *buf, policy);
        Ok(())
    }

    fn insert(&mut self, lba: u64, data: Block, policy: u8) {
        self.tick = self.tick.wrapping_add(1);
        let cap = if policy == CACHE_STREAM {
            self.stream_cap
        } else {
            self.capacity
        };
        if self.desalojables >= cap {
            if let Some(idx) = self.victima(policy) {
                self.reemplazar(idx, lba, data, policy);
                return;
            }
        }
        // Crecer con `try_reserve`: la capacidad es un techo, no una promesa.
        //
        // AVERÍA (2026-08-02): `push` a secas duplica el buffer, y con capacidad
        // 2048 la última duplicación pide 2048 × 4112 B = **8,4 MiB contiguos**
        // de una vez. En la máquina de 48 MiB del test de reclaim eso es un
        // `memory allocation of 8421376 bytes failed` y el kernel entero se cae.
        // No se veía porque el techo fijo de 32 bloques impedía llegar; en cuanto
        // la ventana pasó a ser proporcional, saltó. Si no hay memoria, dejar de
        // crecer y reciclar entradas es una degradación correcta.
        if self.entries.len() < self.capacity
            && self.entries.try_reserve(1).is_ok()
        {
            let idx = self.entries.len();
            self.entries.push(Entry { lba, data, age: self.tick, policy, usado: true });
            if policy == CACHE_PIN {
                self.pin_count += 1;
            } else {
                self.desalojables += 1;
            }
            self.indexar(lba, idx);
            return;
        }
        // Sin sitio para crecer: reutilizar la entrada más vieja que se pueda.
        if let Some(idx) = self.victima(policy).or_else(|| self.victima(CACHE_NORMAL)) {
            self.reemplazar(idx, lba, data, policy);
        }
    }

    /// Lee un rango consecutivo con **una** petición al dispositivo y deja los
    /// bloques en la caché.
    ///
    /// Es el punto medio entre los dos caminos que había: `read_lba` cachea pero
    /// pide de 4 KiB en 4 KiB, y `read_range_direct` agrupa pero no cachea.
    /// Servir las faltas de página sólo con el segundo agrupa las lecturas y a
    /// cambio **triplica los bloques leídos**, porque el planificador libera y
    /// vuelve a mapear los mismos shards y ya no hay nada que absorba la
    /// relectura (medido con `tiny`: 579 bloques → 1986).
    pub fn read_range(&mut self, start: u64, buf: &mut [u8], policy: u8) -> Result<(), BlockError> {
        let bloques = buf.len() / 4096;
        if bloques == 0 {
            return Ok(());
        }
        let todos = (0..bloques as u64).all(|i| self.buscar(start + i).is_some());
        if todos {
            for i in 0..bloques {
                let idx = self.buscar(start + i as u64).unwrap();
                buf[i * 4096..(i + 1) * 4096].copy_from_slice(&self.entries[idx].data);
                self.touch(idx);
            }
            self.aciertos += bloques as u64;
            return Ok(());
        }
        self.vol.read_range_lba(start, buf)?;
        self.fallos += bloques as u64;
        for i in 0..bloques {
            let lba = start + i as u64;
            if self.buscar(lba).is_some() {
                continue;
            }
            let mut b = [0u8; 4096];
            b.copy_from_slice(&buf[i * 4096..(i + 1) * 4096]);
            self.insert(lba, b, policy);
        }
        Ok(())
    }

    pub fn prefetch(&mut self, start_lba: u64, bytes: u32, policy: u8) {
        if self.prefetch_hechos.contains(&start_lba) {
            return;
        }
        self.prefetch_hechos[self.prefetch_siguiente] = start_lba;
        self.prefetch_siguiente = (self.prefetch_siguiente + 1) % PREFETCH_RECIENTES;
        // Nunca traer más de lo que la política puede retener: prefetchear por
        // encima del techo desaloja la cabeza del propio prefetch y garantiza
        // que las lecturas que vienen detrás fallen igualmente.
        let cap = if policy == CACHE_STREAM {
            self.stream_cap as u64
        } else {
            self.capacity as u64
        };
        let blocks = ((bytes as u64 + 4095) / 4096).min(cap);
        let end = start_lba.saturating_add(blocks).min(self.vol.total_blocks());
        let mut lba = start_lba;
        while lba < end {
            if self.buscar(lba).is_some() {
                lba += 1;
                continue;
            }
            let mut buf = [0u8; 4096];
            if self.vol.read_lba(lba, &mut buf).is_ok() {
                self.insert(lba, buf, policy);
            }
            lba += 1;
        }
    }
}
