//! U1: banco del escritor de logs. Reproduce en host lo que hace el kernel —
//! leer el ring por cursor, rotar y anexar— para poder comprobar las reglas
//! que dentro del kernel no tienen forma de probarse.

use soso_log_core::flujo::{Estado, Flujo, LOTE_MAX, MAX_FALLOS, REINTENTO_MS};
use soso_log_core::ring::Ring;
use soso_log_core::rotacion::{self, Accion, Paso, MAX_ACTIVO, ROTACIONES};
use soso_log_core::texto::{self, Cabecera};

/// Destino en memoria con la misma forma que `/var/log/<flujo>{,.1,.2,.3}`.
#[derive(Default)]
struct Destino {
    activo: Vec<u8>,
    historico: Vec<Vec<u8>>,
    fallar: bool,
    rotaciones: usize,
}

impl Destino {
    fn rotar(&mut self) {
        self.historico.insert(0, core::mem::take(&mut self.activo));
        self.historico.truncate(ROTACIONES as usize);
        self.rotaciones += 1;
    }
    fn anexar(&mut self, bytes: &[u8]) -> Result<(), ()> {
        if self.fallar {
            return Err(());
        }
        self.activo.extend_from_slice(bytes);
        Ok(())
    }
    /// Todo lo persistido, del más viejo al más nuevo.
    fn todo(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for h in self.historico.iter().rev() {
            out.extend_from_slice(h);
        }
        out.extend_from_slice(&self.activo);
        out
    }
}

/// Una pasada del escritor. Devuelve `true` si queda material pendiente.
fn pasada<const CAP: usize>(f: &mut Flujo, r: &Ring<CAP>, d: &mut Destino, ahora_ms: u64) -> bool {
    if !f.disponible(ahora_ms) {
        return false;
    }
    let Some(trabajo) = f.planificar(r.escritos(), r.cursor_minimo()) else {
        return false;
    };

    // El lote se copia del ring y **después** se escribe: en el kernel esto es
    // soltar el candado del ring antes de tocar el disco.
    let mut lote = vec![0u8; trabajo.bytes];
    let lectura = r.leer_desde(f.cursor, &mut lote);

    let mut salida = Vec::new();
    if trabajo.marcar_perdida > 0 {
        let mut marca = [0u8; 96];
        let n = texto::marca_perdida(&mut marca, trabajo.marcar_perdida);
        salida.extend_from_slice(&marca[..n]);
    }
    salida.extend_from_slice(&lote[..lectura.copiados]);

    let roto = trabajo.accion == Accion::RotarYAnexar;
    if roto {
        d.rotar();
    }
    match d.anexar(&salida) {
        Ok(()) => f.exito(&lectura, salida.len() as u64, roto),
        Err(()) => {
            f.fallo(ahora_ms, salida.len());
            return false;
        }
    }
    trabajo.mas_pendiente
}

fn drenar<const CAP: usize>(f: &mut Flujo, r: &Ring<CAP>, d: &mut Destino, ahora_ms: u64) {
    for _ in 0..1000 {
        if !pasada(f, r, d, ahora_ms) {
            return;
        }
    }
    panic!("el drenado no termina");
}

// ── Cursor y pérdidas ────────────────────────────────────────────────────

#[test]
fn lo_escrito_llega_entero_y_sin_duplicados() {
    let mut r: Ring<4096> = Ring::new();
    let mut f = Flujo::nuevo("kernel.log");
    let mut d = Destino::default();
    f.activar(0);

    for i in 0..10u32 {
        r.append(format!("linea {i}\n").as_bytes());
        drenar(&mut f, &r, &mut d, 1000);
    }
    let texto = String::from_utf8(d.todo()).unwrap();
    for i in 0..10u32 {
        assert_eq!(texto.matches(&format!("linea {i}\n")).count(), 1, "{texto}");
    }
    assert_eq!(f.perdidos, 0);
}

#[test]
fn el_ring_lleno_sigue_persistiendo() {
    // Es la regresión concreta que motiva U1: `fatlog::poll` comparaba `len()`,
    // que se satura, y al llenarse el ring dejaba de detectar bytes nuevos.
    let mut r: Ring<256> = Ring::new();
    let mut f = Flujo::nuevo("kernel.log");
    let mut d = Destino::default();
    f.activar(0);

    // Llenar el ring del todo antes del primer volcado.
    for i in 0..100u32 {
        r.append(format!("l{i:03}\n").as_bytes());
    }
    assert_eq!(r.byte_len(), 256, "el ring tiene que estar lleno");
    drenar(&mut f, &r, &mut d, 1000);
    let tras_llenar = d.todo().len();
    assert!(tras_llenar > 0);

    // Y a partir de ahí cada línea nueva se sigue persistiendo.
    for i in 100..140u32 {
        r.append(format!("l{i:03}\n").as_bytes());
        drenar(&mut f, &r, &mut d, 1000);
    }
    let texto = String::from_utf8(d.todo()).unwrap();
    assert!(texto.contains("l139\n"), "la última línea no llegó: {texto}");
    assert!(d.todo().len() > tras_llenar);
}

