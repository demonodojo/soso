//! Transporte TCP abstracto para el coordinador (T48).
//!
//! El `Transport` que ya hay en `soso-llm-core` devuelve `Result<usize, ()>`:
//! cero bytes puede ser «todavía no hay nada» o «el otro extremo cerró», y el
//! error no dice cuál de las dos cosas pasó. Eso obliga a cada llamante a
//! inventarse una convención, y basta con que dos no coincidan para que un
//! cierre limpio se lea como una espera eterna —o al revés—.
//!
//! Aquí las tres cosas son tres: [`Paso::Hecho`], [`Paso::Espera`] y
//! [`Paso::Fin`]. Y toda espera lleva [`Plazo`], porque C5 no admite esperas
//! sin tope.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

use crate::tiempo::{Plazo, Reloj};
use crate::{Error, Resultado};

/// Qué ocurrió en una operación de entrada/salida.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Paso {
    /// Se movieron `n` bytes. Puede ser **menos** de los pedidos: una
    /// escritura parcial es normal y el llamante tiene que seguir.
    Hecho(usize),
    /// Ahora mismo no hay nada, pero la conexión sigue viva.
    Espera,
    /// El otro extremo cerró. No va a llegar nada más.
    Fin,
}

/// Una conexión ya establecida.
pub trait Transporte {
    /// Escribe lo que pueda **sin bloquear más de lo que diga el adaptador**.
    fn escribir(&mut self, datos: &[u8]) -> Resultado<Paso>;
    /// Lee lo que haya. `espera_ms` es cuánto puede dormir como mucho.
    fn leer(&mut self, buf: &mut [u8], espera_ms: u64) -> Resultado<Paso>;
    /// Cierra. Idempotente: llamarlo dos veces no es un error.
    fn cerrar(&mut self);
}

/// A dónde conectarse. IPv4 porque es lo que la pila de soso tiene.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Destino {
    pub ip: [u8; 4],
    pub puerto: u16,
}

impl Destino {
    pub fn nuevo(ip: [u8; 4], puerto: u16) -> Self {
        Destino { ip, puerto }
    }

    pub fn local(puerto: u16) -> Self {
        Destino {
            ip: [127, 0, 0, 1],
            puerto,
        }
    }

    pub fn texto(&self) -> String {
        format!(
            "{}.{}.{}.{}:{}",
            self.ip[0], self.ip[1], self.ip[2], self.ip[3], self.puerto
        )
    }
}

/// Quien sabe abrir conexiones. El host lo implementa con `std::net`, el guest
/// con las syscalls de soso.
pub trait Conector {
    type Enlace: Transporte;
    fn conectar(&self, destino: &Destino, plazo: Plazo, reloj: &dyn Reloj)
        -> Resultado<Self::Enlace>;
}

/// Cuánto se duerme como mucho en cada vuelta de sondeo. Acotado para poder
/// mirar el plazo —y, más adelante, una cancelación— con frecuencia.
pub const SONDEO_MS: u64 = 20;

/// Escribe **todo**, respetando escrituras parciales y el plazo.
pub fn escribir_todo<T: Transporte, R: Reloj>(
    enlace: &mut T,
    reloj: &R,
    plazo: Plazo,
    datos: &[u8],
) -> Resultado<()> {
    let mut escrito = 0usize;
    while escrito < datos.len() {
        if plazo.vencido(reloj) {
            return Err(Error::plazo(format!(
                "escribiendo: {escrito} de {} byte(s)",
                datos.len()
            )));
        }
        match enlace.escribir(&datos[escrito..])? {
            Paso::Hecho(0) | Paso::Espera => continue,
            Paso::Hecho(n) => escrito += n,
            // Cerrar mientras escribimos no es un plazo agotado ni un éxito a
            // medias: es que no hay a quién escribir.
            Paso::Fin => {
                return Err(Error::entorno(format!(
                    "el otro extremo cerró tras {escrito} de {} byte(s)",
                    datos.len()
                )))
            }
        }
    }
    Ok(())
}

/// Lee hasta que el otro extremo cierre, con tope de bytes y plazo.
///
/// Devuelve lo leído **y** por qué paró, para que el llamante no tenga que
/// adivinar si un mensaje corto es un mensaje corto o una conexión cortada.
pub fn leer_hasta_fin<T: Transporte, R: Reloj>(
    enlace: &mut T,
    reloj: &R,
    plazo: Plazo,
    max_bytes: usize,
) -> Resultado<Vec<u8>> {
    let mut fuera: Vec<u8> = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        if plazo.vencido(reloj) {
            return Err(Error::plazo(format!(
                "leyendo: {} byte(s) recibidos",
                fuera.len()
            )));
        }
        let hueco = max_bytes.saturating_sub(fuera.len());
        if hueco == 0 {
            return Err(Error::uso(format!(
                "la respuesta pasa del tope de {max_bytes} byte(s)"
            )));
        }
        let cabe = buf.len().min(hueco);
        match enlace.leer(&mut buf[..cabe], plazo.espera_ms(reloj, SONDEO_MS))? {
            Paso::Hecho(0) | Paso::Espera => continue,
            Paso::Hecho(n) => fuera.extend_from_slice(&buf[..n]),
            Paso::Fin => return Ok(fuera),
        }
    }
}

