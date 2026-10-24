//! Aceptación reservada de R47: `read_file_range` no recorre todos los extents del fichero.

#![cfg(feature = "std")]

use block_dev::{BLOCK_SIZE, Block, BlockDevice, BlockError, MemBlockDevice};
use sosofs::builder::build_image;
use sosofs::layout::{ExtentItem, Key, KIND_EXTENT, ROOT_INODE};
use sosofs::{FsError, Sosofs};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use zerocopy::FromBytes;

const EXT: usize = 32 * BLOCK_SIZE; // 128 KiB
const NEXT: usize = 384;
const SIZE: usize = NEXT * EXT; // 48 MiB

/// Dispositivo que cuenta cada bloque leído (no peticiones).
struct Contador {
    inner: MemBlockDevice,
    lecturas: Rc<Cell<u64>>,
}

impl BlockDevice for Contador {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buf: &mut Block) -> Result<(), BlockError> {
        self.lecturas.set(self.lecturas.get() + 1);
        self.inner.read_block(block, buf)
    }
    fn write_block(&mut self, block: u64, buf: &Block) -> Result<(), BlockError> {
        self.inner.write_block(block, buf)
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        self.inner.flush()
    }
}

fn byte(i: usize) -> u8 {
    ((i * 7 + i / 4096) % 251) as u8
}

fn imagen(nombre: &str) -> MemBlockDevice {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("sosofs-aceptacion-r47-{nombre}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let datos: Vec<u8> = (0..SIZE).map(byte).collect();
    std::fs::write(dir.join("grande.bin"), &datos).unwrap();
    std::fs::write(dir.join("pequeno.txt"), b"hola").unwrap();
    let mut dev = MemBlockDevice::new(16384);
    build_image(&dir, &mut dev).unwrap();
    dev
}

fn monta(dev: MemBlockDevice) -> (Sosofs<Contador>, Rc<Cell<u64>>) {
    let c = Rc::new(Cell::new(0));
    let fs = Sosofs::mount(Contador { inner: dev, lecturas: c.clone() }).unwrap();
    (fs, c)
}

/// Lee el rango, comprueba su contenido y devuelve los bloques leídos.
fn lee(fs: &mut Sosofs<Contador>, c: &Rc<Cell<u64>>, ino: u64, off: usize, len: usize) -> u64 {
    let mut out = vec![0u8; len];
    let antes = c.get();
    fs.read_file_range(ino, off, len, &mut out).unwrap();
    let gastadas = c.get() - antes;
    for (i, b) in out.iter().enumerate() {
        assert_eq!(*b, byte(off + i), "byte {} del rango en {off}", i);
    }
    gastadas
}

#[test]
fn rango_en_el_ultimo_extent() {
    let (mut fs, c) = monta(imagen("rango_en_el_ultimo_extent"));
    let ino = fs.resolve("/grande.bin").unwrap();
    let n = lee(&mut fs, &c, ino, SIZE - EXT + 5000, 3000);
    assert!(n <= 8 + 32, "leyó {n} bloques; el máximo es 40");
}

#[test]
fn rango_al_principio_y_en_el_medio() {
    let (mut fs, c) = monta(imagen("rango_al_principio_y_en_el_medio"));
    let ino = fs.resolve("/grande.bin").unwrap();
    let n = lee(&mut fs, &c, ino, 0, 100);
    assert!(n <= 40, "principio: {n} bloques");
    let n = lee(&mut fs, &c, ino, 200 * EXT + 777, 5000);
    assert!(n <= 40, "medio: {n} bloques");
    // Empieza justo en la frontera: el extent anterior no cuenta.
    let n = lee(&mut fs, &c, ino, 200 * EXT, 1);
    assert!(n <= 40, "frontera: {n} bloques");
}

#[test]
fn rango_que_cruza_dos_extents() {
    let (mut fs, c) = monta(imagen("rango_que_cruza_dos_extents"));
    let ino = fs.resolve("/grande.bin").unwrap();
    let n = lee(&mut fs, &c, ino, 101 * EXT - 10, 20);
    assert!(n <= 8 + 64, "leyó {n} bloques; el máximo es 72");
    // Tres extents: desde el final del 50 hasta el comienzo del 52.
    let n = lee(&mut fs, &c, ino, 51 * EXT - 1, EXT + 2);
    assert!(n <= 8 + 96, "tres extents: {n} bloques; el máximo es 104");
}

#[test]
fn el_contenido_completo_no_cambia() {
    let (mut fs, _c) = monta(imagen("el_contenido_completo_no_cambia"));
    let ino = fs.resolve("/grande.bin").unwrap();
    let todo = fs.read_file(ino).unwrap();
    assert_eq!(todo.len(), SIZE);
    assert!(todo.iter().enumerate().all(|(i, b)| *b == byte(i)));
    let peq = fs.resolve("/pequeno.txt").unwrap();
    assert_eq!(fs.read_file(peq).unwrap(), b"hola");
    let mut out = [0u8; 2];
    fs.read_file_range(peq, 1, 2, &mut out).unwrap();
    assert_eq!(&out, b"ol");
}

#[test]
fn los_errores_no_cambian() {
    let (mut fs, _c) = monta(imagen("los_errores_no_cambian"));
    let ino = fs.resolve("/grande.bin").unwrap();
    let mut out = vec![0u8; 16];
    // len 0 es válido, incluso al final.
    fs.read_file_range(ino, SIZE, 0, &mut out).unwrap();
    // Más allá del tamaño.
    assert_eq!(fs.read_file_range(ino, SIZE - 8, 16, &mut out), Err(FsError::Corrupt));
    assert_eq!(fs.read_file_range(ino, SIZE + 1, 0, &mut out), Err(FsError::Corrupt));
    // Buffer más corto que len.
    assert_eq!(fs.read_file_range(ino, 0, 32, &mut out), Err(FsError::Corrupt));
    // Un directorio no es un fichero.
    assert_eq!(fs.read_file_range(ROOT_INODE, 0, 1, &mut out), Err(FsError::NotAFile));
}

#[test]
fn el_checksum_del_extent_se_sigue_verificando() {
    let (mut fs, _c) = monta(imagen("el_checksum_del_extent_se_sigue_verificando"));
    let ino = fs.resolve("/grande.bin").unwrap();
    let k = Key { inode: ino, kind: KIND_EXTENT, offset: ((NEXT - 1) * EXT) as u64 };
    let mut items = Vec::new();
    fs.scan_range(k, k, &mut items).unwrap();
    let (ext, _) = ExtentItem::read_from_prefix(&items[0].1).unwrap();
    let bloque = ext.start_block.get();
    let mut dev = fs.into_device().inner;
    dev.data_mut()[bloque as usize * BLOCK_SIZE + 7] ^= 0x40;
    let (mut fs, _c) = monta(dev);
    let mut out = vec![0u8; 100];
    assert_eq!(
        fs.read_file_range(ino, SIZE - 200, 100, &mut out),
        Err(FsError::BadChecksum { block: bloque })
    );
    // Un rango en otro extent sigue leyéndose.
    fs.read_file_range(ino, 0, 100, &mut out).unwrap();
}
