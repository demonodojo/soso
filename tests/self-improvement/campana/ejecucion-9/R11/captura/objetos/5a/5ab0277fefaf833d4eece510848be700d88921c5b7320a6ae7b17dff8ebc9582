//! C6: reglas de mprotect parcial y preflight ELF, comprobables en host.

#[derive(Clone, Debug, PartialEq)]
struct Reg {
    start: u64,
    len: u64,
    off: u64,
    writable: bool,
}

fn split_prot(regs: &mut Vec<Reg>, addr: u64, len: u64, writable: bool) -> bool {
    let end = match addr.checked_add(len) {
        Some(e) => e,
        None => return false,
    };
    let mut out = Vec::new();
    let mut hit = false;
    for r in regs.drain(..) {
        let rend = r.start.saturating_add(r.len);
        if rend <= addr || r.start >= end {
            out.push(r);
            continue;
        }
        hit = true;
        if r.start < addr {
            out.push(Reg {
                start: r.start,
                len: addr - r.start,
                off: r.off,
                writable: r.writable,
            });
        }
        let mid_start = r.start.max(addr);
        let mid_end = rend.min(end);
        if mid_end > mid_start {
            out.push(Reg {
                start: mid_start,
                len: mid_end - mid_start,
                off: r.off.saturating_add(mid_start - r.start),
                writable,
            });
        }
        if rend > end {
            out.push(Reg {
                start: end,
                len: rend - end,
                off: r.off.saturating_add(end - r.start),
                writable: r.writable,
            });
        }
    }
    *regs = out;
    hit
}

fn load_ok(filesz: u64, memsz: u64, offset: u64, vaddr: u64, file_limit: u64) -> bool {
    filesz <= memsz
        && offset.checked_add(filesz).is_some_and(|e| e <= file_limit)
        && (vaddr & 0xfff) == (offset & 0xfff)
        && (vaddr & 0xfff) <= offset
}

fn entry_in_exec(entry: u64, loads: &[(u64, u64, bool)]) -> bool {
    loads
        .iter()
        .any(|(v, m, x)| *x && entry >= *v && entry < *v + *m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mprotect_interior_unfaulted_splits_region() {
        let mut regs = vec![Reg {
            start: 0x1000,
            len: 0x3000,
            off: 0,
            writable: true,
        }];
        assert!(split_prot(&mut regs, 0x2000, 0x1000, false));
        assert_eq!(
            regs,
            vec![
                Reg {
                    start: 0x1000,
                    len: 0x1000,
                    off: 0,
                    writable: true
                },
                Reg {
                    start: 0x2000,
                    len: 0x1000,
                    off: 0x1000,
                    writable: false
                },
                Reg {
                    start: 0x3000,
                    len: 0x1000,
                    off: 0x2000,
                    writable: true
                },
            ]
        );
    }

    #[test]
    fn elf_preflight_rejects_gap_overflow_incongruent() {
        assert!(!load_ok(0x20, 0x10, 0, 0x400000, 0x100));
        assert!(!load_ok(0x10, 0x20, 0x100, 0x400000, 0x80));
        assert!(!load_ok(0x10, 0x20, 0x1, 0x400000, 0x100));
        assert!(load_ok(0x10, 0x20, 0, 0x400000, 0x100));
        let loads = [(0x400000, 0x1000, true), (0x402000, 0x1000, false)];
        assert!(entry_in_exec(0x400100, &loads));
        assert!(!entry_in_exec(0x401000, &loads));
        assert!(!entry_in_exec(0x402100, &loads));
    }

    fn parse_sosh_ready(text: &str) -> Option<i64> {
        let line = text.lines().next()?.trim();
        let rest = line.strip_prefix("pid=")?;
        rest.parse().ok()
    }

    #[test]
    fn sosh_ready_token_rejects_old_and_banner() {
        assert_eq!(parse_sosh_ready("pid=7\n"), Some(7));
        assert_eq!(parse_sosh_ready("ok\n"), None);
        assert_eq!(parse_sosh_ready("sosh — escribe 'help'\n"), None);
        assert_eq!(parse_sosh_ready("pid=3\n"), Some(3));
        assert_ne!(parse_sosh_ready("pid=3\n"), Some(7));
    }
}
