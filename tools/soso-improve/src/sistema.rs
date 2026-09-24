//! El entorno del host: archivos con `std::fs` y procesos con `std::process`.
//!
//! Es la mitad no portable del coordinador, y por eso está aquí y no en el
//! crate. Cuando exista `user/soso-improve`, su equivalente usará `spawn_io` y
//! `wait` de libsoso sin tocar una línea de la lógica.

use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use soso_improve_core::entorno::{Archivos, Entrada, Orden, Procesos, Salida, Tipo};
use soso_improve_core::{Error, Resultado};

pub struct Host;

fn traducir<T>(r: std::io::Result<T>, que: &str) -> Resultado<T> {
    r.map_err(|e| Error::entorno(format!("{que}: {e}")))
}

impl Archivos for Host {
    fn leer(&self, ruta: &str) -> Resultado<Vec<u8>> {
        traducir(std::fs::read(ruta), &format!("leer {ruta}"))
    }

    fn escribir(&mut self, ruta: &str, datos: &[u8], modo: u32) -> Resultado<()> {
        if let Some(padre) = Path::new(ruta).parent() {
            traducir(
                std::fs::create_dir_all(padre),
                &format!("crear {}", padre.display()),
            )?;
        }
        traducir(std::fs::write(ruta, datos), &format!("escribir {ruta}"))?;
        if modo != 0 {
            traducir(
                std::fs::set_permissions(ruta, std::fs::Permissions::from_mode(modo)),
                &format!("permisos de {ruta}"),
            )?;
        }
        Ok(())
    }

    fn existe(&self, ruta: &str) -> bool {
        Path::new(ruta).exists()
    }

    fn listar(&self, ruta: &str) -> Resultado<Vec<Entrada>> {
        let mut fuera = Vec::new();
        let iter = traducir(std::fs::read_dir(ruta), &format!("listar {ruta}"))?;
        for entrada in iter {
            let entrada = traducir(entrada, &format!("listar {ruta}"))?;
            let nombre = entrada.file_name().to_string_lossy().into_owned();
            let meta = traducir(
                entrada.metadata(),
                &format!("metadatos de {}", entrada.path().display()),
            )?;
            // `metadata()` sigue enlaces; `file_type()` no. Un enlace se
            // registra como «otro»: se declara, no se copia.
            let tipo_real = traducir(entrada.file_type(), "tipo")?;
            let tipo = if tipo_real.is_symlink() {
                Tipo::Otro
            } else if meta.is_dir() {
                Tipo::Directorio
            } else if meta.is_file() {
                Tipo::Archivo
            } else {
                Tipo::Otro
            };
            fuera.push(Entrada {
                ruta: nombre,
                tipo,
                bytes: if tipo == Tipo::Archivo { meta.len() } else { 0 },
                modo: meta.permissions().mode() & 0o777,
            });
        }
        Ok(fuera)
    }

    fn metadatos(&self, ruta: &str) -> Resultado<Entrada> {
        let meta = traducir(std::fs::symlink_metadata(ruta), &format!("stat {ruta}"))?;
        let tipo = if meta.file_type().is_symlink() {
            Tipo::Otro
        } else if meta.is_dir() {
            Tipo::Directorio
        } else if meta.is_file() {
            Tipo::Archivo
        } else {
            Tipo::Otro
        };
        Ok(Entrada {
            ruta: ruta.to_string(),
            tipo,
            bytes: meta.len(),
            modo: meta.permissions().mode() & 0o777,
        })
    }

    fn crear_directorio(&mut self, ruta: &str) -> Resultado<()> {
        traducir(std::fs::create_dir_all(ruta), &format!("crear {ruta}"))
    }

    fn borrar(&mut self, ruta: &str) -> Resultado<()> {
        let p = Path::new(ruta);
        if p.is_dir() {
            traducir(std::fs::remove_dir_all(p), &format!("borrar {ruta}"))
        } else {
            traducir(std::fs::remove_file(p), &format!("borrar {ruta}"))
        }
    }
}

impl Procesos for Host {
    fn ejecutar(&self, orden: &Orden) -> Resultado<Salida> {
        if orden.argv.is_empty() {
            return Err(Error::uso("argv vacío"));
        }
        let mut cmd = Command::new(&orden.argv[0]);
        cmd.args(&orden.argv[1..])
            .current_dir(&orden.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in &orden.entorno {
            cmd.env(k, v);
        }
        let mut hijo = match cmd.spawn() {
            Ok(h) => h,
            Err(e) => {
                return Ok(Salida {
                    codigo: None,
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                    motivo: Some(format!("no se pudo lanzar {}: {e}", orden.argv[0])),
                })
            }
        };
        if let Some(mut stdin) = hijo.stdin.take() {
            let datos = orden.stdin.clone();
            // Escribir en otro hilo evita el bloqueo mutuo clásico cuando el
            // hijo llena su tubería de salida mientras espera entrada.
            std::thread::spawn(move || {
                let _ = stdin.write_all(&datos);
            });
        }
        let (mut salida, mut error) = (Vec::new(), Vec::new());
        let mut fin_stdout = hijo.stdout.take();
        let mut fin_stderr = hijo.stderr.take();
        let hilo_err = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(e) = fin_stderr.as_mut() {
                let _ = e.read_to_end(&mut buf);
            }
            buf
        });
        if let Some(s) = fin_stdout.as_mut() {
            let _ = s.read_to_end(&mut salida);
        }
        error.extend(hilo_err.join().unwrap_or_default());

