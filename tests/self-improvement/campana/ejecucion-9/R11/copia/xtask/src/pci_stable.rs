//! C5: el registro lxdde guarda `Box<LxPciDev>`; este banco fuerza crecimiento
//! y comprueba que los punteros, drvdata e IRQs de los anteriores siguen vivos.

#[derive(Debug)]
struct FakeDev {
    id: u16,
    data: usize,
    irqs: Vec<u8>,
}

struct Table {
    items: Vec<Box<FakeDev>>,
}

impl Table {
    fn new() -> Self {
        Self { items: Vec::new() }
    }

    fn register(&mut self, dev: FakeDev) -> *mut FakeDev {
        self.items.push(Box::new(dev));
        &mut **self.items.last_mut().unwrap()
    }

    fn drop_failed(&mut self, ptr: *mut FakeDev) {
        if let Some(pos) = self.items.iter().position(|d| core::ptr::eq(&**d, ptr)) {
            self.items.remove(pos);
        }
    }

    fn remove(&mut self, ptr: *mut FakeDev) {
        self.drop_failed(ptr);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn growth_keeps_earlier_pointers_drvdata_irqs() {
        let mut t = Table::new();
        let a = t.register(FakeDev {
            id: 0x1111,
            data: 0xaaa,
            irqs: vec![0x30],
        });
        unsafe {
            (*a).data = 0xaaa;
            (*a).irqs.push(0x31);
        }
        let mut kept = Vec::new();
        kept.push(a);
        for i in 0..64u16 {
            let p = t.register(FakeDev {
                id: i,
                data: i as usize,
                irqs: vec![i as u8],
            });
            kept.push(p);
        }
        unsafe {
            assert_eq!((*a).id, 0x1111);
            assert_eq!((*a).data, 0xaaa);
            assert_eq!((*a).irqs, vec![0x30, 0x31]);
            assert_eq!((*kept[1]).id, 0);
            assert_eq!((**kept.last().unwrap()).id, 63);
        }
    }

    #[test]
    fn failed_probe_and_remove_leave_others() {
        let mut t = Table::new();
        let a = t.register(FakeDev {
            id: 1,
            data: 1,
            irqs: vec![1],
        });
        let fail = t.register(FakeDev {
            id: 2,
            data: 2,
            irqs: vec![2],
        });
        let c = t.register(FakeDev {
            id: 3,
            data: 3,
            irqs: vec![3],
        });
        t.drop_failed(fail);
        unsafe {
            assert_eq!((*a).id, 1);
            assert_eq!((*c).id, 3);
            assert_eq!((*c).data, 3);
        }
        t.remove(a);
        unsafe {
            assert_eq!((*c).id, 3);
            assert_eq!((*c).irqs, vec![3]);
        }
        assert_eq!(t.items.len(), 1);
    }
}
