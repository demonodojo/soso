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
    let dev2 = FileBlockDevice::open(&img).unwrap();
    assert_eq!(dev2.block_count(), large);
    let mut fs = mount(dev2).unwrap();
    assert_eq!(fs.total_blocks(), small);
    let mut sb = *fs.superblock();
    import::grow_superblock(&mut sb, large);
    import::commit_grow(fs.cache.volume_mut().inner_mut(), &sb).unwrap();
    fs.reload_from_disk().unwrap();
    assert_eq!(fs.total_blocks(), large);
}

/// Un `grow` a medias NO puede dejar el volumen sin superbloque.
///
/// Es el caso real del pendrive live: la partición 3 queda siempre algo más
/// grande que la imagen, así que el kernel hace `grow` en el primer arranque de
/// cada stick recién flasheado. Si esa escritura se corta (corte de corriente,
/// sector malo, USB que falla), el volumen tiene que seguir montando con el
/// superbloque anterior — que es justo lo que no pasaba cuando `commit_grow`
/// pisaba los dos slots empezando por el que estaba vivo.
#[test]
fn grow_a_medias_no_pierde_el_volumen() {
    let model = tiny_model_dir();
    let img = PathBuf::from("target/test-import-grow-torn.img");
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

    let mut fs = mount(FileBlockDevice::open(&img).unwrap()).unwrap();
    let gen_antes = fs.generation();
    let mut sb = *fs.superblock();
    import::grow_superblock(&mut sb, large);
    import::commit_grow(fs.cache.volume_mut().inner_mut(), &sb).unwrap();
    fs.reload_from_disk().unwrap();
    assert_eq!(fs.total_blocks(), large);
    let gen_nueva = fs.generation();
    assert!(gen_nueva > gen_antes, "el commit debe subir la generación");
    drop(fs);

    // Escritura a medias: el slot recién escrito queda ilegible.
    let slot_nuevo = gen_nueva % 2;
    let mut datos = std::fs::read(&img).unwrap();
    let off = slot_nuevo as usize * BLOCK_SIZE;
    datos[off..off + BLOCK_SIZE].fill(0);
    std::fs::write(&img, &datos).unwrap();

    // Tiene que seguir montando, con el tamaño de antes del grow.
    let fs = mount(FileBlockDevice::open(&img).unwrap())
        .expect("un grow a medias no puede dejar el volumen sin superbloque");
    assert_eq!(fs.generation(), gen_antes);
    assert_eq!(fs.total_blocks(), small);
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

#[test]
fn shrink_superblock_sin_datos_en_cola() {
    let model = tiny_model_dir();
    let img = PathBuf::from("target/test-import-shrink.img");
    let large = (64 * 1024 * 1024 / BLOCK_SIZE) as u64;
    let mut dev = FileBlockDevice::create(&img, large).unwrap();
    build_from_dir(&mut dev, &model, 1).unwrap();
    let mut fs = mount(dev).unwrap();
    let sb = *fs.superblock();
    let used = import::next_free_lba(&sb, &fs.catalog);
    let nuevo = used + 64;
    assert!(nuevo < sb.total_blocks);
    let mut sb2 = sb;
    import::shrink_superblock(&mut sb2, &fs.catalog, nuevo).unwrap();
    import::commit_grow(fs.cache.volume_mut().inner_mut(), &sb2).unwrap();
    fs.reload_from_disk().unwrap();
    assert_eq!(fs.total_blocks(), nuevo);
}
