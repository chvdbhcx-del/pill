//! Placement audit sheets (`cargo test --locked placement_sheets -- --ignored`): the island on the bottom edge and in the taskbar, every tab,
//! the folded capsule and the cards, drawn on a stage with the taskbar strip, the monitor's edge and the tray marked, for a look with the eyes.
//! Invented data only. PNGs land in `$PILL_AUDIT_DIR` (default `target-agent/placement`). One PNG-writing test per cargo invocation.
use super::fonts::FONTS;
use super::snapshots::{media_payload, Scn};
use glasskit::testkit::{Harness, Image};
use glasskit::Color;
use serde_json::{json, Value};
use std::path::PathBuf;

thread_local! {
    static H: Harness = Harness::with_fonts(&FONTS);
}

/// The taskbar strip's height (logical px) of the stage.
const BAR: f32 = 48.0;
const TABS: [&str; 10] = ["home", "weather", "controls", "notifs", "clipboard", "shelf", "note", "text", "chat", "agents"];

#[derive(Clone, Copy)]
struct Cfg {
    pos: &'static str,
    align: &'static str,
    light: bool,
    scale: f32,
}

impl Cfg {
    fn bar(&self) -> f32 {
        if self.pos == "taskbar" {
            BAR
        } else {
            0.0
        }
    }
    fn name(&self) -> String {
        format!("{}-{}-{}-{}", self.pos, self.align, if self.light { "light" } else { "dark" }, (self.scale * 100.0) as u32)
    }
}

fn out_dir() -> PathBuf {
    let d = std::env::var("PILL_AUDIT_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("target-agent").join("placement"));
    let _ = std::fs::create_dir_all(&d);
    d
}

fn settings(cfg: &Cfg, patch: Value) -> Value {
    let mut s = json!({"size":100,"theme": if cfg.light {"light"} else {"dark"},"clock24":true,"shape":"notch","character":"liquid","pet":"none","pet.side":"right","pet.size":"m",
        "musicPill":"title","net.weather":true,"live.obs":false,"language":"auto","edge.position":cfg.pos,"edge.align":cfg.align});
    for (k, v) in patch.as_object().into_iter().flatten() {
        s[k] = v.clone();
    }
    s
}

fn wx() -> Value {
    json!({"temp":18.4,"icon":"partly-day","text":"Partly cloudy"})
}

/// A scenario whose window is `h` logical px tall (plus the bar in the taskbar), as `pill.rs` makes it.
fn scn(cfg: &Cfg, h: f32, patch: Value) -> Scn {
    let mut s = Scn::new(settings(cfg, patch), (720.0, h + cfg.bar()), cfg.scale, false);
    let _ = s.draw(Color::rgba(0.0, 0.0, 0.0, 0.0));
    if cfg.bar() > 0.0 {
        s.emit("pill:taskbar", json!({ "h": BAR }));
    }
    s.emit("weather:current", wx());
    s
}

/// Lets the animation run for `ms` of simulated time and draws.
fn grab(s: &mut Scn, ms: f64) -> Image {
    let frames = (ms * s.sim.hz as f64 / 1000.0) as usize;
    let t = Color::rgba(0.0, 0.0, 0.0, 0.0);
    let _ = s.draw(t);
    s.settle(frames / 2);
    let _ = s.draw(t);
    s.settle(frames);
    s.draw(t)
}

fn over(dst: &mut [u8], src: &[u8]) {
    let a = src[3] as u32;
    for i in 0..3 {
        dst[i] = (src[i] as u32 + dst[i] as u32 * (255 - a) / 255).min(255) as u8;
    }
    dst[3] = 255;
}

/// The island's window on a stage: the desktop, the taskbar strip (under the window for the plain bottom edge, inside it in the bar), the
/// monitor's side (everything beyond is darkened) and the tray (the bar's right end).
fn stage(img: &Image, cfg: &Cfg) -> Image {
    let sc = cfg.scale;
    let bar = (BAR * sc).round() as u32;
    let extra = if cfg.pos == "bottom" { bar } else { 0 };
    let (w, h) = (img.w, img.h + extra);
    let (desk, strip, tray) = if cfg.light { ([0xc9, 0xcd, 0xd3], [0xe9, 0xea, 0xee], [0xd2, 0xd4, 0xda]) } else { ([0x5b, 0x60, 0x68], [0x20, 0x21, 0x24], [0x30, 0x32, 0x36]) };
    let mut out = vec![0u8; (w * h * 4) as usize];
    let edge = (12.0 * sc).round() as u32; // the window overhangs the monitor / tray by 12 px at the taskbar's left / right
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let in_bar = y >= h - bar;
            let c = if in_bar && cfg.pos == "taskbar" && cfg.align == "right" && x >= w - edge {
                tray
            } else if in_bar {
                strip
            } else {
                desk
            };
            out[i..i + 4].copy_from_slice(&[c[2], c[1], c[0], 255]);
        }
    }
    for y in 0..img.h {
        for x in 0..w {
            let (s, d) = (((y * w + x) * 4) as usize, ((y * w + x) * 4) as usize);
            let px: [u8; 4] = img.bgra[s..s + 4].try_into().unwrap();
            over(&mut out[d..d + 4], &px);
        }
    }
    // beyond the monitor
    let (off_l, off_r) = match (cfg.pos, cfg.align) {
        ("taskbar", "left") => (edge, 0),
        _ => (0, 0),
    };
    for y in 0..h {
        for x in 0..w {
            if x < off_l || x >= w - off_r {
                let i = ((y * w + x) * 4) as usize;
                for c in 0..3 {
                    out[i + c] /= 3;
                }
            }
        }
    }
    // the monitor's side: a magenta hairline
    let line = match (cfg.pos, cfg.align) {
        ("taskbar", "left") => Some(edge),
        ("bottom", "left") => Some(0),
        ("bottom", "right") => Some(w - 1),
        _ => None,
    };
    if let Some(x) = line {
        for y in 0..h {
            let i = ((y * w + x.min(w - 1)) * 4) as usize;
            out[i..i + 4].copy_from_slice(&[0xff, 0x00, 0xff, 0xff]);
        }
    }
    Image { w, h, scale: sc, bgra: out }
}

