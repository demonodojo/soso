//! Pruebas contra imágenes reales de `mkfs.fat`, comprobadas con `fsck.fat -n`.

use super::*;
use std::path::PathBuf;
use std::process::Command;
use std::string::ToString;

struct Mem(Vec<u8>);

impl Disco for Mem {
    fn leer(&mut self, lba: u64, buf: &mut [u8]) -> bool {
        let o = lba as usize * SECTOR;
        match self.0.get(o..o + buf.len()) {
            Some(s) => {
                buf.copy_from_slice(s);
                true
            }
            None => false,
        }
    }
    fn escribir(&mut self, lba: u64, buf: &[u8]) -> bool {
        let o = lba as usize * SECTOR;
        match self.0.get_mut(o..o + buf.len()) {
            Some(s) => {
                s.copy_from_slice(buf);
                true
            }
            None => false,
        }
    }
}

fn herramienta(n: &str) -> Option<PathBuf> {
    ["/usr/sbin", "/sbin", "/usr/bin", "/bin"]
        .iter()
        .map(|d| PathBuf::from(d).join(n))
        .find(|p| p.exists())
}

fn tmp(nombre: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("usbfat-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d.join(nombre)
}

/// Imagen nueva de `mib` MiB formateada con `mkfs.fat <args>`. None si no hay mkfs.fat.
fn imagen(nombre: &str, mib: u64, args: &[&str], particionar: bool) -> Option<(PathBuf, u64)> {
    let mkfs = herramienta("mkfs.fat")?;
    let p = tmp(nombre);
    let f = std::fs::File::create(&p).unwrap();
    f.set_len(mib * 1024 * 1024).unwrap();
    drop(f);
    let mut offset = 0u64;
    let mut cmd = Command::new(&mkfs);
    if particionar {
        let sfdisk = herramienta("sfdisk")?;
        let mut h = Command::new(sfdisk)
            .arg("-q")
            .arg(&p)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        use std::io::Write;
        h.stdin.take().unwrap().write_all(b"label: dos\nstart=2048, type=c\n").unwrap();
        assert!(h.wait().unwrap().success());
        offset = 2048;
        cmd.arg("--offset").arg("2048");
    }
    let mut cmd = cmd;
    cmd.args(args).arg(&p);
    if particionar {
        cmd.arg(((mib * 1024 * 1024 / 1024) - 1024).to_string());
    }
    let out = cmd.output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    Some((p, offset))
}

fn fsck(p: &PathBuf, offset: u64) {
    let Some(fsck) = herramienta("fsck.fat") else {
        return;
    };
    let img = if offset == 0 {
        p.clone()
    } else {
        // fsck.fat no sabe de particiones: extraer el volumen.
        let datos = std::fs::read(p).unwrap();
        let q = p.with_extension("vol");
        std::fs::write(&q, &datos[offset as usize * SECTOR..]).unwrap();
        q
    };
    let out = Command::new(fsck).arg("-n").arg("-v").arg(&img).output().unwrap();
    let texto = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    // -v lista lo que comprueba; un problema sale con una de estas formas.
    let malas: Vec<&str> = texto
        .lines()
        .filter(|l| {
            let l = l.to_lowercase();
            (l.contains("free cluster summary")
                || l.contains("bad")
                || l.contains("invalid")
                || l.contains("orphan")
                || l.contains("lost")
                || l.contains("wrong")
                || l.contains("has no")
                || l.contains("contains a free cluster")
                || l.contains("cluster chain")
                || l.contains("duplicate")
                || l.contains("not found"))
                && !l.contains("checking")
                // FSInfo «desconocido» a propósito: válido, obliga a recalcular.
                && !l.contains("summary uninitialized")
        })
        .collect();
    assert!(
        out.status.success() && malas.is_empty(),
        "fsck.fat se queja ({:?}):\n{texto}",
        out.status
    );
}

fn montar(p: &PathBuf) -> Fat<Mem> {
    let datos = std::fs::read(p).unwrap();
    let total = datos.len() as u64 / SECTOR as u64;
    let mut m = Mem(datos);
    let inicio = match buscar_volumen(&mut m, total) {
        Hallazgo::Fat { inicio, .. } => inicio,
        h => panic!("sin volumen FAT: {h:?}"),
    };
    Fat::montar(m, inicio).unwrap()
}

fn guardar(fat: &mut Fat<Mem>, p: &PathBuf) {
    std::fs::write(p, &fat.disco().0).unwrap();
}

fn patron(n: usize, semilla: u8) -> Vec<u8> {
    (0..n).map(|i| (i as u8).wrapping_mul(31).wrapping_add(semilla)).collect()
}

fn leer_todo(fat: &mut Fat<Mem>, n: Nodo) -> Vec<u8> {
    let info = fat.info(n).unwrap();
    let mut b = vec![0u8; info.tam as usize];
    assert_eq!(fat.leer_fichero(n, 0, &mut b).unwrap(), b.len());
    b
}

fn nombres(fat: &mut Fat<Mem>, dir: Nodo) -> Vec<String> {
    let mut v: Vec<String> = fat.listar(dir).unwrap().into_iter().map(|(n, _, _)| n).collect();
    v.sort();
    v
}

/// La batería entera sobre un volumen.
fn bateria(nombre: &str, mib: u64, args: &[&str], tipo: Tipo, particionar: bool) {
    let Some((p, offset)) = imagen(nombre, mib, args, particionar) else {
        std::eprintln!("sin mkfs.fat: prueba omitida");
        return;
    };
    let mut fat = montar(&p);
    assert_eq!(fat.tipo(), tipo);
    assert!(fat.listar(RAIZ).unwrap().is_empty());

    // 8.3 en minúsculas: sin nombre largo, pero se lista igual.
    let t = 1_700_000_000;
    let hola = fat.escribir_fichero(RAIZ, "hola.txt", b"hola mundo\n", t).unwrap();
    assert_eq!(leer_todo(&mut fat, hola), b"hola mundo\n");
    assert_eq!(fat.info(hola).unwrap().mtime, t);

    // Nombre largo con acentos, varios clusters.
    let largo = "Un nombre largo con tildes y eñe — 2026.bin";
    let datos = patron(70_000, 7);
    let n = fat.escribir_fichero(RAIZ, largo, &datos, t).unwrap();
    assert_eq!(leer_todo(&mut fat, n), datos);
    let mut trozo = vec![0u8; 1000];
    assert_eq!(fat.leer_fichero(n, 4000, &mut trozo).unwrap(), 1000);
    assert_eq!(trozo, &datos[4000..5000]);
    // Búsqueda sin distinguir mayúsculas.
    assert_eq!(fat.buscar_en(RAIZ, &largo.to_uppercase()).unwrap(), n);

    // Reemplazar conserva el nodo.
    let otro = patron(3000, 1);
    assert_eq!(fat.escribir_fichero(RAIZ, largo, &otro, t).unwrap(), n);
    assert_eq!(leer_todo(&mut fat, n), otro);

    // Directorios, y uno grande para que crezca.
    let dir = fat.crear_dir(RAIZ, "Carpeta Nueva", t).unwrap();
    assert!(fat.info(dir).unwrap().es_dir);
    assert_eq!(fat.crear_dir(RAIZ, "carpeta nueva", t), Err(Error::Existe));
    for i in 0..150 {
        fat.escribir_fichero(dir, &format!("fichero número {i:03}.txt"), format!("{i}").as_bytes(), t)
            .unwrap();
    }
    assert_eq!(fat.listar(dir).unwrap().len(), 150);
    let f77 = fat.buscar_en(dir, "fichero número 077.txt").unwrap();
    assert_eq!(leer_todo(&mut fat, f77), b"77");

    // Anexar cruzando clusters, truncar arriba y abajo.
    let log = fat.escribir_fichero(dir, "log.txt", b"", t).unwrap();
    let mut esperado = Vec::new();
    for k in 0..40u8 {
        let linea = patron(997, k);
        fat.anexar(log, &linea, t).unwrap();
        esperado.extend_from_slice(&linea);
    }
    assert_eq!(leer_todo(&mut fat, log), esperado);
    fat.truncar(log, 5000, t).unwrap();
    esperado.truncate(5000);
    assert_eq!(leer_todo(&mut fat, log), esperado);
    fat.truncar(log, 9000, t).unwrap();
    esperado.resize(9000, 0);
    assert_eq!(leer_todo(&mut fat, log), esperado);

    // Renombrar: mover a otro directorio, cambiar sólo mayúsculas, mover un dir.
    let movido = fat.renombrar(RAIZ, "hola.txt", dir, "Saludo.TXT").unwrap();
    assert_eq!(leer_todo(&mut fat, movido), b"hola mundo\n");
    assert_eq!(fat.buscar_en(RAIZ, "hola.txt"), Err(Error::NoEncontrado));
    fat.renombrar(dir, "Saludo.TXT", dir, "saludo.txt").unwrap();
    assert!(nombres(&mut fat, dir).contains(&"saludo.txt".to_string()));
    let sub = fat.crear_dir(dir, "sub", t).unwrap();
    fat.escribir_fichero(sub, "dentro", b"x", t).unwrap();
    assert_eq!(fat.renombrar(RAIZ, "Carpeta Nueva", sub, "bucle"), Err(Error::NombreInvalido));
    fat.renombrar(dir, "sub", RAIZ, "sub movido").unwrap();
    let sm = fat.buscar_en(RAIZ, "sub movido").unwrap();
    assert_eq!(nombres(&mut fat, sm), ["dentro"]);
    // Reemplazar un fichero existente al renombrar.
    fat.escribir_fichero(RAIZ, "a", b"AAA", t).unwrap();
    fat.escribir_fichero(RAIZ, "b", b"BBB", t).unwrap();
    let b = fat.renombrar(RAIZ, "a", RAIZ, "b").unwrap();
    assert_eq!(leer_todo(&mut fat, b), b"AAA");
    assert_eq!(fat.buscar_en(RAIZ, "a"), Err(Error::NoEncontrado));

    // Borrar.
    assert_eq!(fat.borrar(RAIZ, "sub movido"), Err(Error::NoVacio));
    fat.borrar(sm, "dentro").unwrap();
    fat.borrar(RAIZ, "sub movido").unwrap();
    for i in (0..150).step_by(2) {
        fat.borrar(dir, &format!("fichero número {i:03}.txt")).unwrap();
    }
    assert_eq!(fat.listar(dir).unwrap().len(), 75 + 2);
    // Los huecos de lo borrado se reutilizan.
    fat.escribir_fichero(dir, "nuevo en un hueco.txt", b"ok", t).unwrap();

    assert_eq!(fat.escribir_fichero(RAIZ, "mal:nombre", b"", t), Err(Error::NombreInvalido));

    let libres_antes = fat.libres().unwrap();
    guardar(&mut fat, &p);
    fsck(&p, offset);

    // Remontar: todo sigue ahí.
    let mut fat = montar(&p);
    let n = fat.buscar_en(RAIZ, largo).unwrap();
    assert_eq!(leer_todo(&mut fat, n), otro);
    let dir = fat.buscar_en(RAIZ, "carpeta nueva").unwrap();
    let log = fat.buscar_en(dir, "LOG.TXT").unwrap();
    assert_eq!(leer_todo(&mut fat, log), esperado);
    assert_eq!(fat.libres().unwrap(), libres_antes);

    // Llenar el volumen: SinEspacio sin estropear nada.
    let grande = vec![0xA5u8; (fat.libres().unwrap() + 4096) as usize];
    assert_eq!(fat.escribir_fichero(RAIZ, "no cabe", &grande, t), Err(Error::SinEspacio));
    assert_eq!(fat.libres().unwrap(), libres_antes);
    guardar(&mut fat, &p);
    fsck(&p, offset);
}

#[test]
fn fat12() {
    bateria("f12.img", 2, &["-F", "12"], Tipo::Fat12, false);
}

#[test]
fn fat16() {
    bateria("f16.img", 32, &["-F", "16"], Tipo::Fat16, false);
}

#[test]
fn fat32() {
    bateria("f32.img", 64, &["-F", "32", "-s", "1"], Tipo::Fat32, false);
}

#[test]
fn fat32_en_particion_mbr() {
    bateria("f32p.img", 80, &["-F", "32", "-s", "1"], Tipo::Fat32, true);
}

#[test]
fn reconoce_exfat_y_ntfs() {
    let mut s = vec![0u8; 4 * SECTOR];
    s[3..11].copy_from_slice(b"EXFAT   ");
    s[510] = 0x55;
    s[511] = 0xAA;
    assert_eq!(buscar_volumen(&mut Mem(s.clone()), 4), Hallazgo::Otro("exFAT"));
    s[3..11].copy_from_slice(b"NTFS    ");
    assert_eq!(buscar_volumen(&mut Mem(s), 4), Hallazgo::Otro("NTFS"));
    assert_eq!(buscar_volumen(&mut Mem(vec![0u8; 4 * SECTOR]), 4), Hallazgo::Nada);
}

#[test]
fn fechas_ida_y_vuelta() {
    for t in [315_532_800u64, 1_000_000_000, 1_700_000_000, 1_764_201_600] {
        let (f, h) = a_fat(t);
        assert_eq!(desde_fat(f, h), t - t % 2);
    }
    assert_eq!(desde_fat(a_fat(0).0, a_fat(0).1), 315_532_800);
}

#[test]
fn nombres_cortos() {
    assert_eq!(como_8_3("hola.txt"), Some((*b"HOLA    TXT", NT_BASE_MIN | NT_EXT_MIN)));
    assert_eq!(como_8_3("README"), Some((*b"README     ", 0)));
    assert_eq!(como_8_3("Hola.txt"), None);
    assert_eq!(como_8_3("demasiado.largo"), None);
    let c = generar_corto("Un nombre largo.jpeg", &[]).unwrap();
    assert_eq!(&c, b"UNNOMB~1JPE");
    let c2 = generar_corto("Un nombre largo.jpeg", &[c]).unwrap();
    assert_eq!(&c2, b"UNNOMB~2JPE");
}
