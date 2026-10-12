//! Crash-injection: registra todas las escrituras al dispositivo durante
//! una serie de transacciones y simula un corte de luz en cada punto
//! posible. El FS montado tras el "corte" debe ser SIEMPRE exactamente el
//! contenido de la generación que anuncia.
//!
//! Modelo de disco: las escrituras entre dos flushes pueden llegar al
//! medio en cualquier orden y de forma incompleta -> además del corte por
//! prefijo estricto, se prueban subconjuntos aleatorios (semilla fija) de
//! la época abierta.

#![cfg(feature = "std")]

use block_dev::{BLOCK_SIZE, Block, BlockDevice, BlockError, MemBlockDevice};
use sosofs::layout::{FT_DIR, ROOT_INODE};
use sosofs::{FsError, Sosofs};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Clone)]
enum Op {
    Write(u64, Box<Block>),
    Flush,
}

/// Dispositivo que registra cada operación de escritura.
struct RecordingDevice {
    inner: MemBlockDevice,
    ops: Rc<RefCell<Vec<Op>>>,
}

impl BlockDevice for RecordingDevice {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buf: &mut Block) -> Result<(), BlockError> {
        self.inner.read_block(block, buf)
    }
    fn write_block(&mut self, block: u64, buf: &Block) -> Result<(), BlockError> {
        self.ops.borrow_mut().push(Op::Write(block, Box::new(*buf)));
        self.inner.write_block(block, buf)
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        self.ops.borrow_mut().push(Op::Flush);
        self.inner.flush()
    }
}

/// Contenido lógico completo: ruta -> Some(datos) o None (directorio).
type Snapshot = BTreeMap<String, Option<Vec<u8>>>;

fn snapshot<D: BlockDevice>(fs: &mut Sosofs<D>) -> Result<Snapshot, FsError> {
    fn walk<D: BlockDevice>(
        fs: &mut Sosofs<D>,
        dir: u64,
        prefijo: &str,
        out: &mut Snapshot,
    ) -> Result<(), FsError> {
        for (nombre, ino) in fs.read_dir(dir)? {
            let ruta = format!("{prefijo}/{nombre}");
            let st = fs.stat_inode(ino)?;
            if st.file_type == FT_DIR {
                out.insert(ruta.clone(), None);
                walk(fs, ino, &ruta, out)?;
            } else {
                out.insert(ruta, Some(fs.read_file(ino)?));
            }
        }
        Ok(())
    }
    let mut out = Snapshot::new();
    walk(fs, ROOT_INODE, "", &mut out)?;
    Ok(out)
}

/// Generador determinista (LCG) para los subconjuntos.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

fn contenido(n: usize, sello: u8) -> Vec<u8> {
    (0..n).map(|i| (i as u8).wrapping_mul(31).wrapping_add(sello)).collect()
}

/// Reconstruye el disco tras un corte: épocas completas en orden y, de la
/// época abierta en el punto de corte, un subconjunto (o todo, si `seed`
/// es None -> corte por prefijo estricto).
fn replay(inicial: &[u8], ops: &[Op], corte: usize, seed: Option<u64>) -> MemBlockDevice {
    let mut dev = MemBlockDevice::new((inicial.len() / BLOCK_SIZE) as u64);
    dev.data_mut().copy_from_slice(inicial);

    let inicio_epoca = ops[..corte]
        .iter()
        .rposition(|op| matches!(op, Op::Flush))
        .map_or(0, |i| i + 1);

    for op in &ops[..inicio_epoca] {
        if let Op::Write(b, data) = op {
            dev.write_block(*b, data).unwrap();
        }
    }
    let mut rng = seed.map(Lcg);
    for op in &ops[inicio_epoca..corte] {
        if let Op::Write(b, data) = op {
            let aplicar = match &mut rng {
                None => true,
                Some(rng) => rng.next() % 2 == 0,
            };
            if aplicar {
                dev.write_block(*b, data).unwrap();
            }
        }
    }
    dev
}