/// Lee exactamente `buf.len()` bytes. Un cierre antes de tiempo es un error
/// **distinto** del plazo agotado.
pub fn leer_exacto<T: Transporte, R: Reloj>(
    enlace: &mut T,
    reloj: &R,
    plazo: Plazo,
    buf: &mut [u8],
) -> Resultado<()> {
    let total = buf.len();
    let mut leido = 0usize;
    while leido < total {
        if plazo.vencido(reloj) {
            return Err(Error::plazo(format!(
                "leyendo: {leido} de {total} byte(s)"
            )));
        }
        let espera = plazo.espera_ms(reloj, SONDEO_MS);
        match enlace.leer(&mut buf[leido..], espera)? {
            Paso::Hecho(0) | Paso::Espera => continue,
            Paso::Hecho(n) => leido += n,
            Paso::Fin => {
                return Err(Error::entorno(format!(
                    "el otro extremo cerró tras {leido} de {total} byte(s)"
                )))
            }
        }
    }
    Ok(())
}

/// Transporte de pruebas: se le dicta exactamente qué va a pasar en cada
/// llamada, incluida la fragmentación byte a byte y el cierre temprano.
#[derive(Debug, Default)]
pub struct TransporteSimulado {
    /// Lo que el par «va a decir», en el orden en que lo dirá.
    pub guion: Vec<Paso>,
    /// Bytes que entrega cada `Paso::Hecho` de lectura, en orden.
    pub entrada: Vec<u8>,
    leidos: usize,
    /// Todo lo que se le ha escrito.
    pub escrito: Vec<u8>,
    paso: usize,
    pub cerrado: bool,
    /// Tope por escritura, para simular escrituras parciales.
    pub tope_escritura: usize,
}

impl TransporteSimulado {
    /// Entrega `entrada` de uno en uno y después cierra: el caso más hostil
    /// para un lector que confunda «espera» con «fin».
    pub fn byte_a_byte(entrada: &[u8]) -> Self {
        let mut guion: Vec<Paso> = Vec::new();
        for _ in entrada {
            guion.push(Paso::Espera);
            guion.push(Paso::Hecho(1));
        }
        guion.push(Paso::Fin);
        TransporteSimulado {
            guion,
            entrada: entrada.to_vec(),
            tope_escritura: usize::MAX,
            ..Default::default()
        }
    }

    /// Entrega `corte` bytes y cierra a media respuesta.
    pub fn cierre_temprano(entrada: &[u8], corte: usize) -> Self {
        TransporteSimulado {
            guion: alloc::vec![Paso::Hecho(corte), Paso::Fin],
            entrada: entrada[..corte.min(entrada.len())].to_vec(),
            tope_escritura: usize::MAX,
            ..Default::default()
        }
    }

    /// Nunca dice nada: sirve para comprobar que el plazo vence de verdad.
    pub fn mudo() -> Self {
        TransporteSimulado {
            guion: Vec::new(),
            tope_escritura: usize::MAX,
            ..Default::default()
        }
    }

    fn siguiente(&mut self) -> Paso {
        // Sin guion, el par calla: es «espera», nunca «fin». Confundirlos es
        // exactamente el fallo que este módulo evita.
        let p = self.guion.get(self.paso).copied().unwrap_or(Paso::Espera);
        self.paso += 1;
        p
    }
}

impl Transporte for TransporteSimulado {
    fn escribir(&mut self, datos: &[u8]) -> Resultado<Paso> {
        if self.cerrado {
            return Ok(Paso::Fin);
        }
        let n = datos.len().min(self.tope_escritura);
        self.escrito.extend_from_slice(&datos[..n]);
        Ok(Paso::Hecho(n))
    }

    fn leer(&mut self, buf: &mut [u8], _espera_ms: u64) -> Resultado<Paso> {
        match self.siguiente() {
            Paso::Hecho(n) => {
                let quedan = self.entrada.len() - self.leidos;
                let n = n.min(buf.len()).min(quedan);
                buf[..n].copy_from_slice(&self.entrada[self.leidos..self.leidos + n]);
                self.leidos += n;
                Ok(Paso::Hecho(n))
            }
            otro => Ok(otro),
        }
    }

