//! Headless render check: draws every pet in every state into PNG files through WARP (software), no window, no GPU needed.
//!   cargo run --release --example pets_dump -- <outdir> [--t 700] [--scale 2] [--box 64] [--margin 16] [--hooks] [--side left] [ids...]
//! One file per pet and state: <id>_<state>.png (+ <id>_poked.png), pet over black at time t after the state was set. tools/dev/pets-diff.mjs
//! compares these with the web version.
#![allow(dead_code)]
#[path = "../src/ui/pets/mod.rs"]
mod pets;

use glasskit::gfx::col;
use glasskit::{Gfx, Rect};
use pets::characters::Characters;
use pets::engine::{catalogue, PetEngine, Side, Size};
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Dxgi::*;

fn warp() -> Option<IDXGIAdapter1> {
    unsafe {
        let f: IDXGIFactory4 = CreateDXGIFactory1().ok()?;
        f.EnumWarpAdapter::<IDXGIAdapter1>().ok()
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().expect("usage: pets_dump <outdir> [options] [ids...]");
    let (mut t, mut scale, mut bx, mut margin, mut hooks, mut side) = (700.0f32, 2.0f32, 64.0f32, 16.0f32, false, Side::Right);
    let mut ids: Vec<String> = Vec::new();
    let rest: Vec<String> = args.collect();
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--t" => {
                t = rest[i + 1].parse().unwrap();
                i += 1;
            }
            "--scale" => {
                scale = rest[i + 1].parse().unwrap();
                i += 1;
            }
            "--box" => {
                bx = rest[i + 1].parse().unwrap();
                i += 1;
            }
            "--margin" => {
                margin = rest[i + 1].parse().unwrap();
                i += 1;
            }
            "--hooks" => hooks = true,
            "--chars" => {}
            "--side" => {
                side = if rest[i + 1] == "left" { Side::Left } else { Side::Right };
                i += 1;
            }
            id => ids.push(id.to_string()),
        }
        i += 1;
    }
    std::fs::create_dir_all(&out).unwrap();
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
    }
    let w = warp();
    let gfx = Gfx::new(w.as_ref()).expect("D3D11 / WARP device");
    eprintln!("device: {}", if gfx.warp { "WARP (fallback)" } else { "WARP adapter" });
    if rest.iter().any(|a| a == "--chars") {
        dump_chars(&gfx, &out, t, scale);
        return;
    }
    let list: Vec<String> = if ids.is_empty() { catalogue().iter().map(|p| p.id.to_string()).collect() } else { ids };
    let states = ["calm", "music", "thinking", "done", "attention", "file", "sleep", "poked"];
    for id in &list {
        let (bw, bh) = {
            let ar = pets::engine::load(id).expect("pet").ar;
            (bx * ar, bx)
        };
        let (cw, ch) = (((bw + 2.0 * margin) * scale).ceil() as u32, ((bh + 2.0 * margin) * scale).ceil() as u32);
        let target = gfx.target(cw, ch).unwrap();
        let cpu = gfx.cpu_read(cw, ch).unwrap();
        let mut eng = PetEngine::new(&gfx).unwrap();
        for st in states {
            let (name, poke) = if st == "poked" { ("calm", true) } else { (st, false) };
            eng.set_pet("none-yet", side, Size::M);
            eng.set_idle(false);
            eng.set_state(name, None); // a pet created in a state starts there without a transition
            eng.set_pet(id, side, Size::M);
            eng.sim_mut().unwrap().hooks_on = hooks;
            eng.tick(0.0);
            if poke {
                assert!(eng.poke());
            }
            let mut left = t;
            while left > 0.0 {
                let dt = left.min(10.0);
                eng.tick(dt);
                left -= dt;
            }
            unsafe {
                let dc = &gfx.dc;
                dc.SetTarget(&target);
                dc.BeginDraw();
                dc.SetTransform(&glasskit::gfx::mat(scale, 0.0, 0.0));
                dc.Clear(Some(&col(0.0, 0.0, 0.0, 1.0)));
                eng.draw(dc, Rect::new(margin, margin, bw, bh), scale).unwrap();
                dc.EndDraw(None, None).unwrap();
                dc.SetTarget(None::<&ID2D1Image>);
            }
            let mut px = vec![0u8; (cw * ch * 4) as usize];
            gfx.read_rect(&target, &cpu, windows::Win32::Foundation::RECT { left: 0, top: 0, right: cw as i32, bottom: ch as i32 }, |y, row| {
                let o = (y as u32 * cw * 4) as usize;
                px[o..o + row.len()].copy_from_slice(row);
            })
            .unwrap();
            let file = format!("{out}/{id}_{st}.png");
            gfx.save_png(&file, cw, ch, &px).unwrap();
        }
        eprintln!("{id}: ok");
    }
}

/// The capsule characters (tail, legs + eyes) as the Settings preview card draws them (149 x 92 css px), at time `t` ms after the state was set.
/// The web side calls PillChars.preview with a frozen clock (performance.now = 10000 ms at setState), so absolute time is 10 s + t.
fn dump_chars(gfx: &Gfx, out: &str, t: f32, scale: f32) {
    let (w, h) = (149.0f32, 92.0f32);
    let (cw, ch) = ((w * scale) as u32, (h * scale) as u32);
    let target = gfx.target(cw, ch).unwrap();
    let cpu = gfx.cpu_read(cw, ch).unwrap();
    let names = ["calm", "music", "thinking", "done", "attention", "file", "sleep"];
    let mut c = Characters::new(&gfx.dc).unwrap();
    for kind in ["tail", "legs"] {
        for (si, st) in names.iter().enumerate() {
            c.set_state(st, None);
            let frame = |c: &mut Characters, abs: f32, rt: f32, keep: bool| {
                let look = [0.8 * (abs * 0.7).sin(), 0.4 * (abs * 0.45 + 1.0).sin()];
                c.set_clock(abs, rt, look);
                unsafe {
                    let dc = &gfx.dc;
                    dc.SetTarget(&target);
                    dc.BeginDraw();
                    dc.SetTransform(&glasskit::gfx::mat(scale, 0.0, 0.0));
                    dc.Clear(Some(&col(0.0, 0.0, 0.0, 1.0)));
                    if kind == "tail" {
                        c.draw_tail(dc, 93.3, 28.5, 1.8);
                    } else {
                        c.draw_legs(dc, 74.5, 36.0, 1.4);
                        c.draw_eyes(dc, 74.5, 18.0, 1.15);
                    }
                    dc.EndDraw(None, None).unwrap();
                    dc.SetTarget(None::<&ID2D1Image>);
                }
                let _ = keep;
            };
            frame(&mut c, 10.0, 0.0, false);
            frame(&mut c, 10.0 + t / 1000.0, t / 1000.0, true);
            let mut px = vec![0u8; (cw * ch * 4) as usize];
            gfx.read_rect(&target, &cpu, windows::Win32::Foundation::RECT { left: 0, top: 0, right: cw as i32, bottom: ch as i32 }, |y, row| {
                let o = (y as u32 * cw * 4) as usize;
                px[o..o + row.len()].copy_from_slice(row);
            })
            .unwrap();
            gfx.save_png(&format!("{out}/chars_{kind}_{st}.png"), cw, ch, &px).unwrap();
            let _ = si;
        }
    }
    eprintln!("chars: ok");
}