fn grid(tiles: &[Image], cols: usize) -> Image {
    let gap = 8u32;
    let rows = tiles.len().div_ceil(cols);
    let cw = tiles.iter().map(|t| t.w).max().unwrap_or(1);
    let rh: Vec<u32> = (0..rows).map(|r| tiles[r * cols..((r + 1) * cols).min(tiles.len())].iter().map(|t| t.h).max().unwrap_or(0)).collect();
    let (w, h) = (cols as u32 * cw + (cols as u32 + 1) * gap, rh.iter().sum::<u32>() + (rows as u32 + 1) * gap);
    let mut out = vec![0u8; (w * h * 4) as usize];
    for p in out.chunks_exact_mut(4) {
        p.copy_from_slice(&[0x10, 0x10, 0x10, 0xff]);
    }
    let mut y0 = gap;
    for r in 0..rows {
        for c in 0..cols {
            let Some(t) = tiles.get(r * cols + c) else { continue };
            let x0 = gap + c as u32 * (cw + gap);
            for y in 0..t.h {
                let d = (((y0 + y) * w + x0) * 4) as usize;
                let s = (y * t.w * 4) as usize;
                out[d..d + (t.w * 4) as usize].copy_from_slice(&t.bgra[s..s + (t.w * 4) as usize]);
            }
        }
        y0 += rh[r] + gap;
    }
    Image { w, h, scale: 1.0, bgra: out }
}

fn save(img: &Image, dir: &std::path::Path, name: &str) {
    H.with(|h| img.save_png(h, dir.join(format!("{name}.png"))).expect("png"));
}

