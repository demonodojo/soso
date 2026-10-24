//! Demo H1: limpia un render target en VRAM con el motor 3D y lo presenta en GOP.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use libsoso::{println, sys};
use soso_abi::GpuInfo;

libsoso::entry!(main);

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

fn main(args: &[String]) -> u8 {
    let frames = parse_frames(args);
    let mut info = GpuInfo::default();
    if sys::gpu_info(&mut info) < 0 || info.present == 0 {
        println!("soso-gfx-demo: sin GPU");
        return 1;
    }
    if info._pad[0] == 0 {
        println!("soso-gfx-demo: motor 3D no disponible (gfx3d=0)");
        return 1;
    }

    let mut fb = soso_abi::FbInfo::default();
    if sys::fb_info(&mut fb) < 0 || fb.present == 0 {
        println!("soso-gfx-demo: sin framebuffer GOP");
        return 1;
    }
    let _ = sys::fb_set_mode(soso_abi::FB_MODE_GRAPHICS);

    let w = fb.width as u32;
    let h = fb.height as u32;
    let pitch = w.saturating_mul(4);
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

        let tp0 = sys::uptime_ms();
        let mut pres = [0u8; 25];
        pres[..5].copy_from_slice(b"GPRES");
        pres[5..13].copy_from_slice(&rt_h.to_le_bytes());
        pres[13..17].copy_from_slice(&w.to_le_bytes());
        pres[17..21].copy_from_slice(&h.to_le_bytes());
        pres[21..25].copy_from_slice(&pitch.to_le_bytes());
        if sys::gpu_submit(&pres) < 0 {
            println!("soso-gfx-demo: GPRES falló en frame {f}");
            break;
        }
        t_present += sys::uptime_ms() - tp0;
        ok_frames += 1;
    }

    let elapsed = (sys::uptime_ms() - t0).max(1);
    let fps = (ok_frames as i64 * 1000) / elapsed;
    println!(
        "soso-gfx-demo: {fps} FPS ({ok_frames} frames, clear~{}ms present~{}ms)",
        t_clear / ok_frames.max(1) as i64,
        t_present / ok_frames.max(1) as i64,
    );

    let _ = sys::gpu_free(rt_h);
    let _ = sys::fb_set_mode(soso_abi::FB_MODE_CONSOLE);
    if ok_frames == 0 { 1 } else { 0 }
}
