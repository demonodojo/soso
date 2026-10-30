//! Demo H1/F2: limpia un render target en VRAM, opcional quad texturizado, presenta en GOP o NVDisplay.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use libsoso::{println, sys};
use soso_abi::{GpuInfo, GPU_SCANOUT_EFFECTIVE_GOP, GPU_SCANOUT_EFFECTIVE_NVDISP};

libsoso::entry!(main);

const NVDISP_W: u32 = 640;
const NVDISP_H: u32 = 480;
const NVDISP_PITCH: u32 = NVDISP_W * 4;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScanoutMode {
    Auto,
    Gop,
    Nvdisp,
}

fn parse_scanout(args: &[String]) -> ScanoutMode {
    let mut i = 0;
    while i + 1 < args.len() {
        if args[i] == "--scanout" {
            return match args[i + 1].as_str() {
                "gop" => ScanoutMode::Gop,
                "nvdisp" | "nv" => ScanoutMode::Nvdisp,
                _ => ScanoutMode::Auto,
            };
        }
        i += 1;
    }
    ScanoutMode::Auto
}

fn parse_frames(args: &[String]) -> u32 {
    let mut n = 120u32;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--frames" && i + 1 < args.len() {
            if let Ok(v) = args[i + 1].parse::<u32>() {
                n = v.max(1);
            }
            i += 2;
            continue;
        }
        i += 1;
    }
    n
}

fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a.as_str() == name)
}

#[derive(Clone, Copy)]
enum PresentTag {
    Gpres,
    Gnvd,
}

fn pick_present(info: &GpuInfo, mode: ScanoutMode) -> Option<PresentTag> {
    let nvdisp = info._pad[1] != 0;
    let gop = info._pad[2] != 0;
    match mode {
        ScanoutMode::Gop if gop => Some(PresentTag::Gpres),
        ScanoutMode::Gop => None,
        ScanoutMode::Nvdisp if nvdisp => Some(PresentTag::Gnvd),
        ScanoutMode::Nvdisp => None,
        ScanoutMode::Auto => match info._pad[3] {
            GPU_SCANOUT_EFFECTIVE_NVDISP => Some(PresentTag::Gnvd),
            GPU_SCANOUT_EFFECTIVE_GOP => Some(PresentTag::Gpres),
            _ => None,
        },
    }
}

fn encode_present(tag: PresentTag, rt_h: u64, w: u32, h: u32, pitch: u32) -> [u8; 25] {
    let mut pres = [0u8; 25];
    match tag {
        PresentTag::Gpres => pres[..5].copy_from_slice(b"GPRES"),
        PresentTag::Gnvd => {
            pres[..4].copy_from_slice(b"GNVD");
            pres[4] = 0;
        }
    }
    pres[5..13].copy_from_slice(&rt_h.to_le_bytes());
    pres[13..17].copy_from_slice(&w.to_le_bytes());
    pres[17..21].copy_from_slice(&h.to_le_bytes());
    pres[21..25].copy_from_slice(&pitch.to_le_bytes());
    pres
}

