//! Directorio experimental de snapshots, gestionado por una autoridad única.
//! Las revisiones contienen una época nueva en cada reinicio de la autoridad;
//! este módulo no proporciona consenso ni sincroniza relojes entre nodos.

use alloc::collections::BTreeMap;

pub const MAX_ENTRADAS: usize = 1024;
pub const MAX_LEASE_MS: u64 = 60 * 60 * 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Revision {
    pub epoca: [u8; 16],
    pub numero: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entrada {
    pub propietario: [u8; 16],
    /// Identidad fuerte de contenido; debe vincular perfil y prefijo.
    pub snapshot: [u8; 32],
    pub revision: Revision,
    /// Tiempo monotónico de la autoridad, exclusivo (caducada si ahora >= vence).
    pub vence_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Reloj,
    Plazo,
    Conflicto,
    Lleno,
    RevisionAgotada,
}

pub struct Directorio {
    epoca: [u8; 16],
    siguiente: u64,
    ultimo_ms: u64,
    entradas: BTreeMap<[u8; 32], Entrada>,
}

impl Directorio {
    /// La época debe ser impredecible o única por instancia y reinicio; si se
    /// reutiliza, una revisión antigua podría coincidir con una posterior.
    pub fn nuevo(epoca: [u8; 16]) -> Self {
        Self { epoca, siguiente: 0, ultimo_ms: 0, entradas: BTreeMap::new() }
    }

    fn reloj(&mut self, ahora_ms: u64) -> Result<(), Error> {
        if ahora_ms < self.ultimo_ms { return Err(Error::Reloj); }
        self.ultimo_ms = ahora_ms;
        Ok(())
    }

    /// Consulta solo leases vivos. El reloj es el de la autoridad, no el del cliente.
    pub fn obtener(&mut self, clave: &[u8; 32], ahora_ms: u64) -> Result<Option<Entrada>, Error> {
        self.reloj(ahora_ms)?;
        Ok(self.entradas.get(clave).copied().filter(|e| ahora_ms < e.vence_ms))
    }

    /// Compare-and-swap: `None` crea únicamente si no existe lease vivo;
    /// `Some(revision)` actualiza exactamente esa revisión viva.
    pub fn publicar(&mut self, clave: [u8; 32], propietario: [u8; 16], snapshot: [u8; 32],
                    esperada: Option<Revision>, ahora_ms: u64, plazo_ms: u64) -> Result<Entrada, Error> {
        self.reloj(ahora_ms)?;
        if plazo_ms == 0 || plazo_ms > MAX_LEASE_MS { return Err(Error::Plazo); }
        let vence = ahora_ms.checked_add(plazo_ms).ok_or(Error::Plazo)?;
        let previa = self.entradas.get(&clave).filter(|e| ahora_ms < e.vence_ms);
        if previa.map(|e| e.revision) != esperada { return Err(Error::Conflicto); }
        if previa.is_none() && self.entradas.len() >= MAX_ENTRADAS {
            self.entradas.retain(|_, e| ahora_ms < e.vence_ms);
            if self.entradas.len() >= MAX_ENTRADAS { return Err(Error::Lleno); }
        }
        let numero = self.siguiente.checked_add(1).ok_or(Error::RevisionAgotada)?;
        let entrada = Entrada { propietario, snapshot, revision: Revision { epoca: self.epoca, numero }, vence_ms: vence };
        self.siguiente = numero;
        self.entradas.insert(clave, entrada);
        Ok(entrada)
    }

    /// Solo la revisión vigente puede retirar una entrada, incluso con el mismo propietario.
    pub fn retirar(&mut self, clave: &[u8; 32], revision: Revision, ahora_ms: u64) -> Result<(), Error> {
        let actual = self.obtener(clave, ahora_ms)?;
        if actual.map(|e| e.revision) != Some(revision) { return Err(Error::Conflicto); }
        self.entradas.remove(clave);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cas_caducidad_y_reinicio_rechazan_propietarios_obsoletos() {
        let mut d = Directorio::nuevo([1; 16]);
        let k = [3; 32];
        let primero = d.publicar(k, [4; 16], [5; 32], None, 100, 20).unwrap();
        assert_eq!(d.publicar(k, [6; 16], [7; 32], None, 101, 10), Err(Error::Conflicto));
        let segundo = d.publicar(k, [6; 16], [7; 32], Some(primero.revision), 102, 10).unwrap();
        assert_eq!(d.retirar(&k, primero.revision, 103), Err(Error::Conflicto));
        assert_eq!(d.obtener(&k, 112).unwrap(), None);
        assert_eq!(d.publicar(k, [4; 16], [5; 32], Some(segundo.revision), 112, 10), Err(Error::Conflicto));
        let tercero = d.publicar(k, [4; 16], [5; 32], None, 112, 10).unwrap();
        assert_ne!(tercero.revision, primero.revision);
        assert_eq!(d.obtener(&k, 111), Err(Error::Reloj));
        let mut nuevo = Directorio::nuevo([2; 16]);
        let reiniciado = nuevo.publicar(k, [4; 16], [5; 32], None, 0, 10).unwrap();
        assert_ne!(reiniciado.revision, primero.revision);
        assert_eq!(nuevo.retirar(&k, primero.revision, 1), Err(Error::Conflicto));
    }

    #[test]
    fn limita_plazos_y_entradas_y_recicla_caducadas() {
        let mut d = Directorio::nuevo([1; 16]);
        assert_eq!(d.publicar([0; 32], [1; 16], [2; 32], None, 0, 0), Err(Error::Plazo));
        assert_eq!(d.publicar([0; 32], [1; 16], [2; 32], None, 0, MAX_LEASE_MS + 1), Err(Error::Plazo));
        for i in 0..MAX_ENTRADAS {
            let mut k = [0; 32];
            k[..8].copy_from_slice(&(i as u64).to_le_bytes());
            d.publicar(k, [1; 16], [2; 32], None, 0, 5).unwrap();
        }
        let mut extra = [0; 32];
        extra[..8].copy_from_slice(&(MAX_ENTRADAS as u64).to_le_bytes());
        assert_eq!(d.publicar(extra, [1; 16], [2; 32], None, 1, 5), Err(Error::Lleno));
        d.publicar(extra, [1; 16], [2; 32], None, 5, 5).unwrap();
        assert!(d.obtener(&extra, 6).unwrap().is_some());
    }
}
