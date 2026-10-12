//! Editor de texto mínimo para soso: flechas, insertar, borrar, Ctrl-S guardar, Ctrl-Q salir.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use libsoso::{abi, print, println, sys};

libsoso::entry!(main);

const MAX_LINEA: usize = 4096;
const MAX_LINEAS: usize = 4096;

struct Editor {
    lineas: Vec<String>,
    fila: usize,
    col: usize,
    modificado: bool,
    nombre: String,
    fila_scroll: usize,
}

impl Editor {
    fn nueva(path: &str) -> Self {
        Editor {
            lineas: Vec::new(),
            fila: 0,
            col: 0,
            modificado: false,
            nombre: String::from(path),
            fila_scroll: 0,
        }
    }

    fn cargar(&mut self) -> bool {
        let fd = sys::open(&self.nombre, abi::O_RDONLY);
        if fd < 0 {
            self.lineas.push(String::new());
            return true;
        }
        let mut buf = [0u8; 4096];
        let mut todo = Vec::new();
        loop {
            let n = sys::read(fd as u64, &mut buf);
            if n < 0 {
                sys::close(fd as u64);
                return false;
            }
            if n == 0 {
                break;
            }
            todo.extend_from_slice(&buf[..n as usize]);
        }
        sys::close(fd as u64);
        let texto = core::str::from_utf8(&todo).unwrap_or("");
        if texto.is_empty() {
            self.lineas.push(String::new());
        } else {
            for linea in texto.split_inclusive('\n') {
                let mut s = String::from(linea);
                if s.ends_with('\n') {
                    s.pop();
                }
                self.lineas.push(s);
            }
        }
        if self.lineas.is_empty() {
            self.lineas.push(String::new());
        }
        true
    }

    fn guardar(&mut self) -> bool {
        let mut data = Vec::new();
        for (i, l) in self.lineas.iter().enumerate() {
            data.extend_from_slice(l.as_bytes());
            if i + 1 < self.lineas.len() || self.modificado {
                data.push(b'\n');
            }
        }
        let fd = sys::open(
            &self.nombre,
            abi::O_WRONLY | abi::O_CREAT | abi::O_TRUNC,
        );
        if fd < 0 {
            return false;
        }
        if !data.is_empty() && sys::write_all(fd as u64, &data).is_err() {
            sys::close(fd as u64);
            return false;
        }
        sys::close(fd as u64);
        self.modificado = false;
        true
    }

    fn insertar(&mut self, c: u8) {
        if c < 0x20 && c != b'\t' {
            return;
        }
        self.modificado = true;
        if self.fila >= self.lineas.len() {
            return;
        }
        let l = &mut self.lineas[self.fila];
        if l.len() >= MAX_LINEA {
            return;
        }
        if self.col > l.len() {
            self.col = l.len();
        }
        l.insert(self.col, c as char);
        self.col += 1;
    }

    fn borrar(&mut self) {
        if self.fila >= self.lineas.len() {
            return;
        }
        let l = &mut self.lineas[self.fila];
        if self.col > 0 && self.col <= l.len() {
            l.remove(self.col - 1);
            self.col -= 1;
            self.modificado = true;
        } else if self.fila > 0 {
            let actual = self.lineas.remove(self.fila);
            self.fila -= 1;
            self.col = self.lineas[self.fila].len();
            self.lineas[self.fila].push_str(&actual);
            self.modificado = true;
        }
    }

    fn nueva_linea(&mut self) {
        if self.lineas.len() >= MAX_LINEAS {
            return;
        }
        self.modificado = true;
        let resto = self.lineas[self.fila].split_off(self.col);
        self.fila += 1;
        self.lineas.insert(self.fila, resto);
        self.col = 0;
    }

    fn mover(&mut self, df: i32, dc: i32) {
        if df < 0 && self.fila > 0 {
            self.fila -= 1;
        } else if df > 0 && self.fila + 1 < self.lineas.len() {
            self.fila += 1;
        }
        let len = self.lineas[self.fila].len();
        if dc < 0 {
            self.col = self.col.saturating_sub(1);
        } else if dc > 0 {
            self.col = (self.col + 1).min(len);
        } else {
            self.col = self.col.min(len);
        }
    }

