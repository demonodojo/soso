//! Mapeo de nombres whisper.cpp → layout .som

pub struct Mapped {
    pub som_name: String,
    pub shape: Vec<u32>,
    pub mode: MapMode,
}

pub enum MapMode {
    /// Copia lineal (conv, bias 1D).
    Linear,
    /// Matriz 2D ggml ne=(cols,rows) → soso [rows, cols].
    Transpose2d,
    /// Posición / embedding de tokens ggml ne=(d, seq) → soso [seq, d].
    PosEmbed,
}

pub fn map_tensor(name: &str, ne: &[i32]) -> Option<Mapped> {
    let d = |x: i32| x as u32;
    if name == "encoder.conv1.bias" {
        return Some(Mapped {
            som_name: "conv1.bias".into(),
            shape: vec![d(ne[ne.len() - 1])],
            mode: MapMode::Linear,
        });
    }
    if name == "encoder.conv2.bias" {
        return Some(Mapped {
            som_name: "conv2.bias".into(),
            shape: vec![d(ne[ne.len() - 1])],
            mode: MapMode::Linear,
        });
    }
    if name == "encoder.conv1.weight" {
        return Some(Mapped {
            som_name: "conv1.weight".into(),
            shape: vec![d(ne[2]), d(ne[1]), d(ne[0])],
            mode: MapMode::Linear,
        });
    }
    if name == "encoder.conv2.weight" {
        return Some(Mapped {
            som_name: "conv2.weight".into(),
            shape: vec![d(ne[2]), d(ne[1]), d(ne[0])],
            mode: MapMode::Linear,
        });
    }
    if name == "encoder.positional_embedding" {
        return Some(Mapped {
            som_name: "pos_embed".into(),
            shape: vec![d(ne[1]), d(ne[0])],
            mode: MapMode::PosEmbed,
        });
    }
    if name == "encoder.ln_post.weight" {
        return Some(Mapped {
            som_name: "enc_ln_post.weight".into(),
            shape: vec![d(ne[0])],
            mode: MapMode::Linear,
        });
    }
    if name == "encoder.ln_post.bias" {
        return Some(Mapped {
            som_name: "enc_ln_post.bias".into(),
            shape: vec![d(ne[0])],
            mode: MapMode::Linear,
        });
    }
    if name == "decoder.token_embedding.weight" {
        return Some(Mapped {
            som_name: "token_embed".into(),
            shape: vec![d(ne[1]), d(ne[0])],
            mode: MapMode::PosEmbed,
        });
    }
    if name == "decoder.positional_embedding" {
        return Some(Mapped {
            som_name: "dec_pos_embed".into(),
            shape: vec![d(ne[1]), d(ne[0])],
            mode: MapMode::PosEmbed,
        });
    }
    if name == "decoder.ln.weight" {
        return Some(Mapped {
            som_name: "dec_ln.weight".into(),
            shape: vec![d(ne[0])],
            mode: MapMode::Linear,
        });
    }
    if name == "decoder.ln.bias" {
        return Some(Mapped {
            som_name: "dec_ln.bias".into(),
            shape: vec![d(ne[0])],
            mode: MapMode::Linear,
        });
    }

    if let Some(rest) = name.strip_prefix("encoder.blocks.") {
        return map_block(rest, "E", ne);
    }
    if let Some(rest) = name.strip_prefix("decoder.blocks.") {
        return map_block(rest, "D", ne);
    }
    None
}

fn map_block(rest: &str, prefix: &str, ne: &[i32]) -> Option<Mapped> {
    let (layer, tail) = rest.split_once('.')?;
    let layer: u32 = layer.parse().ok()?;
    let p = format!("{prefix}{layer:02}");
    let d = |x: i32| x as u32;
    let mapped = match tail {
        "attn_ln.weight" => (format!("{p}.attn_ln.weight"), vec![d(ne[0])], MapMode::Linear),
        "attn_ln.bias" => (format!("{p}.attn_ln.bias"), vec![d(ne[0])], MapMode::Linear),
        "attn.query.weight" => (format!("{p}.attn_q.weight"), vec![d(ne[1]), d(ne[0])], MapMode::Transpose2d),
        "attn.query.bias" => (format!("{p}.attn_q.bias"), vec![d(ne[0])], MapMode::Linear),
        "attn.key.weight" => (format!("{p}.attn_k.weight"), vec![d(ne[1]), d(ne[0])], MapMode::Transpose2d),
        "attn.value.weight" => (format!("{p}.attn_v.weight"), vec![d(ne[1]), d(ne[0])], MapMode::Transpose2d),
        "attn.value.bias" => (format!("{p}.attn_v.bias"), vec![d(ne[0])], MapMode::Linear),
        "attn.out.weight" => (format!("{p}.attn_out.weight"), vec![d(ne[1]), d(ne[0])], MapMode::Transpose2d),
        "attn.out.bias" => (format!("{p}.attn_out.bias"), vec![d(ne[0])], MapMode::Linear),
        "mlp_ln.weight" => (format!("{p}.mlp_ln.weight"), vec![d(ne[0])], MapMode::Linear),
        "mlp_ln.bias" => (format!("{p}.mlp_ln.bias"), vec![d(ne[0])], MapMode::Linear),
        "mlp.0.weight" => (format!("{p}.mlp_fc1.weight"), vec![d(ne[1]), d(ne[0])], MapMode::Transpose2d),
        "mlp.0.bias" => (format!("{p}.mlp_fc1.bias"), vec![d(ne[0])], MapMode::Linear),
        "mlp.2.weight" => (format!("{p}.mlp_fc2.weight"), vec![d(ne[1]), d(ne[0])], MapMode::Transpose2d),
        "mlp.2.bias" => (format!("{p}.mlp_fc2.bias"), vec![d(ne[0])], MapMode::Linear),
        "cross_attn_ln.weight" => (format!("{p}.cross_ln.weight"), vec![d(ne[0])], MapMode::Linear),
        "cross_attn_ln.bias" => (format!("{p}.cross_ln.bias"), vec![d(ne[0])], MapMode::Linear),
        "cross_attn.query.weight" => (format!("{p}.cross_q.weight"), vec![d(ne[1]), d(ne[0])], MapMode::Transpose2d),
        "cross_attn.query.bias" => (format!("{p}.cross_q.bias"), vec![d(ne[0])], MapMode::Linear),
        "cross_attn.key.weight" => (format!("{p}.cross_k.weight"), vec![d(ne[1]), d(ne[0])], MapMode::Transpose2d),
        "cross_attn.value.weight" => (format!("{p}.cross_v.weight"), vec![d(ne[1]), d(ne[0])], MapMode::Transpose2d),
        "cross_attn.out.weight" => (format!("{p}.cross_out.weight"), vec![d(ne[1]), d(ne[0])], MapMode::Transpose2d),
        "cross_attn.out.bias" => (format!("{p}.cross_out.bias"), vec![d(ne[0])], MapMode::Linear),
        _ => return None,
    };
    Some(Mapped {
        som_name: mapped.0,
        shape: mapped.1,
        mode: mapped.2,
    })
}
