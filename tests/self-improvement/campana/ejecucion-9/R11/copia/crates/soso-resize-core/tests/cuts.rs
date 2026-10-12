//! Tests de cortes: slide reanudable, journal y patrones de partición.

use soso_resize_core::*;
use gptdisk::{Guid, Header, entry_mut, set_entry_first_lba, set_entry_last_lba};
use sha2::{Digest, Sha256};
use std::cell::Cell;

struct MemDisk {
    data: Vec<u8>,
    fail_after: Cell<Option<u64>>,
    fail_flush: Cell<bool>,
    writes: Cell<u64>,
    reads: Cell<u64>,
}

impl MemDisk {
    fn new(sectors: u64) -> Self {
        Self {
            data: vec![0; sectors as usize * SECTOR],
            fail_after: Cell::new(None),
            fail_flush: Cell::new(false),
            writes: Cell::new(0),
            reads: Cell::new(0),
        }
    }

    fn sector_offset(lba: u64) -> usize {
        lba as usize * SECTOR
    }

    fn paint_range(&mut self, first: u64, count: u64, tag: u8) {
        for i in 0..count {
            let off = Self::sector_offset(first + i);
            self.data[off..off + SECTOR].fill(tag);
        }
    }

    fn hash_range(&self, first: u64, count: u64) -> [u8; 32] {
        let off = Self::sector_offset(first);
        let end = off + count as usize * SECTOR;
        Sha256::digest(&self.data[off..end]).into()
    }
}

impl Disk512 for MemDisk {
    fn read_sector(&self, lba: u64, buf: &mut [u8; SECTOR]) -> Result<(), Error> {
        self.reads.set(self.reads.get() + 1);
        let off = Self::sector_offset(lba);
        if off + SECTOR > self.data.len() {
            return Err(Error::OutOfRange);
        }
        buf.copy_from_slice(&self.data[off..off + SECTOR]);
        Ok(())
    }