    fn pintar(&self) {
        let _ = sys::write_all(1, b"\x1b[2J\x1b[H");
        let barra = if self.modificado { '*' } else { ' ' };
        println!(
            "soso-ed {barra} {} — fila {} col {} — Ctrl-S guardar, Ctrl-Q salir",
            self.nombre,
            self.fila + 1,
            self.col + 1
        );
        println!("────────────────────────────────────────");
        let visibles = 20usize;
        if self.fila >= self.fila_scroll + visibles {
            // scroll implícito vía fila_scroll no usado aún
        }
        let fin = (self.fila_scroll + visibles).min(self.lineas.len());
        for i in self.fila_scroll..fin {
            let pref = if i == self.fila { '>' } else { ' ' };
            let l = &self.lineas[i];
            if i == self.fila {
                let (a, b) = l.split_at(self.col.min(l.len()));
                print!("{pref} {a}\x1b[7m");
                if self.col < l.len() {
                    print!("{}", l.as_bytes()[self.col] as char);
                } else {
                    print!(" ");
                }
                print!("\x1b[0m{b}\n");
            } else {
                println!("{pref} {l}");
            }
        }
    }
}

enum Tecla {
    Char(u8),
    Arriba,
    Abajo,
    Izq,
    Derecha,
    Desconocida,
}

fn leer_tecla() -> Option<Tecla> {
    let mut b = [0u8; 1];
    let n = sys::read(0, &mut b);
    if n <= 0 {
        return None;
    }
    match b[0] {
        0x03 => return None, // Ctrl-C
        0x11 => return Some(Tecla::Char(0x11)), // Ctrl-Q
        0x13 => return Some(Tecla::Char(0x13)), // Ctrl-S
        0x0d | 0x0a => return Some(Tecla::Char(b'\n')),
        0x08 | 0x7f => return Some(Tecla::Char(0x08)),
        0x1b => {
            let mut seq = [0u8; 2];
            if sys::read(0, &mut seq[..1]) <= 0 {
                return Some(Tecla::Desconocida);
            }
            if seq[0] != b'[' {
                return Some(Tecla::Desconocida);
            }
            if sys::read(0, &mut seq[1..2]) <= 0 {
                return Some(Tecla::Desconocida);
            }
            return Some(match seq[1] {
                b'A' => Tecla::Arriba,
                b'B' => Tecla::Abajo,
                b'C' => Tecla::Derecha,
                b'D' => Tecla::Izq,
                _ => Tecla::Desconocida,
            });
        }
        c @ 0x20..=0x7e => Some(Tecla::Char(c)),
        c => Some(Tecla::Char(c)),
    }
}

fn main(args: &str) -> u8 {
    let path = args.trim();
    if path.is_empty() {
        println!("uso: soso-ed RUTA");
        return 2;
    }
    let mut ed = Editor::nueva(path);
    if !ed.cargar() {
        println!("soso-ed: no se pudo leer {path}");
        return 1;
    }
    loop {
        ed.pintar();
        let Some(t) = leer_tecla() else {
            break;
        };
        match t {
            Tecla::Char(0x11) => break, // Ctrl-Q
            Tecla::Char(0x13) => {
                if !ed.guardar() {
                    println!("\nsoso-ed: error al guardar");
                }
            }
            Tecla::Char(b'\n') => ed.nueva_linea(),
            Tecla::Char(0x08) => ed.borrar(),
            Tecla::Char(c) => ed.insertar(c),
            Tecla::Arriba => ed.mover(-1, 0),
            Tecla::Abajo => ed.mover(1, 0),
            Tecla::Izq => ed.mover(0, -1),
            Tecla::Derecha => ed.mover(0, 1),
            Tecla::Desconocida => {}
        }
    }
    if ed.modificado {
        println!("\n(sin guardar — vuelve con Ctrl-S antes de salir si quieres conservar cambios)");
    }
    0
}
