//! Aceptación reservada de R39.

use block_dev::{BLOCK_SIZE, BlockDevice, BlockError, MemBlockDevice};

#[test]
fn lo_normal_sigue_funcionando() {
    let mut d = MemBlockDevice::new(4);
    let mut b = [0u8; BLOCK_SIZE];
    b[0] = 7;
    b[BLOCK_SIZE - 1] = 9;
    d.write_block(3, &b).unwrap();
    let mut out = [0u8; BLOCK_SIZE];
    d.read_block(3, &mut out).unwrap();
    assert_eq!(out, b);
    let mut dos = vec![0u8; 2 * BLOCK_SIZE];
    d.read_blocks(2, &mut dos).unwrap();
    assert_eq!(dos[BLOCK_SIZE], 7);
    assert_eq!(dos[2 * BLOCK_SIZE - 1], 9);
}

#[test]
fn justo_fuera_de_rango_es_out_of_range() {
    let mut d = MemBlockDevice::new(4);
    let mut b = [0u8; BLOCK_SIZE];
    assert_eq!(d.read_block(4, &mut b), Err(BlockError::OutOfRange));
    assert_eq!(d.write_block(4, &b), Err(BlockError::OutOfRange));
    let mut dos = vec![0u8; 2 * BLOCK_SIZE];
    assert_eq!(d.read_blocks(3, &mut dos), Err(BlockError::OutOfRange));
    assert!(d.read_blocks(2, &mut dos).is_ok());
}

#[test]
fn indices_enormes_no_desbordan() {
    let mut d = MemBlockDevice::new(4);
    let mut b = [0u8; BLOCK_SIZE];
    for blk in [u64::MAX, u64::MAX / 2, 1 << 52, (1 << 52) - 1, (u64::MAX / BLOCK_SIZE as u64) + 1] {
        assert_eq!(d.read_block(blk, &mut b), Err(BlockError::OutOfRange), "read_block({blk})");
        assert_eq!(d.write_block(blk, &b), Err(BlockError::OutOfRange), "write_block({blk})");
        let mut dos = vec![0u8; 2 * BLOCK_SIZE];
        assert_eq!(d.read_blocks(blk, &mut dos), Err(BlockError::OutOfRange), "read_blocks({blk})");
    }
}

#[test]
fn un_fallo_no_toca_nada() {
    let mut d = MemBlockDevice::new(2);
    let mut b = [0xAAu8; BLOCK_SIZE];
    d.write_block(0, &b).unwrap();
    let copia = d.data().to_vec();
    assert!(d.write_block(u64::MAX, &[0x55u8; BLOCK_SIZE]).is_err());
    assert_eq!(d.data(), &copia[..]);
    b.fill(0x11);
    assert!(d.read_block(u64::MAX, &mut b).is_err());
    assert_eq!(b, [0x11u8; BLOCK_SIZE], "el búfer del llamante queda intacto");
    let mut grande = vec![0x22u8; 3 * BLOCK_SIZE];
    assert!(d.read_blocks(1, &mut grande).is_err());
    assert!(grande.iter().all(|&x| x == 0x22));
}

#[test]
fn longitud_no_multiplo_sigue_siendo_out_of_range() {
    let mut d = MemBlockDevice::new(2);
    let mut raro = vec![0u8; BLOCK_SIZE + 1];
    assert_eq!(d.read_blocks(0, &mut raro), Err(BlockError::OutOfRange));
}

#[test]
fn el_contador_cuenta_peticiones() {
    let mut d = MemBlockDevice::new(8);
    let mut v = vec![0u8; 4 * BLOCK_SIZE];
    d.read_blocks(0, &mut v).unwrap();
    assert_eq!(d.read_count(), 1);
}
