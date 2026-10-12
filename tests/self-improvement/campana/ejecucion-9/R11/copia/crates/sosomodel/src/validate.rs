//! Validación de integridad de un modelo .som.

use crate::index::{TensorIndex, verify_shard};
use crate::manifest::Manifest;
use alloc::collections::BTreeMap;

pub struct ModelBundle<'a> {
    pub manifest: Manifest,
    pub index: TensorIndex,
    pub shards: BTreeMap<&'a str, &'a [u8]>,
}

impl<'a> ModelBundle<'a> {
    pub fn validate(&self) -> Result<(), ()> {
        for e in &self.index.entries {
            let shard = self.shards.get(e.shard.as_str()).ok_or(())?;
            let payload = verify_shard(shard)?;
            let end = e.offset.checked_add(e.byte_len).ok_or(())?;
            if end > payload.len() as u64 {
                return Err(());
            }
        }
        if self.manifest.prefetch.len() as u32 != self.manifest.num_layers {
            return Err(());
        }
        Ok(())
    }
}