    fn write_sector(&mut self, lba: u64, buf: &[u8; SECTOR]) -> Result<(), Error> {
        let n = self.writes.get() + 1;
        self.writes.set(n);
        if self.fail_after.get().is_some_and(|f| n > f) {
            return Err(Error::Io);
        }
        let off = Self::sector_offset(lba);
        if off + SECTOR > self.data.len() {
            return Err(Error::OutOfRange);
        }
        self.data[off..off + SECTOR].copy_from_slice(buf);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Error> {
        if self.fail_flush.get() {
            return Err(Error::Io);
        }
        Ok(())
    }
}

fn write_test_gpt(disk: &mut MemDisk, p2: PartGeom, p3: PartGeom, p4: PartGeom) {
    let disk_sectors = (disk.data.len() / SECTOR) as u64;
    let mut hdr = Header {
        revision: 0x0001_0000,
        header_size: 92,
        my_lba: 1,
        alternate_lba: disk_sectors - 1,
        first_usable: 34,
        last_usable: disk_sectors - 34,
        disk_guid: Guid::ZERO,
        entries_lba: 2,
        num_entries: 128,
        entry_size: 128,
    };
    let mut entries = vec![0u8; hdr.entries_bytes()];
    for (idx, geom) in [(GPT_ROOT, p2), (GPT_MODELS, p3), (GPT_INSTALL, p4)] {
        if let Some(e) = entry_mut(&mut entries, &hdr, idx) {
            e[0] = 0x01;
            set_entry_first_lba(e, geom.first);
            set_entry_last_lba(e, geom.last);
        }
    }
    let plan = gptdisk::relayout(&mut hdr, &mut entries, disk_sectors).unwrap();
    // Dejar hueco tras install (relayout estira la última partición hasta last_usable).
    if let Some(e) = entry_mut(&mut entries, &hdr, GPT_INSTALL) {
        set_entry_last_lba(e, p4.last);
    }
    let sec = hdr.render(&entries);
    disk.write_sector(1, &sec).unwrap();
    for (i, chunk) in entries.chunks(SECTOR).enumerate() {
        let mut sec = [0u8; SECTOR];
        sec[..chunk.len()].copy_from_slice(chunk);
        disk.write_sector(2 + i as u64, &sec).unwrap();
    }
    let primary = gptdisk::render_primary(&hdr, &entries, &plan);
    disk.write_sector(plan.primary_header_lba, &primary).unwrap();
    for (i, chunk) in entries.chunks(SECTOR).enumerate() {
        let mut sec = [0u8; SECTOR];
        sec[..chunk.len()].copy_from_slice(chunk);
        disk.write_sector(plan.backup_entries_lba + i as u64, &sec).unwrap();
    }
    let backup = gptdisk::render_backup(&hdr, &entries, &plan);
    disk.write_sector(plan.backup_header_lba, &backup).unwrap();
}

fn read_part_backup(disk: &MemDisk, entry_index: usize) -> PartGeom {
    use gptdisk::{Header, entry, entry_first_lba, entry_last_lba};
    let disk_sectors = (disk.data.len() / SECTOR) as u64;
    let mut hdr_sec = [0u8; SECTOR];
    disk.read_sector(disk_sectors - 1, &mut hdr_sec).unwrap();
    let hdr = Header::parse(&hdr_sec).unwrap();
    let bytes = hdr.entries_bytes();
    let padded = bytes.div_ceil(SECTOR) * SECTOR;
    let mut entries = vec![0u8; padded];
    for i in 0..hdr.entries_sectors() {
        let mut sec = [0u8; SECTOR];
        disk.read_sector(hdr.entries_lba + i, &mut sec).unwrap();
        entries[i as usize * SECTOR..(i as usize + 1) * SECTOR].copy_from_slice(&sec);
    }
    entries.truncate(bytes);
    let ent = entry(&entries, &hdr, entry_index).unwrap();
    PartGeom::from_lba(entry_first_lba(ent), entry_last_lba(ent))
}

#[test]
fn journal_roundtrip() {
    let j = Journal {
        phase: JOURNAL_SLIDE_P3,
        delta_sectors: 128,
        slide_done: 64,
    };
    let mut sec = [0u8; SECTOR];
    j.encode(&mut sec);
    let got = Journal::decode(&sec).unwrap();
    assert_eq!(got, j);
}

#[test]
fn journal_blank_is_idle() {
    for fill in [0u8, b'\n', b' '] {
        let sec = [fill; SECTOR];
        assert_eq!(Journal::decode(&sec).unwrap(), Journal::idle());
    }
}

#[test]
fn journal_corrupt_rejected() {
    let mut sec = [0u8; SECTOR];
    sec[0] = 0xA5;
    sec[20] = 0x5A;
    assert!(Journal::decode(&sec).is_err());
}

#[test]
fn slide_resumes_after_cut() {
    let delta = 16u64;
    let p3 = PartGeom::from_lba(1000, 1099);
    let p4 = PartGeom::from_lba(1100, 1149);
    let p2 = PartGeom::from_lba(34, 999);
    let mut disk = MemDisk::new(5000);
    disk.paint_range(p3.first, p3.sectors, 0xA3);
    disk.paint_range(p4.first, p4.sectors, 0xB4);
    write_test_gpt(&mut disk, p2, p3, p4);

    let hash_p3_before = disk.hash_range(p3.first, p3.sectors - delta);
    let hash_p4_before = disk.hash_range(p4.first, p4.sectors);

    let p3_after = p3.sectors - delta;
    let mut done = 0u64;
    disk.writes.set(0);
    disk.fail_after.set(Some(10));
    assert!(slide_sectors(&mut disk, p3.first, p3.first + delta, p3_after, &mut done).is_err());
    assert!(done > 0 && done < p3_after);

    disk.fail_after.set(None);
    slide_sectors(&mut disk, p3.first, p3.first + delta, p3_after, &mut done).unwrap();
    done = 0;
    slide_sectors(&mut disk, p4.first, p4.first + delta, p4.sectors, &mut done).unwrap();

    assert_eq!(
        disk.hash_range(p3.first + delta, p3_after),
        hash_p3_before
    );
    assert_eq!(
        disk.hash_range(p4.first + delta, p4.sectors),
        hash_p4_before
    );
}

#[test]
fn slide_flush_error_does_not_commit_progress() {
    let delta = 16u64;
    let p3 = PartGeom::from_lba(1000, 1099);
    let p3_after = p3.sectors - delta;
    let mut disk = MemDisk::new(5000);
    disk.paint_range(p3.first, p3.sectors, 0xA3);
    let mut done = 0u64;
    disk.fail_flush.set(true);
    assert_eq!(
        slide_sectors(&mut disk, p3.first, p3.first + delta, p3_after, &mut done),
        Err(Error::Io)
    );
    assert_eq!(done, 0);
    disk.fail_flush.set(false);
    slide_sectors(&mut disk, p3.first, p3.first + delta, p3_after, &mut done).unwrap();
    assert_eq!(done, p3_after);
}

#[test]
fn apply_gpt_flush_error_keeps_journal_phase() {
    let (mut disk, _p2, p3, p4, delta) = setup_painted_disk();
    let mut journal = Journal {
        phase: JOURNAL_GPT,
        delta_sectors: delta,
        slide_done: 0,
    };
    let p3_after = p3.sectors - delta;
    slide_sectors(
        &mut disk,
        p3.first,
        p3.first + delta,
        p3_after,
        &mut journal.slide_done,
    )
    .unwrap();
    journal.slide_done = 0;
    slide_sectors(
        &mut disk,
        p4.first,
        p4.first + delta,
        p4.sectors,
        &mut journal.slide_done,
    )
    .unwrap();
    journal.slide_done = 0;
    disk.fail_flush.set(true);
    assert_eq!(
        apply_gpt_and_slides(&mut disk, delta, &mut journal, |_| {}),
        Err(Error::Io)
    );
    assert_eq!(journal.phase, JOURNAL_GPT);
    disk.fail_flush.set(false);
    apply_gpt_and_slides(&mut disk, delta, &mut journal, |_| {}).unwrap();
    assert_eq!(journal.phase, JOURNAL_IDLE);
}

#[test]
fn validate_geometry_rejects_huge_delta() {
    let p3 = PartGeom::from_lba(1000, 1009);
    let p4 = PartGeom::from_lba(1010, 1019);
    let p2 = PartGeom::from_lba(34, 999);
    let mut disk = MemDisk::new(5000);
    write_test_gpt(&mut disk, p2, p3, p4);
    assert!(validate_geometry(&disk, 9999).is_err());
}

#[test]
fn idle_journal_no_recovery() {
    let j = Journal::idle();
    assert!(!j.needs_recovery());
}

#[test]
fn slide_partial_done_resumes() {
    let mut disk = MemDisk::new(200);
    disk.paint_range(10, 20, 0xCC);
    let mut done = 0u64;
    slide_sectors(&mut disk, 10, 30, 20, &mut done).unwrap();
    assert_eq!(done, 20);
    assert_eq!(disk.hash_range(30, 20), disk.hash_range(10, 20));
}

fn test_layout() -> (PartGeom, PartGeom, PartGeom, u64) {
    let delta = 16u64;
    let p3 = PartGeom::from_lba(1000, 1099);
    let p4 = PartGeom::from_lba(1100, 1149);
    let p2 = PartGeom::from_lba(34, 999);
    (p2, p3, p4, delta)
}

fn setup_painted_disk() -> (MemDisk, PartGeom, PartGeom, PartGeom, u64) {
    let (p2, p3, p4, delta) = test_layout();
    let mut disk = MemDisk::new(5000);
    disk.paint_range(p2.first, p2.sectors, 0xA2);
    disk.paint_range(p3.first, p3.sectors, 0xA3);
    disk.paint_range(p4.first, p4.sectors, 0xB4);
    write_test_gpt(&mut disk, p2, p3, p4);
    (disk, p2, p3, p4, delta)
}

#[test]
fn apply_full_pipeline_preserves_partition_hashes() {
    let (mut disk, p2, p3, p4, delta) = setup_painted_disk();
    validate_geometry(&disk, delta).unwrap();

    let hash_p2 = disk.hash_range(p2.first, p2.sectors);
    let hash_p3_tail = disk.hash_range(p3.first, p3.sectors - delta);
    let hash_p4 = disk.hash_range(p4.first, p4.sectors);

    let mut journal = Journal {
        phase: JOURNAL_SHRINK_MODELS,
        delta_sectors: delta,
        slide_done: 0,
    };
    apply_gpt_and_slides(&mut disk, delta, &mut journal, |_| {}).unwrap();
    assert_eq!(journal.phase, JOURNAL_IDLE);

    let p2n = read_part(&disk, GPT_ROOT).unwrap();
    let p3n = read_part(&disk, GPT_MODELS).unwrap();
    let p4n = read_part(&disk, GPT_INSTALL).unwrap();

    assert_eq!(p2n.sectors, p2.sectors + delta);
    assert_eq!(p3n.sectors, p3.sectors - delta);
    assert_eq!(p4n.first, p4.first + delta);
    assert_eq!(p4n.sectors, p4.sectors);

    assert_eq!(disk.hash_range(p2.first, p2.sectors), hash_p2);
    assert_eq!(
        disk.hash_range(p3n.first, p3n.sectors),
        hash_p3_tail
    );
    assert_eq!(
        disk.hash_range(p4n.first, p4n.sectors),
        hash_p4
    );
}

#[test]
fn apply_resumes_after_cut_mid_p3() {
    let (mut disk, p2, p3, p4, delta) = setup_painted_disk();
    let hash_p3_tail = disk.hash_range(p3.first, p3.sectors - delta);
    let hash_p4 = disk.hash_range(p4.first, p4.sectors);

    let mut journal = Journal {
        phase: JOURNAL_SHRINK_MODELS,
        delta_sectors: delta,
        slide_done: 0,
    };

    disk.writes.set(0);
    disk.fail_after.set(Some(8));
    assert!(apply_gpt_and_slides(&mut disk, delta, &mut journal, |_| {}).is_err());
    assert_eq!(journal.phase, JOURNAL_SLIDE_P3);
    assert!(journal.slide_done > 0);

    disk.fail_after.set(None);
    apply_gpt_and_slides(&mut disk, delta, &mut journal, |_| {}).unwrap();
    assert_eq!(journal.phase, JOURNAL_IDLE);

    let p3n = read_part(&disk, GPT_MODELS).unwrap();
    let p4n = read_part(&disk, GPT_INSTALL).unwrap();
    assert_eq!(
        disk.hash_range(p3n.first, p3n.sectors),
        hash_p3_tail
    );
    assert_eq!(
        disk.hash_range(p4n.first, p4n.sectors),
        hash_p4
    );
    let _ = p2;
}

#[test]
fn apply_resumes_after_cut_mid_p4() {
    let (mut disk, _p2, p3, p4, delta) = setup_painted_disk();
    let hash_p4 = disk.hash_range(p4.first, p4.sectors);
    let p3_after = p3.sectors - delta;

    let mut journal = Journal {
        phase: JOURNAL_SLIDE_P3,
        delta_sectors: delta,
        slide_done: 0,
    };
    slide_sectors(
        &mut disk,
        p3.first,
        p3.first + delta,
        p3_after,
        &mut journal.slide_done,
    )
    .unwrap();
    journal.phase = JOURNAL_SLIDE_P4;
    journal.slide_done = 0;

    disk.writes.set(0);
    disk.fail_after.set(Some(5));
    assert!(apply_gpt_and_slides(&mut disk, delta, &mut journal, |_| {}).is_err());
    assert_eq!(journal.phase, JOURNAL_SLIDE_P4);
    assert!(journal.slide_done > 0);

    disk.fail_after.set(None);
    apply_gpt_and_slides(&mut disk, delta, &mut journal, |_| {}).unwrap();

    let p4n = read_part(&disk, GPT_INSTALL).unwrap();
    assert_eq!(
        disk.hash_range(p4n.first, p4n.sectors),
        hash_p4
    );
}

#[test]
fn journal_invalid_phase_not_valid() {
    let j = Journal {
        phase: 99,
        delta_sectors: 16,
        slide_done: 0,
    };
    assert!(!j.valid_phase());
    assert!(!j.needs_recovery());
}

#[test]
fn validate_geometry_rejects_delta_ge_p3() {
    let (disk, p2, p3, p4, _) = setup_painted_disk();
    assert!(validate_geometry(&disk, p3.sectors).is_err());
    let _ = (p2, p4);
}

#[test]
fn recover_slides_rejects_intent_without_shrink() {
    let (mut disk, _p2, _p3, _p4, delta) = setup_painted_disk();
    let mut journal = Journal {
        phase: JOURNAL_INTENT,
        delta_sectors: delta,
        slide_done: 0,
    };
    assert!(recover_slides_and_gpt(&mut disk, &mut journal, |_| {}).is_err());
}

#[test]
fn recovery_reloads_journal_from_esp() {
    let (mut disk, _p2, p3, p4, delta) = setup_painted_disk();
    let hash_p4 = disk.hash_range(p4.first, p4.sectors);
    let esp = Cell::new([0u8; SECTOR]);

    let mut journal = Journal {
        phase: JOURNAL_SHRINK_MODELS,
        delta_sectors: delta,
        slide_done: 0,
    };
    let persist = |j: &Journal| {
        let mut sec = esp.get();
        j.encode(&mut sec);
        esp.set(sec);
    };

    disk.writes.set(0);
    disk.fail_after.set(Some(12));
    assert!(apply_gpt_and_slides(&mut disk, delta, &mut journal, persist).is_err());
    persist(&journal);

    let mut journal = Journal::decode(&esp.get()).unwrap();
    disk.fail_after.set(None);
    recover_slides_and_gpt(&mut disk, &mut journal, persist).unwrap();

    let p4n = read_part(&disk, GPT_INSTALL).unwrap();
    assert_eq!(disk.hash_range(p4n.first, p4n.sectors), hash_p4);
    let _ = p3;
}

#[test]
fn gpt_backup_matches_primary_after_apply() {
    let (mut disk, _p2, _p3, _p4, delta) = setup_painted_disk();
    let mut journal = Journal {
        phase: JOURNAL_SHRINK_MODELS,
        delta_sectors: delta,
        slide_done: 0,
    };
    apply_gpt_and_slides(&mut disk, delta, &mut journal, |_| {}).unwrap();

    for idx in [GPT_ROOT, GPT_MODELS, GPT_INSTALL] {
        let pri = read_part(&disk, idx).unwrap();
        let bak = read_part_backup(&disk, idx);
        assert_eq!(pri, bak, "partición {idx}");
    }
}

#[test]
fn apply_resumes_after_cut_mid_gpt() {
    let (mut disk, _p2, p3, p4, delta) = setup_painted_disk();
    let hash_p4 = disk.hash_range(p4.first, p4.sectors);

    let mut journal = Journal {
        phase: JOURNAL_GPT,
        delta_sectors: delta,
        slide_done: 0,
    };
    let p3_after = p3.sectors - delta;
    slide_sectors(
        &mut disk,
        p3.first,
        p3.first + delta,
        p3_after,
        &mut journal.slide_done,
    )
    .unwrap();
    journal.slide_done = 0;
    slide_sectors(
        &mut disk,
        p4.first,
        p4.first + delta,
        p4.sectors,
        &mut journal.slide_done,
    )
    .unwrap();

    disk.writes.set(0);
    disk.fail_after.set(Some(2));
    assert!(apply_gpt_and_slides(&mut disk, delta, &mut journal, |_| {}).is_err());

    disk.fail_after.set(None);
    apply_gpt_and_slides(&mut disk, delta, &mut journal, |_| {}).unwrap();

    let p4n = read_part(&disk, GPT_INSTALL).unwrap();
    assert_eq!(disk.hash_range(p4n.first, p4n.sectors), hash_p4);
    assert_eq!(read_part_backup(&disk, GPT_INSTALL), p4n);
}

#[test]
fn phase_rank_orders_intent_before_shrink() {
    assert!(phase_rank(JOURNAL_INTENT).unwrap() < phase_rank(JOURNAL_SHRINK_MODELS).unwrap());
    assert!(phase_rank(JOURNAL_SHRINK_MODELS).unwrap() < phase_rank(JOURNAL_SLIDE_P3).unwrap());
}

#[test]
fn geom_cache_skips_gpt_on_ordinary_lookup() {
    let (disk, p2, p3, _p4, _) = setup_painted_disk();
    let mut cache = GeomCache::default();
    cache.reload(&disk).unwrap();
    let after_load = disk.reads.get();
    assert_eq!(cache.reloads(), 1);
    assert_eq!(cache.get(GPT_ROOT).unwrap(), p2);
    assert_eq!(cache.get(GPT_MODELS).unwrap(), p3);
    for _ in 0..32 {
        let _ = cache.get(GPT_ROOT);
        let _ = cache.get(GPT_MODELS);
        let _ = cache.get(GPT_INSTALL);
    }
    assert_eq!(disk.reads.get(), after_load, "lookup no debe releer GPT");
}

#[test]
fn geom_cache_sees_new_limits_after_reload() {
    let (mut disk, p2, p3, p4, _) = setup_painted_disk();
    let mut cache = GeomCache::default();
    cache.reload(&disk).unwrap();
    let delta = 8;
    let mut journal = Journal {
        phase: JOURNAL_SHRINK_MODELS,
        delta_sectors: delta,
        slide_done: 0,
    };
    apply_gpt_and_slides(&mut disk, delta, &mut journal, |_| {}).unwrap();
    assert_eq!(cache.get(GPT_ROOT).unwrap().last, p2.last, "sin reload sigue la geometría vieja");
    cache.reload(&disk).unwrap();
    let root = cache.get(GPT_ROOT).unwrap();
    let models = cache.get(GPT_MODELS).unwrap();
    assert_eq!(root.last, p2.last + delta);
    assert_eq!(models.first, p3.first + delta);
    assert_eq!(cache.get(GPT_INSTALL).unwrap().first, p4.first + delta);
    assert_eq!(cache.reloads(), 2);
}

fn seq_read(disk: &impl Disk512, first: u64, sectors: u64) {
    let mut buf = [0u8; SECTOR];
    for i in 0..sectors {
        disk.read_sector(first + i, &mut buf).unwrap();
    }
}

fn mib_s(bytes: u64, dt: std::time::Duration) -> f64 {
    let secs = dt.as_secs_f64().max(1e-9);
    (bytes as f64 / (1024.0 * 1024.0)) / secs
}

/// B6: releer GPT en cada sector vs geometría en RAM (disco en memoria, 4 MiB).
#[test]
fn geom_cache_seq_read_mibs() {
    let models_sectors = 8192u64;
    let p3 = PartGeom::from_lba(2000, 2000 + models_sectors - 1);
    let p4 = PartGeom::from_lba(p3.last + 1, p3.last + 32);
    let p2 = PartGeom::from_lba(34, 1999);
    let mut disk = MemDisk::new(p4.last + 40);
    disk.paint_range(p3.first, p3.sectors, 0x5A);
    write_test_gpt(&mut disk, p2, p3, p4);

    disk.reads.set(0);
    let t0 = std::time::Instant::now();
    for i in 0..models_sectors {
        let g = read_part(&disk, GPT_MODELS).unwrap();
        let mut buf = [0u8; SECTOR];
        disk.read_sector(g.first + i, &mut buf).unwrap();
    }
    let naive_dt = t0.elapsed();
    let naive_reads = disk.reads.get();

    disk.reads.set(0);
    let mut cache = GeomCache::default();
    cache.reload(&disk).unwrap();
    let g = cache.get(GPT_MODELS).unwrap();
    let t1 = std::time::Instant::now();
    seq_read(&disk, g.first, models_sectors);
    let cached_dt = t1.elapsed();
    let cached_reads = disk.reads.get();

    let bytes = models_sectors * SECTOR as u64;
    eprintln!(
        "B6 host 4MiB: naive {} lecturas {:.1} MiB/s; cache {} lecturas {:.1} MiB/s (reload={})",
        naive_reads,
        mib_s(bytes, naive_dt),
        cached_reads,
        mib_s(bytes, cached_dt),
        cache.reloads()
    );
    assert!(
        naive_reads > cached_reads * 4,
        "releer GPT por sector debe multiplicar las lecturas"
    );
    assert_eq!(cache.reloads(), 1);
}

struct FileDisk {
    file: std::sync::Mutex<std::fs::File>,
    reads: Cell<u64>,
}

impl Disk512 for FileDisk {
    fn read_sector(&self, lba: u64, buf: &mut [u8; SECTOR]) -> Result<(), Error> {
        use std::io::{Read, Seek, SeekFrom};
        self.reads.set(self.reads.get() + 1);
        let mut f = self.file.lock().unwrap();
        f.seek(SeekFrom::Start(lba * SECTOR as u64))
            .map_err(|_| Error::Io)?;
        f.read_exact(buf).map_err(|_| Error::Io)?;
        Ok(())
    }

