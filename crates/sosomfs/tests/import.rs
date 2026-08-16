use block_dev::{BlockDevice, FileBlockDevice, BLOCK_SIZE};
use sosomodel::layout::MANIFEST_FILE;
use sosomfs::{build_from_dir, import, mount, ImportSession};
use std::path::PathBuf;
use std::process::Command;

fn tiny_model_dir() -> PathBuf {
    let dir = PathBuf::from("target/test-import-tiny");
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

fn tiny2_model_dir() -> PathBuf {
    let dir = PathBuf::from("target/test-import-tiny2");
    if !dir.join("manifest.som").exists() {
        std::fs::create_dir_all(&dir).unwrap();
        let status = Command::new("cargo")
            .args(["run", "-q", "-p", "mkmodel-soso", "--", "--name", "tiny2"])
            .arg(&dir)
            .status()
            .expect("mkmodel-soso tiny2");
        assert!(status.success());
    }
    dir
}

fn read_model_files(model: &PathBuf) -> Vec<(String, Vec<u8>)> {
    use sosomodel::layout::{INDEX_FILE, SHARDS_DIR};
    let mut out = Vec::new();
    for name in [MANIFEST_FILE, INDEX_FILE] {
        out.push((String::from(name), std::fs::read(model.join(name)).unwrap()));
    }
    let shards = model.join(SHARDS_DIR);
    for ent in std::fs::read_dir(&shards).unwrap() {
        let ent = ent.unwrap();
        let rel = format!("{SHARDS_DIR}/{}", ent.file_name().to_string_lossy());
        out.push((rel, std::fs::read(ent.path()).unwrap()));
    }
    out
}

#[test]
fn grow_ampliar_total_blocks() {
    let model = tiny_model_dir();
    let img = PathBuf::from("target/test-import-grow.img");
    let small = (32 * 1024 * 1024 / BLOCK_SIZE) as u64;
    let large = (64 * 1024 * 1024 / BLOCK_SIZE) as u64;
    let mut dev = FileBlockDevice::create(&img, small).unwrap();
    build_from_dir(&mut dev, &model, 1).unwrap();
    drop(dev);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&img)
        .unwrap()
        .set_len(large * BLOCK_SIZE as u64)
        .unwrap();
    let mut dev2 = FileBlockDevice::open(&img).unwrap();
    assert_eq!(dev2.block_count(), large);
    let mut fs = mount(dev2).unwrap();
    assert_eq!(fs.total_blocks(), small);
    let mut sb = *fs.superblock();
    import::grow_superblock(&mut sb, large);
    import::commit_grow(fs.cache.volume_mut().inner_mut(), &sb).unwrap();
    fs.reload_from_disk().unwrap();
    assert_eq!(fs.total_blocks(), large);
}

#[test]
fn import_segundo_modelo() {
    let model = tiny_model_dir();
    let model2 = tiny2_model_dir();
    let img = PathBuf::from("target/test-import-append.img");
    let blocks = (128 * 1024 * 1024 / BLOCK_SIZE) as u64;
    let mut dev = FileBlockDevice::create(&img, blocks).unwrap();
    build_from_dir(&mut dev, &model, 1).unwrap();
    drop(dev);
    let fs = mount(FileBlockDevice::open(&img).unwrap()).unwrap();
    let sb = *fs.superblock();
    let catalog = fs.catalog_ref().clone();
    drop(fs);
    let files2 = read_model_files(&model2);
    let mut dev2 = FileBlockDevice::open(&img).unwrap();
    let mut session = ImportSession::begin(sb, catalog, "tiny2").unwrap();
    for (rel, data) in &files2 {
        session.put_file(&mut dev2, rel, data).unwrap();
    }
    let (sb2, cat) = session.commit(&mut dev2).unwrap();
    assert_eq!(cat.models.len(), 2);
    assert!(cat.find_model("tiny2").is_some());
    assert_eq!(sb2.generation, 2);
    let mut fs2 = mount(FileBlockDevice::open(&img).unwrap()).unwrap();
    fs2.reload_from_disk().unwrap();
    let data = fs2.read_file("/models/tiny2/manifest.som").unwrap();
    let manifest = sosomodel::Manifest::parse(&data).unwrap();
    assert_eq!(manifest.name, "tiny2");
}

#[test]
fn abort_no_cambia_catalogo() {
    let model = tiny_model_dir();
    let img = PathBuf::from("target/test-import-abort.img");
    let blocks = (64 * 1024 * 1024 / BLOCK_SIZE) as u64;
    let mut dev = FileBlockDevice::create(&img, blocks).unwrap();
    build_from_dir(&mut dev, &model, 1).unwrap();
    drop(dev);
    let fs = mount(FileBlockDevice::open(&img).unwrap()).unwrap();
    let gen0 = fs.generation();
    let sb = *fs.superblock();
    let catalog = fs.catalog_ref().clone();
    drop(fs);
    let mut dev2 = FileBlockDevice::open(&img).unwrap();
    let mut session = ImportSession::begin(sb, catalog, "abortme").unwrap();
    let manifest = std::fs::read(model.join(MANIFEST_FILE)).unwrap();
    session
        .put_file(&mut dev2, MANIFEST_FILE, &manifest)
        .unwrap();
    session.abort();
    let fs2 = mount(FileBlockDevice::open(&img).unwrap()).unwrap();
    assert_eq!(fs2.generation(), gen0);
    assert!(fs2.catalog_ref().find_model("abortme").is_none());
}

#[test]
fn catalog_demasiado_grande_falla() {
    let model = tiny_model_dir();
    let img = PathBuf::from("target/test-import-catbig.img");
    let blocks = (64 * 1024 * 1024 / BLOCK_SIZE) as u64;
    let mut dev = FileBlockDevice::create(&img, blocks).unwrap();
    build_from_dir(&mut dev, &model, 1).unwrap();
    drop(dev);
    let fs = mount(FileBlockDevice::open(&img).unwrap()).unwrap();
    let mut catalog = fs.catalog_ref().clone();
    for i in 0..3000usize {
        catalog.models.push(sosomfs::ModelEntry {
            name: format!("m{i:03}"),
            manifest_lba: 0,
            manifest_blocks: 0,
            index_lba: 0,
            index_blocks: 0,
            shards: Vec::new(),
        });
    }
    let sb = *fs.superblock();
    drop(fs);
    let mut dev2 = FileBlockDevice::open(&img).unwrap();
    let mut session = ImportSession::begin(sb, catalog, "overflow").unwrap();
    let manifest = std::fs::read(model.join(MANIFEST_FILE)).unwrap();
    let mut m = manifest.clone();
    if let Ok(mut man) = sosomodel::Manifest::parse(&manifest) {
        man.name = String::from("overflow");
        m = man.serialize();
    }
    session.put_file(&mut dev2, MANIFEST_FILE, &m).unwrap();
    let idx = std::fs::read(model.join("index.som")).unwrap();
    session.put_file(&mut dev2, "index.som", &idx).unwrap();
    assert!(matches!(
        session.commit(&mut dev2),
        Err(sosomfs::ImportError::CatalogTooLarge)
    ));
}
