//! Buzón `SOSOUPD.TXT` en la ESP.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub const MAILBOX_MAGIC: &str = "SOSOUPD v1";

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum MailboxCmd {
    /// Vacío o sólo magic.
    #[default]
    Idle,
    /// `KERNEL <tam> <sha256hex> <version>`
    Kernel {
        size: u64,
        hash: String,
        version: String,
    },
    /// Tras aplicar en el shim, esperando arranque.
    Probando { version: String },
    /// Init confirmó el arranque.
    Ok { version: String },
    /// Petición manual de revertir.
    Revertir,
    /// Shim restauró el kernel anterior.
    Revertido { version: String },
}

#[derive(Clone, Debug, Default)]
pub struct Mailbox {
    pub cmd: MailboxCmd,
}

impl Mailbox {
    pub fn parse(text: &str) -> Self {
        let mut cmd = MailboxCmd::Idle;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line == MAILBOX_MAGIC {
                continue;
            }
            if let Some(rest) = line.strip_prefix("KERNEL ") {
                if let Some((size, rest)) = rest.split_once(' ') {
                    if let Some((hash, ver)) = rest.split_once(' ') {
                        cmd = MailboxCmd::Kernel {
                            size: size.parse().unwrap_or(0),
                            hash: hash.into(),
                            version: ver.into(),
                        };
                    }
                }
            } else if let Some(ver) = line.strip_prefix("PROBANDO ") {
                cmd = MailboxCmd::Probando {
                    version: ver.trim().into(),
                };
            } else if let Some(ver) = line.strip_prefix("OK ") {
                cmd = MailboxCmd::Ok {
                    version: ver.trim().into(),
                };
            } else if line == "REVERTIR" {
                cmd = MailboxCmd::Revertir;
            } else if let Some(ver) = line.strip_prefix("REVERTIDO ") {
                cmd = MailboxCmd::Revertido {
                    version: ver.trim().into(),
                };
            }
        }
        Self { cmd }
    }

    pub fn format_kernel(size: u64, hash: &str, version: &str) -> Vec<u8> {
        format_payload(&format!("KERNEL {size} {hash} {version}"))
    }

    pub fn format_probando(version: &str) -> Vec<u8> {
        format_payload(&format!("PROBANDO {version}"))
    }

    pub fn format_ok(version: &str) -> Vec<u8> {
        format_payload(&format!("OK {version}"))
    }

    pub fn format_revertir() -> Vec<u8> {
        format_payload("REVERTIR")
    }

    pub fn format_revertido(version: &str) -> Vec<u8> {
        format_payload(&format!("REVERTIDO {version}"))
    }

    pub fn format_idle() -> Vec<u8> {
        format_payload("")
    }
}

fn format_payload(body: &str) -> Vec<u8> {
    use crate::UPD_MAILBOX_SIZE;
    let mut out = Vec::with_capacity(UPD_MAILBOX_SIZE);
    out.extend_from_slice(MAILBOX_MAGIC.as_bytes());
    out.push(b'\n');
    if !body.is_empty() {
        out.extend_from_slice(body.as_bytes());
        out.push(b'\n');
    }
    while out.len() < UPD_MAILBOX_SIZE {
        out.push(b'\n');
    }
    out.truncate(UPD_MAILBOX_SIZE);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_kernel() {
        let hash = "ab".repeat(32);
        let payload = Mailbox::format_kernel(123, &hash, "0.2.0");
        let text = core::str::from_utf8(&payload).unwrap();
        let m = Mailbox::parse(text);
        assert!(matches!(
            m.cmd,
            MailboxCmd::Kernel { size: 123, .. }
        ));
    }
}