#[test]
fn cortes_de_luz_en_todos_los_puntos() {
    // Imagen inicial mínima construida con el builder (generación 1).
    let src = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("crash-src");
    let _ = std::fs::remove_dir_all(&src);
    std::fs::create_dir_all(src.join("etc")).unwrap();
    std::fs::write(src.join("etc/motd"), "hola\n").unwrap();

    let mut base = MemBlockDevice::new(2048); // 8 MiB
    sosofs::builder::build_image(&src, &mut base).unwrap();
    let inicial = base.data().to_vec();

    // Ejecutar transacciones registrando las escrituras y el estado
    // esperado tras cada commit.
    let ops = Rc::new(RefCell::new(Vec::new()));
    let dev = RecordingDevice { inner: base, ops: ops.clone() };
    let mut fs = Sosofs::mount(dev).unwrap();

    let mut esperado: BTreeMap<u64, Snapshot> = BTreeMap::new();
    esperado.insert(1, snapshot(&mut fs).unwrap());
    let mut sellar = |fs: &mut Sosofs<RecordingDevice>| {
        esperado.insert(fs.generation(), snapshot(fs).unwrap());
    };

    fs.create_file(ROOT_INODE, "a.txt", b"uno", 100).unwrap();
    sellar(&mut fs);
    let d = fs.mkdir(ROOT_INODE, "d", 101).unwrap();
    sellar(&mut fs);
    fs.create_file(d, "b.bin", &contenido(10_000, 3), 102).unwrap();
    sellar(&mut fs);
    for i in 0..40 {
        fs.create_file(d, &format!("f{i:02}"), format!("dato {i}").as_bytes(), 103).unwrap();
        sellar(&mut fs);
    }
    // Sobreescritura con fichero multi-extent (>128 KiB).
    fs.create_file(ROOT_INODE, "a.txt", &contenido(200_000, 7), 104).unwrap();
    sellar(&mut fs);
    for i in (0..40).step_by(2) {
        fs.unlink(d, &format!("f{i:02}")).unwrap();
        sellar(&mut fs);
    }
    fs.unlink(d, "b.bin").unwrap();
    sellar(&mut fs);
    fs.create_file(d, "final", b"tras borrar", 105).unwrap();
    sellar(&mut fs);

    let ops = ops.borrow();
    assert!(ops.len() > 100, "escenario demasiado corto: {} ops", ops.len());

    // El corte en CADA punto, con prefijo estricto y 2 barajados de época.
    let mut probados = 0;
    for corte in 0..=ops.len() {
        for seed in [None, Some(corte as u64), Some(corte as u64 + 0x9e37)] {
            let dev = replay(&inicial, &ops, corte, seed);
            let mut fs = Sosofs::mount(dev).expect("el FS debe montar siempre");
            let g = fs.generation();
            let contenido_real = snapshot(&mut fs)
                .unwrap_or_else(|e| panic!("corte {corte} seed {seed:?}: gen {g} ilegible: {e:?}"));
            let previsto = esperado
                .get(&g)
                .unwrap_or_else(|| panic!("corte {corte}: generación desconocida {g}"));
            assert_eq!(
                &contenido_real, previsto,
                "corte {corte} seed {seed:?}: la generación {g} no coincide con lo comprometido"
            );
            probados += 1;
        }
    }
    println!("{probados} escenarios de corte verificados sobre {} ops", ops.len());

    // Con todas las escrituras aplicadas debe verse el último estado.
    let dev = replay(&inicial, &ops, ops.len(), None);
    let mut fs = Sosofs::mount(dev).unwrap();
    assert_eq!(&snapshot(&mut fs).unwrap(), esperado.values().last().unwrap());
}

/// Crear y borrar en bucle no puede agotar el disco: el espacio se recicla.
#[test]
fn el_espacio_se_recicla() {
    let mut dev = MemBlockDevice::new(1024); // 4 MiB
    {
        let src = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("recicla-src");
        let _ = std::fs::remove_dir_all(&src);
        std::fs::create_dir_all(&src).unwrap();
        sosofs::builder::build_image(&src, &mut dev).unwrap();
    }
    let mut fs = Sosofs::mount(dev).unwrap();
    let libres_antes = fs.free_blocks();
    for ciclo in 0..30 {
        for i in 0..5 {
            fs.create_file(ROOT_INODE, &format!("f{i}"), &contenido(60_000, i as u8), 1)
                .unwrap_or_else(|e| panic!("ciclo {ciclo}, f{i}: {e:?}"));
        }
        for i in 0..5 {
            fs.unlink(ROOT_INODE, &format!("f{i}")).unwrap();
        }
    }
    let libres_despues = fs.free_blocks();
    assert!(
        libres_despues + 4 >= libres_antes,
        "fuga de espacio: {libres_antes} -> {libres_despues}"
    );

    // Y el contenido sigue siendo consistente tras remontar.
    let mut fs2 = Sosofs::mount(replay_dev(fs)).unwrap();
    assert_eq!(snapshot(&mut fs2).unwrap().len(), 0);
}

/// Extrae el dispositivo para remontar (consume el fs).
fn replay_dev(fs: Sosofs<MemBlockDevice>) -> MemBlockDevice {
    fs.into_device()
}