#[test]
fn la_perdida_se_marca_una_vez_y_no_se_inventan_datos() {
    let mut r: Ring<64> = Ring::new();
    let mut f = Flujo::nuevo("kernel.log");
    let mut d = Destino::default();
    f.activar(0);

    r.append(b"A".repeat(40).as_slice());
    drenar(&mut f, &r, &mut d, 1000);
    assert_eq!(f.perdidos, 0);

    // Ráfaga que da la vuelta al ring sin que nadie haya drenado.
    r.append(b"B".repeat(200).as_slice());
    drenar(&mut f, &r, &mut d, 1000);

    let texto = String::from_utf8(d.todo()).unwrap();
    // 240 escritos, 64 vivos: el byte más antiguo del ring es el 176, y como
    // el cursor iba por el 40, el hueco es de 136 bytes.
    assert_eq!(f.perdidos, 136);
    assert_eq!(texto.matches("se perdieron").count(), 1, "{texto}");
    assert!(texto.contains("se perdieron 136 bytes"));
    // Lo que sí quedaba en el ring se persiste una sola vez.
    assert_eq!(texto.matches('B').count(), 64);

    // Una pasada más sin nada nuevo no vuelve a marcar nada.
    drenar(&mut f, &r, &mut d, 2000);
    let texto = String::from_utf8(d.todo()).unwrap();
    assert_eq!(texto.matches("se perdieron").count(), 1);
}

#[test]
fn el_cursor_no_retrocede_ni_relee() {
    let mut r: Ring<1024> = Ring::new();
    r.append(b"hola");
    let mut out = [0u8; 16];
    let l1 = r.leer_desde(0, &mut out);
    assert_eq!(&out[..l1.copiados], b"hola");
    assert_eq!(l1.cursor, 4);
    assert_eq!(l1.pendientes, 0);
    // Sin novedades, la siguiente lectura no devuelve nada.
    let l2 = r.leer_desde(l1.cursor, &mut out);
    assert_eq!(l2.copiados, 0);
    assert_eq!(l2.cursor, 4);
    // Un cursor por delante del ring (fichero de otro arranque) se recorta.
    let l3 = r.leer_desde(9999, &mut out);
    assert_eq!(l3.copiados, 0);
    assert_eq!(l3.cursor, 4);
}

#[test]
fn el_lote_esta_acotado() {
    let mut r: Ring<{ 256 * 1024 }> = Ring::new();
    let mut f = Flujo::nuevo("kernel.log");
    f.activar(0);
    r.append(&vec![b'x'; 200 * 1024]);
    let t = f.planificar(r.escritos(), r.cursor_minimo()).unwrap();
    assert_eq!(t.bytes, LOTE_MAX, "una pasada no puede monopolizar el disco");
    assert!(t.mas_pendiente);

    // Y aun así se drena entero en varias pasadas.
    let mut d = Destino::default();
    drenar(&mut f, &r, &mut d, 1000);
    assert_eq!(d.todo().len(), 200 * 1024);
}

// ── Rotación ─────────────────────────────────────────────────────────────

#[test]
fn rota_al_llenarse_y_conserva_el_historico() {
    assert_eq!(rotacion::decidir(0, 5_000_000), Accion::Anexar, "activo vacío");
    assert_eq!(rotacion::decidir(MAX_ACTIVO - 10, 10), Accion::Anexar);
    assert_eq!(rotacion::decidir(MAX_ACTIVO - 10, 11), Accion::RotarYAnexar);
}

#[test]
fn un_lote_gigante_rota_una_vez_y_se_escribe_entero() {
    // Si un lote mayor que el máximo provocara rotación en bucle, el historial
    // se borraría entero sin llegar a escribir nada.
    let mut d = Destino::default();
    let mut f = Flujo::nuevo("kernel.log");
    f.activar(MAX_ACTIVO);
    let mut r: Ring<{ 256 * 1024 }> = Ring::new();
    r.append(&vec![b'y'; 200 * 1024]);
    drenar(&mut f, &r, &mut d, 1000);
    assert_eq!(d.rotaciones, 1);
    assert_eq!(d.activo.len(), 200 * 1024);
}

