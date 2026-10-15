//! Aceptación reservada de R26.

use gptdisk::{entry_mut, set_entry_first_lba, set_entry_last_lba, Guid, Header};
use soso_resize_core::*;
use std::cell::Cell;

struct Disco {
    data: Vec<u8>,
    lecturas: Cell<u64>,
    falla_desde: Cell<Option<u64>>,
}

impl Disco {
    fn new(p2: PartGeom, p3: PartGeom, p4: PartGeom) -> Self {
        let sectores = 4096u64;
        let mut d = Disco { data: vec![0; sectores as usize * SECTOR], lecturas: Cell::new(0), falla_desde: Cell::new(None) };
        d.escribir_gpt(p2, p3, p4);
        d
    }

    fn escribir_gpt(&mut self, p2: PartGeom, p3: PartGeom, p4: PartGeom) {
        let sectores = (self.data.len() / SECTOR) as u64;
        let hdr = Header {
            revision: 0x0001_0000,
            header_size: 92,
            my_lba: 1,
            alternate_lba: sectores - 1,
            first_usable: 34,
            last_usable: sectores - 34,
            disk_guid: Guid::ZERO,
            entries_lba: 2,
            num_entries: 128,
            entry_size: 128,
        };
        let mut entries = vec![0u8; hdr.entries_bytes()];
        for (idx, g) in [(GPT_ROOT, p2), (GPT_MODELS, p3), (GPT_INSTALL, p4)] {
            let e = entry_mut(&mut entries, &hdr, idx).unwrap();
            e[0] = 0x01;
            set_entry_first_lba(e, g.first);
            set_entry_last_lba(e, g.last);
        }
        self.data[SECTOR..2 * SECTOR].copy_from_slice(&hdr.render(&entries));
        self.data[2 * SECTOR..2 * SECTOR + entries.len()].copy_from_slice(&entries);
    }
}

impl Disk512 for Disco {
    fn read_sector(&self, lba: u64, buf: &mut [u8; SECTOR]) -> Result<(), Error> {
        let n = self.lecturas.get();
        self.lecturas.set(n + 1);
        if self.falla_desde.get().is_some_and(|k| n >= k) {
            return Err(Error::Io);
        }
        let off = lba as usize * SECTOR;
        buf.copy_from_slice(&self.data[off..off + SECTOR]);
        Ok(())
    }
    fn write_sector(&mut self, _lba: u64, _buf: &[u8; SECTOR]) -> Result<(), Error> {
        Err(Error::Io)
    }
}

fn geom(f: u64, l: u64) -> PartGeom {
    PartGeom::from_lba(f, l)
}

fn foto(c: &GeomCache) -> (Vec<Option<PartGeom>>, u64) {
    ((0..5).map(|i| c.get(i)).collect(), c.reloads())
}

fn disco_viejo() -> Disco {
    Disco::new(geom(34, 999), geom(1000, 1999), geom(2000, 2099))
}

fn al_estado_nuevo(d: &mut Disco) {
    d.escribir_gpt(geom(34, 1099), geom(1100, 1999), geom(2100, 2199));
}

#[test]
fn reload_fallido_deja_la_cache_intacta_en_cualquier_punto_de_fallo() {
    // Lecturas de un reload completo y de una sola partición.
    let mut d = disco_viejo();
    al_estado_nuevo(&mut d);
    let total = {
        let mut c = GeomCache::default();
        c.reload(&d).unwrap();
        d.lecturas.get()
    };
    d.lecturas.set(0);
    read_part(&d, GPT_ROOT).unwrap();
    let una = d.lecturas.get();

    let mut errores = 0u64;
    for k in 0..total {
        let mut d = disco_viejo();
        let mut cache = GeomCache::default();
        cache.reload(&d).unwrap();
        let antes = foto(&cache);
        assert_eq!(antes.0[GPT_ROOT], Some(geom(34, 999)));

        al_estado_nuevo(&mut d);
        d.lecturas.set(0);
        d.falla_desde.set(Some(k));
        match cache.reload(&d) {
            Err(_) => {
                errores += 1;
                assert_eq!(foto(&cache), antes, "fallo en la lectura {k}: la caché cambió");
            }
            Ok(()) => {
                // Sólo puede salir bien si lo que falló era opcional (ESP / install).
                assert_eq!(cache.get(GPT_ROOT), Some(geom(34, 1099)), "k={k}");
                assert_eq!(cache.get(GPT_MODELS), Some(geom(1100, 1999)), "k={k}");
                assert_eq!(cache.reloads(), 2, "k={k}");
            }
        }
    }
    assert!(errores >= 2 * una, "debe haber fallos tras leer la primera partición ({errores} errores)");
}

#[test]
fn reload_fallido_sobre_cache_vacia_la_deja_vacia() {
    let disco = Disco::new(geom(34, 999), geom(1000, 1999), geom(2000, 2099));
    let mut cache = GeomCache::default();
    // Falla a mitad de la segunda partición obligatoria.
    disco.lecturas.set(0);
    let una = {
        read_part(&disco, GPT_ROOT).unwrap();
        disco.lecturas.get()
    };
    disco.lecturas.set(0);
    disco.falla_desde.set(Some(una + una / 2));
    assert!(cache.reload(&disco).is_err());
    for i in 0..5 {
        assert_eq!(cache.get(i), None, "índice {i}");
    }
    assert_eq!(cache.reloads(), 0);
}

#[test]
fn reload_correcto_cuenta_y_actualiza() {
    let mut disco = Disco::new(geom(34, 999), geom(1000, 1999), geom(2000, 2099));
    let mut cache = GeomCache::default();
    cache.reload(&disco).unwrap();
    assert_eq!(cache.reloads(), 1);
    disco.escribir_gpt(geom(34, 1099), geom(1100, 1999), geom(2100, 2199));
    cache.reload(&disco).unwrap();
    assert_eq!(cache.reloads(), 2);
    assert_eq!(cache.get(GPT_ROOT), Some(geom(34, 1099)));
    assert_eq!(cache.get(GPT_INSTALL), Some(geom(2100, 2199)));
}