        let estado = traducir(hijo.wait(), "esperar al proceso")?;
        Ok(Salida {
            codigo: estado.code(),
            stdout: salida,
            stderr: error,
            motivo: if estado.code().is_none() {
                Some("terminado por señal".to_string())
            } else {
                None
            },
        })
    }
}

/// Directorio temporal que se borra solo.
pub struct Temporal(pub PathBuf);

impl Temporal {
    pub fn nuevo(prefijo: &str) -> Resultado<Temporal> {
        let base = std::env::temp_dir();
        let unico = format!(
            "{prefijo}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let ruta = base.join(unico);
        traducir(std::fs::create_dir_all(&ruta), "crear temporal")?;
        Ok(Temporal(ruta))
    }

    pub fn ruta(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
}

impl Drop for Temporal {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---------------------------------------------------------------------------
// Reloj y transporte del host (T48)
// ---------------------------------------------------------------------------

use soso_improve_core::tiempo::{Plazo, Reloj};
use soso_improve_core::transporte::{Conector, Destino, Paso, Transporte};

/// Reloj monotónico del host. `Instant` no retrocede aunque cambie la hora.
pub struct RelojHost {
    origen: std::time::Instant,
}

impl Default for RelojHost {
    fn default() -> Self {
        RelojHost {
            origen: std::time::Instant::now(),
        }
    }
}

impl Reloj for RelojHost {
    fn ahora_ms(&self) -> u64 {
        self.origen.elapsed().as_millis() as u64
    }
}

/// Conexión TCP del host. El socket va en modo no bloqueante para poder
/// distinguir «todavía nada» de «el otro cerró», que es lo que pide T48.
pub struct EnlaceHost {
    flujo: Option<std::net::TcpStream>,
}

impl Transporte for EnlaceHost {
    fn escribir(&mut self, datos: &[u8]) -> Resultado<Paso> {
        let Some(f) = self.flujo.as_mut() else {
            return Ok(Paso::Fin);
        };
        match f.write(datos) {
            Ok(0) => Ok(Paso::Fin),
            Ok(n) => Ok(Paso::Hecho(n)),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(Paso::Espera),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => Ok(Paso::Espera),
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(Paso::Fin),
            Err(e) => Err(Error::entorno(format!("escribir: {e}"))),
        }
    }

    fn leer(&mut self, buf: &mut [u8], espera_ms: u64) -> Resultado<Paso> {
        let Some(f) = self.flujo.as_mut() else {
            return Ok(Paso::Fin);
        };
        match f.read(buf) {
            // Cero bytes en una lectura **bloqueante o no** significa EOF en
            // POSIX; `WouldBlock` es la espera. No son lo mismo.
            Ok(0) => Ok(Paso::Fin),
            Ok(n) => Ok(Paso::Hecho(n)),
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::Interrupted =>
            {
                if espera_ms > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(espera_ms.min(50)));
                }
                Ok(Paso::Espera)
            }
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => Ok(Paso::Fin),
            Err(e) => Err(Error::entorno(format!("leer: {e}"))),
        }
    }

    fn cerrar(&mut self) {
        if let Some(f) = self.flujo.take() {
            let _ = f.shutdown(std::net::Shutdown::Both);
        }
    }
}

pub struct ConectorHost;

impl Conector for ConectorHost {
    type Enlace = EnlaceHost;

    fn conectar(
        &self,
        destino: &Destino,
        plazo: Plazo,
        reloj: &dyn Reloj,
    ) -> Resultado<EnlaceHost> {
        let dir = std::net::SocketAddr::from((destino.ip, destino.puerto));
        // El plazo manda también en la conexión, no sólo en la E/S.
        let restante = plazo.restante_ms(reloj);
        let espera = std::time::Duration::from_millis(restante.min(30_000).max(1));
        let flujo = std::net::TcpStream::connect_timeout(&dir, espera).map_err(|e| {
            if e.kind() == std::io::ErrorKind::TimedOut {
                Error::plazo(format!("conectando a {}: {e}", destino.texto()))
            } else {
                Error::entorno(format!("conectar a {}: {e}", destino.texto()))
            }
        })?;
        flujo
            .set_nonblocking(true)
            .map_err(|e| Error::entorno(format!("no bloqueante: {e}")))?;
        Ok(EnlaceHost { flujo: Some(flujo) })
    }
}

// ---------------------------------------------------------------------------
// Durabilidad en el host (T46)
// ---------------------------------------------------------------------------

use soso_improve_core::durable::Durable;

impl Durable for Host {
    fn crear_exclusivo(&mut self, ruta: &str, datos: &[u8]) -> Resultado<()> {
        if let Some(padre) = Path::new(ruta).parent() {
            traducir(
                std::fs::create_dir_all(padre),
                &format!("crear {}", padre.display()),
            )?;
        }
        // `create_new` es la creación exclusiva: si existe, falla, y eso es
        // justo la señal de que otro escritor ganó la carrera.
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(ruta)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    Error::uso(format!("{ruta} ya existe"))
                } else {
                    Error::entorno(format!("crear {ruta}: {e}"))
                }
            })?;
        traducir(f.write_all(datos), &format!("escribir {ruta}"))?;
        // Sin esto la promesa es falsa: el contenido puede seguir en caché.
        traducir(f.sync_all(), &format!("sincronizar {ruta}"))?;
        Ok(())
    }

    fn sincronizar(&mut self, ruta: &str) -> Resultado<()> {
        let f = traducir(std::fs::File::open(ruta), &format!("abrir {ruta}"))?;
        traducir(f.sync_all(), &format!("sincronizar {ruta}"))
    }
}