/// The folded capsule in several live states and the cards, stacked.
fn folded_sheet(cfg: &Cfg) -> Image {
    let mut tiles = Vec::new();
    let mut s = scn(cfg, 120.0, json!({}));
    tiles.push(stage(&grab(&mut s, 900.0), cfg));
    let mut s = scn(cfg, 120.0, json!({"showDate": true}));
    tiles.push(stage(&grab(&mut s, 900.0), cfg));
    let mut s = scn(cfg, 120.0, json!({}));
    s.emit("media:changed", media_payload());
    tiles.push(stage(&grab(&mut s, 1000.0), cfg));
    let mut s = scn(cfg, 120.0, json!({"pet":"cat","character":"legs"}));
    tiles.push(stage(&grab(&mut s, 1200.0), cfg));
    let mut s = scn(cfg, 120.0, json!({}));
    s.emit("rec:state", json!({"recording":true,"startedAt": 1_793_356_440_000i64 - 65_000,"mode":"screen","fps":30}));
    tiles.push(stage(&grab(&mut s, 1000.0), cfg));
    for (ev, payload) in [
        ("capture:done", json!({"kind":"screenshot","path":"C:\\Demo\\Pictures\\shot-0042.png","copied":true,"w":1920,"h":1080})),
        ("live:notification", json!({"app":"Slack","title":"Anna K.","body":"Are we still on for the 3 pm review? I pushed the new build.","id":"n1"})),
        ("uninstall:ready", json!({"name":"Contoso Editor","kind":"app","count":3})),
        ("phone:notif", json!({ "kind": "posted", "notif": { "key": "k1", "pkg": "org.telegram.messenger", "app": "Telegram", "title": "Demo", "text": "hi, how are you?", "ongoing": false, "replay": false, "fresh": true, "canReply": true, "avatar": null, "icon": null } })),
    ] {
        let mut s = scn(cfg, 200.0, json!({}));
        s.emit(ev, payload);
        tiles.push(stage(&grab(&mut s, 1300.0), cfg));
    }
    grid(&tiles, 2)
}

fn tab_tile(cfg: &Cfg, tab: &str) -> Image {
    let mut s = scn(cfg, 480.0, json!({}));
    s.emit("live:notification", json!({"app":"Mail","title":"Demo Sender","body":"Invented body text for the audit.","id":"n1"}));
    s.emit("media:changed", media_payload());
    s.emit("pill:expand", json!({"tab": tab, "via": "test"}));
    stage(&grab(&mut s, 1400.0), cfg)
}

fn tabs_sheet(cfg: &Cfg, tabs: &[&str], cols: usize) -> Image {
    grid(&tabs.iter().map(|t| tab_tile(cfg, t)).collect::<Vec<_>>(), cols)
}

#[test]
#[ignore = "writes PNGs for a look with the eyes; run on its own"]
fn placement_sheets() {
    let dir = out_dir();
    let all = std::env::var("PILL_AUDIT").unwrap_or_else(|_| "main".into());
    let mut cfgs = Vec::new();
    for pos in ["bottom", "taskbar"] {
        for align in ["left", "center", "right"] {
            cfgs.push(Cfg { pos, align, light: false, scale: 1.0 });
        }
    }
    if all == "variants" {
        cfgs.clear();
        for (pos, align, light, scale) in [("bottom", "left", true, 1.5), ("taskbar", "right", true, 2.0), ("taskbar", "center", false, 1.5), ("taskbar", "left", true, 1.0), ("bottom", "right", false, 2.0)] {
            cfgs.push(Cfg { pos, align, light, scale });
        }
    }
    let only = std::env::var("PILL_AUDIT_ONLY").ok();
    for cfg in cfgs {
        let name = cfg.name();
        if only.as_deref().is_some_and(|o| !name.contains(o)) {
            continue;
        }
        save(&folded_sheet(&cfg), &dir, &format!("{name}-folded"));
        save(&tabs_sheet(&cfg, &TABS[..4], 2), &dir, &format!("{name}-tabs1"));
        save(&tabs_sheet(&cfg, &TABS[4..8], 2), &dir, &format!("{name}-tabs2"));
        save(&tabs_sheet(&cfg, &TABS[8..], 2), &dir, &format!("{name}-tabs3"));
    }
}