    fn cerrar(&mut self) {
        self.cerrado = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiempo::RelojSimulado;

    /// Reloj que avanza solo: cada consulta cuesta 1 ms. Así un bucle de
    /// sondeo termina por vencer sin que la prueba tenga que dormir.
    struct RelojQueCorre(RelojSimulado);
    impl Reloj for RelojQueCorre {
        fn ahora_ms(&self) -> u64 {
            let t = self.0.ahora_ms();
            self.0.avanzar(1);
            t
        }
    }

    #[test]
    fn lee_fragmentado_byte_a_byte() {
        let r = RelojSimulado::nuevo(0);
        let mut t = TransporteSimulado::byte_a_byte(b"hola mundo");
        let salida = leer_hasta_fin(&mut t, &r, Plazo::infinito(), 1024).unwrap();
        assert_eq!(salida, b"hola mundo");
    }

    /// Un carácter multibyte partido entre dos lecturas tiene que llegar
    /// entero al final: el transporte mueve bytes, no caracteres.
    #[test]
    fn utf8_partido_se_recompone() {
        let r = RelojSimulado::nuevo(0);
        let texto = "café ☕ ¡hola!";
        let mut t = TransporteSimulado::byte_a_byte(texto.as_bytes());
        let salida = leer_hasta_fin(&mut t, &r, Plazo::infinito(), 1024).unwrap();
        assert_eq!(core::str::from_utf8(&salida).unwrap(), texto);
    }

    /// Cerrar a media respuesta **no** es un plazo agotado, y se tiene que
    /// poder distinguir por el error.
    #[test]
    fn cierre_temprano_no_es_plazo() {
        let r = RelojSimulado::nuevo(0);
        let mut t = TransporteSimulado::cierre_temprano(b"hola mundo", 4);
        let mut buf = [0u8; 10];
        let e = leer_exacto(&mut t, &r, Plazo::infinito(), &mut buf).unwrap_err();
        assert!(matches!(e, Error::Entorno(_)), "{e}");
        assert!(format!("{e}").contains("cerró"), "{e}");

        // Lo ya recibido no se pierde si se lee hasta el fin.
        let mut t = TransporteSimulado::cierre_temprano(b"hola mundo", 4);
        let salida = leer_hasta_fin(&mut t, &r, Plazo::infinito(), 1024).unwrap();
        assert_eq!(salida, b"hola");
    }

    /// Un par que calla agota el plazo; y el error lo dice, no lo disfraza de
    /// cierre.
    #[test]
    fn el_par_mudo_agota_el_plazo() {
        let r = RelojQueCorre(RelojSimulado::nuevo(0));
        let plazo = Plazo::en(&r, 50);
        let mut t = TransporteSimulado::mudo();
        let e = leer_hasta_fin(&mut t, &r, plazo, 1024).unwrap_err();
        assert!(matches!(e, Error::Plazo(_)), "{e}");

        let r2 = RelojQueCorre(RelojSimulado::nuevo(0));
        let plazo2 = Plazo::en(&r2, 50);
        let mut t2 = TransporteSimulado::mudo();
        let mut buf = [0u8; 4];
        let e2 = leer_exacto(&mut t2, &r2, plazo2, &mut buf).unwrap_err();
        assert!(matches!(e2, Error::Plazo(_)), "{e2}");
    }

    /// Escritura parcial: el helper insiste hasta terminar.
    #[test]
    fn escribe_en_trozos_hasta_terminar() {
        let r = RelojSimulado::nuevo(0);
        let mut t = TransporteSimulado::mudo();
        t.tope_escritura = 3;
        escribir_todo(&mut t, &r, Plazo::infinito(), b"doce caracteres").unwrap();
        assert_eq!(t.escrito, b"doce caracteres");
    }

    #[test]
    fn escribir_a_un_cerrado_no_es_plazo() {
        let r = RelojSimulado::nuevo(0);
        let mut t = TransporteSimulado::mudo();
        t.cerrar();
        let e = escribir_todo(&mut t, &r, Plazo::infinito(), b"algo").unwrap_err();
        assert!(matches!(e, Error::Entorno(_)), "{e}");
    }

    #[test]
    fn el_tope_de_bytes_se_respeta() {
        let r = RelojSimulado::nuevo(0);
        let mut t = TransporteSimulado::byte_a_byte(b"0123456789");
        let e = leer_hasta_fin(&mut t, &r, Plazo::infinito(), 4).unwrap_err();
        assert!(format!("{e}").contains("tope"), "{e}");
    }

    #[test]
    fn el_destino_se_escribe_legible() {
        assert_eq!(Destino::local(7422).texto(), "127.0.0.1:7422");
        assert_eq!(Destino::nuevo([10, 0, 2, 15], 22).texto(), "10.0.2.15:22");
    }
}
