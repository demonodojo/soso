use std::io::Read;

struct Contador {
    siguiente: u32,
    fin: u32,
}

impl Iterator for Contador {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        // Incorrecta: al llegar al final vuelve a empezar, así que el iterador
        // nunca se agota y quien lo recorra entero no termina.
        if self.siguiente >= self.fin {
            self.siguiente = 0;
            if self.fin == 0 {
                return None;
            }
        }
        let v = self.siguiente;
        self.siguiente += 1;
        Some(v)
    }
}

fn main() {
    let mut entrada = String::new();
    std::io::stdin().read_to_string(&mut entrada).unwrap();
    let n: u32 = entrada.trim().parse().unwrap();

    let mut it = Contador { siguiente: 0, fin: n };
    let mut piezas: Vec<String> = Vec::new();
    for _ in 0..(n + 3) {
        piezas.push(match it.next() {
            Some(v) => v.to_string(),
            None => String::from("-"),
        });
    }
    println!("{}", piezas.join(" "));
}
