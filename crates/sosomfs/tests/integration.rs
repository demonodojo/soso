use block_dev::{BlockDevice, FileBlockDevice, SparseBlockDevice, BLOCK_SIZE};
use sosomfs::layout::{CACHE_PIN, CACHE_STREAM, SEGMENT_SIZE};
use sosomfs::{build_from_dir, mount, Sosomfs};
use std::process::Command;
use std::path::PathBuf;

fn tiny_model_dir() -> PathBuf {
    let dir = PathBuf::from("target/test-tiny-model");
    if !dir.join("manifest.som").exists() {
        std::fs::create_dir_all(&dir).unwrap();
        let status = Command::new("cargo")
            .args(["run", "-q", "-p", "mkmodel-soso", "--"])
            .arg(&dir)
            .status()
            .expect("mkmodel-soso");
        assert!(status.success());
    }
    dir
}

#[test]
fn roundtrip_read_manifest() {
    let model = tiny_model_dir();
    let img = PathBuf::from("target/test-sosomfs.img");
    let blocks = (64 * 1024 * 1024 / BLOCK_SIZE) as u64;
    let mut dev = FileBlockDevice::create(&img, blocks).unwrap();
    build_from_dir(&mut dev, &model, 1).unwrap();
    let dev2 = FileBlockDevice::open(&img).unwrap();
    let mut fs = mount(dev2).unwrap();
    let data = fs.read_file("/models/tiny/manifest.som").unwrap();
    let manifest = sosomodel::Manifest::parse(&data).unwrap();
    assert_eq!(manifest.name, "tiny");
    assert_eq!(manifest.num_layers, 4);
}

#[test]
fn sparse_1tib_addressing() {
    let model = tiny_model_dir();
    let blocks = 1024u64 * 1024 * 1024 * 1024 / BLOCK_SIZE as u64;
    let mut dev = SparseBlockDevice::new(blocks);
    let report = build_from_dir(&mut dev, &model, 1).unwrap();
    assert_eq!(report.total_blocks, blocks);
    let mut fs = Sosomfs::mount(sosomfs::SingleDev::new(dev)).unwrap();
    assert_eq!(fs.total_blocks(), blocks);
    let st = fs.stat("/models/tiny/shards/L00.attn_q.tensor").unwrap();
    assert!(st.0 > 0);
}

#[test]
fn segment_crc_corrupt_fails() {
    let model = tiny_model_dir();
    let mut dev = SparseBlockDevice::new(2 * 1024 * 1024);
    build_from_dir(&mut dev, &model, 1).unwrap();
    let fs = Sosomfs::mount(sosomfs::SingleDev::new(dev)).unwrap();
    let (_, shard) = fs.resolve("/models/tiny/shards/L00.attn_q.tensor").unwrap();
    let lba = shard.unwrap().extents[0].start_lba;
    drop(fs);
    let mut dev2 = SparseBlockDevice::new(2 * 1024 * 1024);
    build_from_dir(&mut dev2, &model, 1).unwrap();
    let mut corrupt = [0u8; BLOCK_SIZE];
    dev2.read_block(lba, &mut corrupt).unwrap();
    corrupt[0] ^= 0xff;
    dev2.write_block(lba, &corrupt).unwrap();
    let mut fs2 = Sosomfs::mount(sosomfs::SingleDev::new(dev2)).unwrap();
    let s2 = fs2
        .shard_for_path("/models/tiny/shards/L00.attn_q.tensor")
        .unwrap();
    let mut out = vec![0u8; SEGMENT_SIZE.min(s2.byte_len as usize)];
    let err = fs2.read_range(&s2, 0, out.len(), &mut out);
    assert!(matches!(err, Err(sosomfs::FsError::Corrupt)));
}

#[test]
fn prefetch_chain_present() {
    let model = tiny_model_dir();
    let mut dev = SparseBlockDevice::new(2 * 1024 * 1024);
    build_from_dir(&mut dev, &model, 1).unwrap();
    let fs = Sosomfs::mount(sosomfs::SingleDev::new(dev)).unwrap();
    let m = fs.catalog.find_model("tiny").unwrap();
    let q = m
        .shards
        .iter()
        .find(|s| s.rel_path == "shards/L00.attn_q.tensor")
        .unwrap();
    let k = m
        .shards
        .iter()
        .find(|s| s.rel_path == "shards/L00.attn_k.tensor")
        .unwrap();
    assert_ne!(q.prefetch_next_lba, 0);
    assert_eq!(q.prefetch_next_lba, k.extents[0].start_lba);
    assert_eq!(q.cache_policy, CACHE_STREAM);
}

#[test]
fn manifest_pin_policy() {
    let model = tiny_model_dir();
    let mut dev = SparseBlockDevice::new(2 * 1024 * 1024);
    build_from_dir(&mut dev, &model, 1).unwrap();
    let fs = Sosomfs::mount(sosomfs::SingleDev::new(dev)).unwrap();
    let m = fs.catalog.find_model("tiny").unwrap();
    let man = m.shards.iter().find(|s| s.rel_path == "manifest.som").unwrap();
    assert_eq!(man.cache_policy, CACHE_PIN);
}
