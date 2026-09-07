use soso_update_core::manifest::{Manifest, MANIFEST_MAGIC};
use soso_update_core::mailbox::{Mailbox, MailboxCmd, MAILBOX_MAGIC};
use soso_update_core::pack::{PackReader, PackWriter};
use soso_update_core::semver::{cmp, parse};

#[test]
fn manifest_roundtrip() {
    let text = format!(
        "{MANIFEST_MAGIC}\nversion=1.0.0\nbuild=abc\nfecha=2026-01-01\n\
         kernel aa{} 100\npack bb{} 200\nf cc{} 0 10 bin/init\n",
        "0".repeat(62),
        "1".repeat(62),
        "2".repeat(62),
    );
    let m = Manifest::parse(&text).expect("parse");
    assert_eq!(m.version_raw, "1.0.0");
    assert_eq!(m.files.len(), 1);
    let again = m.format();
    let m2 = Manifest::parse(&again).expect("parse2");
    assert_eq!(m2.version_raw, m.version_raw);
    assert_eq!(m2.files[0].path, "bin/init");
}

#[test]
fn pack_writer_order() {
    let mut w = PackWriter::new();
    w.push("b.txt".into(), b"bb".to_vec());
    w.push("a.txt".into(), b"aa".to_vec());
    w.sort_entries();
    let (blob, files) = w.build();
    assert_eq!(blob, b"aabb");
    assert_eq!(files[0].path, "a.txt");
    assert_eq!(files[0].offset, 0);
    assert_eq!(files[1].offset, 2);
    let reader = PackReader::new(&blob);
    assert!(reader.verify_entry(&files[0]));
}

#[test]
fn semver() {
    assert_eq!(
        cmp(&parse("0.2.0").unwrap(), &parse("0.1.9").unwrap()),
        core::cmp::Ordering::Greater
    );
}

#[test]
fn mailbox_kernel() {
    let hash = "ab".repeat(32);
    let p = soso_update_core::Mailbox::format_kernel(42, &hash, "0.2.0");
    let m = Mailbox::parse(core::str::from_utf8(&p).unwrap());
    assert!(matches!(m.cmd, MailboxCmd::Kernel { size: 42, .. }));
    assert!(core::str::from_utf8(&p).unwrap().starts_with(MAILBOX_MAGIC));
}