fn main(args: &[String]) -> u8 {
    let frames = parse_frames(args);
    let with_quad = has_flag(args, "--quad");
    let scanout = parse_scanout(args);

    let mut info = GpuInfo::default();
    if sys::gpu_info(&mut info) < 0 || info.present == 0 {
        println!("soso-gfx-demo: sin GPU");
        return 1;
    }
    if info._pad[0] == 0 {
        println!("soso-gfx-demo: motor 3D no disponible (gfx3d=0)");
        return 1;
    }

    let present_tag = match pick_present(&info, scanout) {
        Some(p) => p,
        None => {
            println!("soso-gfx-demo: scanout no disponible para --scanout");
            return 1;
        }
    };

    let mut fb = soso_abi::FbInfo::default();
    let fb_ok = sys::fb_info(&mut fb) >= 0 && fb.present != 0;

    let (w, h, pitch) = if matches!(present_tag, PresentTag::Gnvd) && !fb_ok {
        (NVDISP_W, NVDISP_H, NVDISP_PITCH)
    } else if fb_ok {
        if sys::fb_set_mode(soso_abi::FB_MODE_GRAPHICS).is_err() {
            println!("soso-gfx-demo: fb_set_mode falló");
            return 1;
        }
        let w = fb.width as u32;
        let h = fb.height as u32;
        let pitch = w.saturating_mul(4);
        if matches!(present_tag, PresentTag::Gnvd) {
            (
                w.min(NVDISP_W),
                h.min(NVDISP_H),
                w.min(NVDISP_W).saturating_mul(4),
            )
        } else {
            (w, h, pitch)
        }
    } else {
        println!("soso-gfx-demo: sin framebuffer GOP");
        return 1;
    };

    let fb_bytes = (pitch as u64).saturating_mul(h as u64);
    if fb_bytes == 0 || fb_bytes > 32 * 1024 * 1024 {
        println!("soso-gfx-demo: tamaño de FB inválido");
        return 1;
    }

    let rt_h = sys::gpu_alloc_vram(fb_bytes);
    if rt_h < 0 {
        println!("soso-gfx-demo: gpu_alloc_vram falló");
        return 1;
    }
    let rt_h = rt_h as u64;

    let quad = if with_quad {
        let tw = 128u32;
        let th = 128u32;
        let tpitch = tw * 4;
        let tbytes = (tpitch as u64) * (th as u64);
        let tex_h = sys::gpu_alloc_vram(tbytes);
        if tex_h < 0 {
            println!("soso-gfx-demo: textura VRAM falló");
            let _ = sys::gpu_free(rt_h);
            return 1;
        }
        let tex_h = tex_h as u64;
        let mut scratch = alloc::vec::Vec::with_capacity(tbytes as usize);
        scratch.resize(tbytes as usize, 0);
        for y in 0..th as usize {
            for x in 0..tw as usize {
                let o = y * tpitch as usize + x * 4;
                scratch[o] = 0x40;
                scratch[o + 1] = 0x80;
                scratch[o + 2] = 0xe0;
                scratch[o + 3] = 0xff;
            }
        }
        if sys::gpu_map(tex_h, scratch.as_ptr() as u64, tbytes) < 0 {
            println!("soso-gfx-demo: gpu_map textura falló");
            let _ = sys::gpu_free(tex_h);
            let _ = sys::gpu_free(rt_h);
            return 1;
        }
        Some((tex_h, tw, th, tpitch))
    } else {
        None
    };

    let pres_name = match present_tag {
        PresentTag::Gpres => "GPRES",
        PresentTag::Gnvd => "GNVD",
    };

    let t0 = sys::uptime_ms();
    let mut t_clear = 0i64;
    let mut t_present = 0i64;
    let mut ok_frames = 0u32;

    for f in 0..frames {
        let phase = (f * 4) % 256;
        let rgba = 0xff000000u32 | ((phase as u32) << 16) | ((255 - phase as u32) << 8);

        let mut cmd = [0u8; 29];
        cmd[..5].copy_from_slice(b"GCLR3");
        cmd[5..13].copy_from_slice(&rt_h.to_le_bytes());
        cmd[13..17].copy_from_slice(&w.to_le_bytes());
        cmd[17..21].copy_from_slice(&h.to_le_bytes());
        cmd[21..25].copy_from_slice(&pitch.to_le_bytes());
        cmd[25..29].copy_from_slice(&rgba.to_le_bytes());

        let tc0 = sys::uptime_ms();
        let sub = sys::gpu_submit(&cmd);
        t_clear += sys::uptime_ms() - tc0;
        if sub < 0 {
            println!("soso-gfx-demo: GCLR3 falló en frame {f}");
            break;
        }

        if let Some((tex_h, tw, th, tpitch)) = quad {
            let dx = 80 + (f * 3) % (w.saturating_sub(tw));
            let dy = 80 + (f * 2) % (h.saturating_sub(th));
            let mut qcmd = [0u8; 49];
            qcmd[..5].copy_from_slice(b"GQUAD");
            qcmd[5..13].copy_from_slice(&rt_h.to_le_bytes());
            qcmd[13..21].copy_from_slice(&tex_h.to_le_bytes());
            qcmd[21..25].copy_from_slice(&dx.to_le_bytes());
            qcmd[25..29].copy_from_slice(&dy.to_le_bytes());
            qcmd[29..33].copy_from_slice(&tw.to_le_bytes());
            qcmd[33..37].copy_from_slice(&th.to_le_bytes());
            qcmd[37..41].copy_from_slice(&pitch.to_le_bytes());
            qcmd[41..45].copy_from_slice(&tpitch.to_le_bytes());
            if sys::gpu_submit(&qcmd) < 0 {
                println!("soso-gfx-demo: GQUAD falló en frame {f}");
                break;
            }
        }

        let tp0 = sys::uptime_ms();
        let pres = encode_present(present_tag, rt_h, w, h, pitch);
        if sys::gpu_submit(&pres) < 0 {
            println!("soso-gfx-demo: {pres_name} falló en frame {f}");
            break;
        }
        t_present += sys::uptime_ms() - tp0;
        ok_frames += 1;
    }

    let elapsed = (sys::uptime_ms() - t0).max(1);
    let fps = (ok_frames as i64 * 1000) / elapsed;
    let tag = if with_quad {
        alloc::format!("GCLR3+GQUAD+{pres_name}")
    } else {
        alloc::format!("GCLR3+{pres_name}")
    };
    println!(
        "soso-gfx-demo ({tag}): {fps} FPS ({ok_frames} frames, clear~{}ms present~{}ms)",
        t_clear / ok_frames.max(1) as i64,
        t_present / ok_frames.max(1) as i64,
    );

    if let Some((tex_h, _, _, _)) = quad {
        let _ = sys::gpu_free(tex_h);
    }
    let _ = sys::gpu_free(rt_h);
    if fb_ok {
        let _ = sys::fb_set_mode(soso_abi::FB_MODE_CONSOLE);
    }
    if ok_frames == 0 { 1 } else { 0 }
}
