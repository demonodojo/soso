//! manifest.som: arquitectura del transformer y grafo de prefetch.

use crate::layout::{MAGIC, SOM_HEADER_SIZE};
use crate::{align_up, crc32c, CACHE_ALIGN};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Debug)]
pub struct LayerPrefetch {
    pub layer: u32,
    pub shards: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Manifest {
    pub name: String,
    pub vocab_size: u32,
    pub hidden_dim: u32,
    pub num_layers: u32,
    pub num_heads: u32,
    pub ffn_dim: u32,
    pub max_seq: u32,
    pub prefetch: Vec<LayerPrefetch>,
}

impl Manifest {
    pub fn tiny(name: &str) -> Self {
        let mut prefetch = Vec::new();
        for layer in 0..4u32 {
            prefetch.push(LayerPrefetch {
                layer,
                shards: alloc::vec![
                    format!("L{layer:02}.attn_q.tensor"),
                    format!("L{layer:02}.attn_k.tensor"),
                    format!("L{layer:02}.attn_v.tensor"),
                    format!("L{layer:02}.ffn_up.tensor"),
                    format!("L{layer:02}.ffn_down.tensor"),
                ],
            });
        }
        Self {
            name: String::from(name),
            vocab_size: 256,
            hidden_dim: 128,
            num_layers: 4,
            num_heads: 4,
            ffn_dim: 256,
            max_seq: 128,
            prefetch,
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(self.name.as_bytes());
        body.push(0);
        body.extend_from_slice(&self.vocab_size.to_le_bytes());
        body.extend_from_slice(&self.hidden_dim.to_le_bytes());
        body.extend_from_slice(&self.num_layers.to_le_bytes());
        body.extend_from_slice(&self.num_heads.to_le_bytes());
        body.extend_from_slice(&self.ffn_dim.to_le_bytes());
        body.extend_from_slice(&self.max_seq.to_le_bytes());
        body.extend_from_slice(&(self.prefetch.len() as u32).to_le_bytes());
        for pf in &self.prefetch {
            body.extend_from_slice(&pf.layer.to_le_bytes());
            body.extend_from_slice(&(pf.shards.len() as u32).to_le_bytes());
            for s in &pf.shards {
                body.extend_from_slice(s.as_bytes());
                body.push(0);
            }
        }
        let crc = crc32c(&body);
        let mut out = Vec::with_capacity(SOM_HEADER_SIZE + body.len());
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&(body.len() as u64).to_le_bytes());
        out.extend_from_slice(&body);
        let pad = align_up(out.len(), CACHE_ALIGN) - out.len();
        out.resize(out.len() + pad, 0);
        out
    }

    pub fn parse(data: &[u8]) -> Result<Self, ()> {
        if data.len() < SOM_HEADER_SIZE || data[..8] != MAGIC {
            return Err(());
        }
        let payload_len = u64::from_le_bytes(data[16..24].try_into().unwrap()) as usize;
        let body = &data[SOM_HEADER_SIZE..SOM_HEADER_SIZE + payload_len];
        if crc32c(body) != u32::from_le_bytes(data[8..12].try_into().unwrap()) {
            return Err(());
        }
        let mut off = 0usize;
        let nul = body[off..].iter().position(|&b| b == 0).ok_or(())?;
        let name = core::str::from_utf8(&body[off..off + nul]).map_err(|_| ())?;
        off += nul + 1;
        let read_u32 = |b: &[u8]| u32::from_le_bytes(b.try_into().unwrap());
        let vocab_size = read_u32(&body[off..off + 4]);
        off += 4;
        let hidden_dim = read_u32(&body[off..off + 4]);
        off += 4;
        let num_layers = read_u32(&body[off..off + 4]);
        off += 4;
        let num_heads = read_u32(&body[off..off + 4]);
        off += 4;
        let ffn_dim = read_u32(&body[off..off + 4]);
        off += 4;
        let max_seq = read_u32(&body[off..off + 4]);
        off += 4;
        let n_pf = read_u32(&body[off..off + 4]) as usize;
        off += 4;
        let mut prefetch = Vec::with_capacity(n_pf);
        for _ in 0..n_pf {
            let layer = read_u32(&body[off..off + 4]);
            off += 4;
            let n_shards = read_u32(&body[off..off + 4]) as usize;
            off += 4;
            let mut shards = Vec::with_capacity(n_shards);
            for _ in 0..n_shards {
                let nul = body[off..].iter().position(|&b| b == 0).ok_or(())?;
                let s = core::str::from_utf8(&body[off..off + nul]).map_err(|_| ())?;
                shards.push(String::from(s));
                off += nul + 1;
            }
            prefetch.push(LayerPrefetch { layer, shards });
        }
        Ok(Self {
            name: String::from(name),
            vocab_size,
            hidden_dim,
            num_layers,
            num_heads,
            ffn_dim,
            max_seq,
            prefetch,
        })
    }
}