#[test]
fn el_historico_no_crece_sin_limite() {
    let mut d = Destino::default();
    let mut f = Flujo::nuevo("kernel.log");
    let mut r: Ring<4096> = Ring::new();
    f.activar(0);
    for _ in 0..20 {
        f.activo_bytes = MAX_ACTIVO;
        r.append(&vec![b'z'; 1024]);
        drenar(&mut f, &r, &mut d, 1000);
    }
    assert_eq!(d.historico.len(), ROTACIONES as usize);
}

#[test]
fn los_pasos_de_rotacion_van_del_mas_viejo_al_mas_nuevo() {
    // Al revés, `.1 → .2` pisaría un `.2` que todavía hay que conservar.
    assert_eq!(
        rotacion::pasos(),
        [
            Paso::Borrar(3),
            Paso::Renombrar { de: 2, a: 3 },
            Paso::Renombrar { de: 1, a: 2 },
            Paso::Renombrar { de: 0, a: 1 },
        ]
    );
    let mut buf = [0u8; 4];
    assert_eq!(rotacion::sufijo(0, &mut buf), 0);
    assert_eq!(rotacion::sufijo(2, &mut buf), 2);
    assert_eq!(&buf[..2], b".2");
}

// ── Errores del sistema de ficheros ──────────────────────────────────────

#[test]
fn un_disco_que_falla_no_bloquea_ni_pierde_el_ring() {
    let mut r: Ring<4096> = Ring::new();
    let mut f = Flujo::nuevo("kernel.log");
    let mut d = Destino { fallar: true, ..Default::default() };
    f.activar(0);

    r.append(b"algo que registrar\n");
    for _ in 0..MAX_FALLOS {
        assert!(!pasada(&mut f, &r, &mut d, 1000));
    }
    assert!(matches!(f.estado, Estado::Suspendido { .. }), "{:?}", f.estado);
    assert_eq!(f.fallos_totales, MAX_FALLOS);
    assert!(f.descartados > 0);
    // Suspendido: no se vuelve a intentar en cada vuelta del planificador.
    assert!(!f.disponible(1001));
    assert!(!pasada(&mut f, &r, &mut d, 1001));
    assert_eq!(f.fallos_totales, MAX_FALLOS, "no insiste contra el mismo error");

    // Pasado el plazo se reintenta, y si el disco vuelve, se persiste lo que
    // siga vivo en el ring: el cursor nunca avanzó sobre lo no escrito.
    d.fallar = false;
    assert!(f.disponible(1000 + REINTENTO_MS));
    drenar(&mut f, &r, &mut d, 1000 + REINTENTO_MS);
    assert_eq!(f.estado, Estado::Activo);
    assert_eq!(String::from_utf8(d.todo()).unwrap(), "algo que registrar\n");
}

#[test]
fn sin_destino_no_se_escribe_ni_se_avanza_el_cursor() {
    let mut r: Ring<4096> = Ring::new();
    let mut f = Flujo::nuevo("kernel.log"); // Inactivo: sosofs sin montar
    let mut d = Destino::default();
    r.append(b"traza temprana\n");
    assert!(!pasada(&mut f, &r, &mut d, 10));
    assert_eq!(f.cursor, 0);
    assert!(d.todo().is_empty());

    // Al montar, lo capturado en RAM se vuelca entero.
    f.activar(0);
    drenar(&mut f, &r, &mut d, 20);
    assert_eq!(String::from_utf8(d.todo()).unwrap(), "traza temprana\n");
}

// ── Cabeceras ────────────────────────────────────────────────────────────

#[test]
fn la_cabecera_identifica_el_arranque() {
    let mut buf = [0u8; 256];
    let n = texto::cabecera(
        &mut buf,
        &Cabecera {
            flujo: "kernel.log",
            boot_id: 0x1234_5678_9abc_def0,
            version: "0.3.0",
            uptime_ms: 1234,
            fecha: None,
        },
    );
    let s = core::str::from_utf8(&buf[..n]).unwrap();
    assert_eq!(
        s,
        "=== soso 0.3.0 arranque 123456789abcdef0 flujo kernel.log monotónico 1234ms ===\n"
    );
    // Con reloj utilizable se añade la fecha; sin él no se inventa.
    let n = texto::cabecera(
        &mut buf,
        &Cabecera {
            flujo: "kernel.log",
            boot_id: 1,
            version: "0.3.0",
            uptime_ms: 5,
            fecha: Some("2026-09-16T10:00:00Z"),
        },
    );
    assert!(core::str::from_utf8(&buf[..n]).unwrap().contains("fecha 2026-09-16T10:00:00Z"));
}

#[test]
fn las_marcas_no_desbordan_un_buffer_corto() {
    let mut corto = [0u8; 8];
    let n = texto::marca_perdida(&mut corto, 999_999);
    assert_eq!(n, 8, "se trunca, no se sale del buffer");
    let n = texto::marca_fallo(&mut corto, "kernel.log", 3);
    assert_eq!(n, 8);
}