    fn write_sector(&mut self, _lba: u64, _buf: &[u8; SECTOR]) -> Result<(), Error> {
        Err(Error::Io)
    }
}

/// B6: misma imagen live; 8 MiB de p3 con y sin caché GPT.
#[test]
fn geom_cache_live_image_mibs() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/usb-live/soso-live.img");
    if !path.is_file() {
        eprintln!("B6 live: sin {} — omitido", path.display());
        return;
    }
    let file = std::fs::File::open(&path).unwrap();
    let disk = FileDisk {
        file: std::sync::Mutex::new(file),
        reads: Cell::new(0),
    };
    let mut cache = GeomCache::default();
    cache.reload(&disk).unwrap();
    let models = cache.get(GPT_MODELS).expect("p3 modelos");
    let take = models.sectors.min(16384); // 8 MiB
    let bytes = take * SECTOR as u64;

    disk.reads.set(0);
    let t0 = std::time::Instant::now();
    for i in 0..take {
        let g = read_part(&disk, GPT_MODELS).unwrap();
        let mut buf = [0u8; SECTOR];
        disk.read_sector(g.first + i, &mut buf).unwrap();
    }
    let naive_dt = t0.elapsed();
    let naive_reads = disk.reads.get();

    disk.reads.set(0);
    let t1 = std::time::Instant::now();
    seq_read(&disk, models.first, take);
    let cached_dt = t1.elapsed();
    let cached_reads = disk.reads.get();

    eprintln!(
        "B6 live {}: naive {} lecturas {:.1} MiB/s; cache {} lecturas {:.1} MiB/s",
        path.display(),
        naive_reads,
        mib_s(bytes, naive_dt),
        cached_reads,
        mib_s(bytes, cached_dt)
    );
    assert!(naive_reads > cached_reads * 4);
    assert_eq!(cached_reads, take);
}
