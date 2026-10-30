//! Painting one frame of the capture overlay. Everything is drawn from the cached `cap::CaptureView` view-model, in the z-order of
//! `capture.css`: frame, dim + cut-out, handles / size badge, markup (clipped to the selection), text box, hint, toolbar, tip, pixel
//! marker, loupe, copy flash, toast.
use super::icons::IconSet;
use super::tools::{self, rgba_to_bgra, FrameSrc, Tools};
use super::{CaptureView, DIM_RAMP_MS, HINT_IN_MS, TIP_DELAY_MS, TOAST_MS};
use glasskit::canvas::scale_about;
use glasskit::color::Color;
use glasskit::geom::Rect;
use glasskit::gfx::rf;
use glasskit::text::Style;
use glasskit::theme::Theme;
use glasskit::Canvas;
use pill_ui_core::capture as cap;
use pill_ui_core::capture::{AnimKind, AnimTarget, Anim, DrawCmd};
use windows::Win32::Graphics::Direct2D::*;

/// Per-device GPU state of one overlay.
pub(crate) struct Gpu {
    dev_id: usize,
    _dev: ID2D1DeviceContext,
    pub tools: Tools,
    pub icons: IconSet,
    pub frame: Option<ID2D1Bitmap1>,
}

const BLACK: Color = Color::rgba(0.0, 0.0, 0.0, 1.0);
const WHITE: Color = Color::rgba(1.0, 1.0, 1.0, 1.0);

fn gr(r: &cap::Rect) -> Rect {
    Rect::new(r.x as f32, r.y as f32, r.w as f32, r.h as f32)
}

fn ease_out(t: f64) -> f32 {
    glasskit::ease::curves::EASE_OUT.at(t.clamp(0.0, 1.0) as f32)
}

fn find<'a>(anims: &'a [Anim], kind: AnimKind, target: &AnimTarget) -> Option<&'a Anim> {
    anims.iter().find(|a| a.spec.kind == kind && &a.target == target)
}

fn find_kind(anims: &[Anim], kind: AnimKind) -> Option<&Anim> {
    anims.iter().find(|a| a.spec.kind == kind)
}

pub(crate) fn paint(v: &mut CaptureView, cv: &Canvas, theme: &Theme, rect: Rect, cx: &crate::ui::view::Cx) {
    let now = cx.now_ms;
    let nowu = now as u64;
    // ---- device resources (rebuilt when the host made a new device) ----
    let id = tools::device_id(cv.gfx);
    if v.gpu.as_ref().is_none_or(|g| g.dev_id != id) {
        v.gpu = Tools::new(cv.gfx).ok().map(|tools| Gpu { dev_id: id, _dev: cv.gfx.dc.clone(), tools, icons: IconSet::new(), frame: None });
        v.frame_dirty = true;
    }
    // ---- the viewport the model lays out in must be this window ----
    if (v.viewport.0 - rect.w).abs() > 0.01 || (v.viewport.1 - rect.h).abs() > 0.01 {
        v.viewport = (rect.w, rect.h);
        v.model.on_input(cap::Input::Resize { vw: rect.w as f64, vh: rect.h as f64 }, &mut super::io::NoIo(nowu));
        v.vm = v.model.view(&v.lang);
    }
    let (fw, fh) = v.model.frame_size();
    let frame_rgba = v.frame_rgba.clone();
    {
        let Some(gpu) = v.gpu.as_mut() else { return };
        if v.frame_dirty {
            gpu.frame = None;
            gpu.tools.clear_pix();
            if let Some(f) = frame_rgba.as_ref().filter(|f| fw > 0 && fh > 0 && f.len() == fw as usize * fh as usize * 4) {
                gpu.frame = cv.gfx.from_bgra(fw as u32, fh as u32, &rgba_to_bgra(f)).ok();
            }
            v.frame_dirty = false;
        }
    }
    let Some(gpu) = v.gpu.as_ref() else { return };
    let vm = &v.vm;
    let reduce = cx.reduced_motion || v.opts.reduce_motion;
    let ramp = |from: f64, dur: f64| if reduce { 1.0 } else { ease_out((now - from) / dur) };
    let dim_k = ramp(v.armed_ms, DIM_RAMP_MS);
    let (w, h) = (rect.w, rect.h);

    // ---- 1. the frozen frame ----
    if let Some(bmp) = &gpu.frame {
        unsafe { cv.dc.DrawBitmap(bmp, Some(&rf(0.0, 0.0, w, h)), 1.0, D2D1_INTERPOLATION_MODE_LINEAR, None, None) };
    }

    // ---- 2. dim and the cut-out ----
    if vm.dim_full {
        cv.fill_rect(rect, BLACK.alpha(0.38 * dim_k));
    }
    let ghost_sel = vm.ghost.as_ref().map(|g| g.selection).filter(|s| s.w > 0.0 && s.h > 0.0);
    let cut = match (&vm.selection, ghost_sel) {
        (Some(s), _) => Some((gr(&s.rect), 0.46 * dim_k)),
        (None, Some(g)) => {
            // exit: the dim around the confirmed area fades out with the flash instead of snapping off
            let k = find_kind(&vm.anims, AnimKind::CopyFlash).or_else(|| find_kind(&vm.anims, AnimKind::ToolbarLeave)).map_or(0.0, |a| {
                let f = a.frame(nowu);
                (f.opacity / a.spec.opacity_from.max(0.01)) as f32
            });
            Some((gr(&g), 0.46 * k))
        }
        _ => None,
    };
    if let Some((s, a)) = cut {
        let shade = BLACK.alpha(a);
        cv.fill_rect(Rect::new(0.0, 0.0, w, s.y.max(0.0)), shade);
        cv.fill_rect(Rect::new(0.0, s.bottom(), w, (h - s.bottom()).max(0.0)), shade);
        cv.fill_rect(Rect::new(0.0, s.y, s.x.max(0.0), s.h), shade);
        cv.fill_rect(Rect::new(s.right(), s.y, (w - s.right()).max(0.0), s.h), shade);
        if vm.selection.is_some() && (s.w >= 1.0 || s.h >= 1.0) {
            cv.stroke_rr(s.inset(-1.0), 0.0, 1.0, WHITE.alpha(0.92)); // CSS `outline`: 1 px just outside the box
        }
    }

    // ---- 3. handles and the size badge ----
    if let Some(sel) = &vm.selection {
        let s = gr(&sel.rect);
        let pop = find(&vm.anims, AnimKind::HandlesPop, &AnimTarget::Handles).map(|a| a.frame(nowu));
        for hd in &sel.handles {
            let (k, o) = pop.map_or((1.0, 1.0), |f| (f.progress as f32, f.opacity as f32));
            let c = (hd.at.x as f32, hd.at.y as f32);
            cv.with_opacity(o, |cv| {
                cv.fill_circle(c, 4.5 * k.max(0.0), theme.accent);
                cv.fill_circle(c, 3.0 * k.max(0.0), WHITE);
            });
        }
        if s.w >= 1.0 || s.h >= 1.0 {
            let st = Style::new(11.0, 600).mono();
            let tw = cv.text_width(&sel.size_label, st);
            let (bw, bh) = (tw + 14.0, 20.0);
            let at = if sel.size_inside { (s.x + 6.0, s.y + 6.0) } else { (s.x, s.y - 6.0 - bh) };
            let b = Rect::new(at.0, at.1, bw, bh);
            cv.fill_rr(b, 7.0, BLACK.alpha(0.78));
            cv.text(b, &sel.size_label, st.center(), WHITE);
        }
    }

    // ---- 4. markup, clipped to the selection ----
    if let (Some(sel), Some(src)) = (&vm.selection, frame_rgba.as_ref()) {
        let s = gr(&sel.rect);
        let (rx, ry) = (vm.scale.rx as f32, vm.scale.ry as f32);
        let fsrc = FrameSrc { rgba: src, fw, fh };
        cv.with_clip(s, |cv| {
            cv.with_transform(scale_about((0.0, 0.0), 1.0 / rx, 1.0 / ry), |cv| {
                for op in vm.ops.iter().chain(vm.draft.iter()) {
                    let ins = find(&vm.anims, AnimKind::OpInsert, &AnimTarget::Op(op.id)).map(|a| a.frame(nowu));
                    let k = ins.map_or(1.0, |f| f.opacity as f32);
                    let sc = ins.map_or(1.0, |f| f.scale as f32);
                    let center = op_center(gpu, cv, &op.cmds);
                    let draw = |cv: &Canvas| {
                        for c in &op.cmds {
                            gpu.tools.draw_cmd(cv.gfx, c, fsrc, k);
                        }
                    };
                    match center {
                        Some(c) if (sc - 1.0).abs() > 1e-3 => cv.with_scale_about(c, sc, draw),
                        _ => draw(cv),
                    }
                    if op.selected {
                        if let Some(b) = op_bounds(gpu, cv, &op.cmds) {
                            let r = (b.x as f32 - 4.0 * rx, b.y as f32 - 4.0 * ry, (b.right() as f32 + 4.0 * rx, b.bottom() as f32 + 4.0 * ry));
                            unsafe {
                                cv.dc.DrawRectangle(&rf(r.0, r.1, r.2 .0, r.2 .1), gpu.tools.brush(WHITE.alpha(0.9)), 1.5 * rx, gpu.tools.dashed());
                            }
                        }
                    }
                }
            });
        });
    }

    // ---- 5. the text box ----
    if let Some(t) = &vm.text_edit {
        let col = tools::parse_color(&t.color);
        let (px, wt) = (16.0f32, 600u16);
        let (tw, _) = gpu.tools.measure(cv.gfx, &t.text, px, wt);
        let box_ = Rect::new(t.pos.x as f32, t.pos.y as f32, (tw + 4.0).max(40.0), 20.0);
        unsafe {
            let o = box_.inset(-0.5);
            cv.dc.DrawRectangle(&rf(o.x, o.y, o.right(), o.bottom()), gpu.tools.brush(WHITE.alpha(0.8)), 1.0, gpu.tools.dashed());
        }
        gpu.tools.draw_text_outlined(cv.gfx, &t.text, pt_f(box_.x + 2.0, box_.y + 0.0), col, px, wt, 3.0, 1.0);
        let cx_ = box_.x + 2.0 + gpu.tools.caret_x(cv.gfx, &t.text, px, wt, t.caret);
        if reduce || ((now / 530.0) as i64) % 2 == 0 {
            cv.fill_rect(Rect::new(cx_, box_.y + 1.0, 1.5, 18.0), col);
        }
    }

    // ---- 6. the hint pill ----
    if let Some(hint) = &vm.hint {
        let a = if hint.visible {
            ramp(v.armed_ms + 60.0, HINT_IN_MS)
        } else {
            find(&vm.anims, AnimKind::HintFade, &AnimTarget::Hint).map_or(0.0, |a| a.frame(nowu).opacity as f32)
        };
        if a > 0.0 && !hint.text.is_empty() {
            let st = Style::new(12.0, 600);
            let tw = cv.text_width(&hint.text, st);
            let (pw, ph) = (tw + 30.0, 32.0);
            let r = Rect::new((w - pw) / 2.0, 22.0, pw, ph);
            cv.with_opacity(a, |cv| {
                cv.fill_rr(r, 16.0, BLACK.alpha(0.82));
                cv.stroke_rr(r, 16.0, 1.0, WHITE.alpha(0.12));
                cv.text(r.inset_xy(0.0, 0.0), &hint.text, st.center(), WHITE);
            });
        }
    }

    // ---- 7. toolbar (and its exit ghost), tip ----
    if let Some(tb) = &vm.toolbar {
        let en = find(&vm.anims, AnimKind::ToolbarEnter, &AnimTarget::Toolbar).map(|a| a.frame(nowu));
        draw_bar(v, gpu, cv, tb, en.map_or(1.0, |f| f.opacity as f32), en.map_or(0.0, |f| f.dy as f32));
        draw_tip(v, cv, theme, tb, rect, now, reduce);
    } else if let Some(tb) = vm.ghost.as_ref().and_then(|g| g.toolbar.as_ref()) {
        let lv = find(&vm.anims, AnimKind::ToolbarLeave, &AnimTarget::Toolbar).map(|a| a.frame(nowu));
        if let Some(f) = lv {
            draw_bar(v, gpu, cv, tb, f.opacity as f32, 0.0);
        }
    }

    // ---- 8. colour picker: marker + lens ----
    if let Some(l) = &vm.loupe {
        let m = Rect::new(l.marker.x as f32 - 7.0, l.marker.y as f32 - 7.0, 14.0, 14.0);
        cv.stroke_rr(m.inset(-1.0), 4.0, 1.0, BLACK);
        cv.stroke_rr(m, 3.0, 1.5, WHITE);
        cv.stroke_rr(m.inset(1.5), 2.0, 1.0, BLACK);
        let show = find(&vm.anims, AnimKind::LoupeShow, &AnimTarget::Loupe).map(|a| a.frame(nowu));
        let mut pos = (l.box_pos.x as f32, l.box_pos.y as f32);
        if let (Some(from), Some(a)) = (vm.loupe_from, find(&vm.anims, AnimKind::LoupeFollow, &AnimTarget::Loupe)) {
            let p = a.frame(nowu).progress as f32;
            pos = (from.x as f32 + (pos.0 - from.x as f32) * p, from.y as f32 + (pos.1 - from.y as f32) * p);
        }
        let (o, sc) = show.map_or((1.0, 1.0), |f| (f.opacity as f32, f.scale as f32));
        // the browser snaps the lens box to whole device px: the 1 px grid lines stay crisp
        let k = cx.scale.max(0.25);
        let pos = ((pos.0 * k).round() / k, (pos.1 * k).round() / k);
        let c = (pos.0 + LENS_W / 2.0, pos.1 + LENS_H / 2.0);
        cv.with_opacity(o, |cv| cv.with_scale_about(c, sc, |cv| draw_lens(cv, theme, l, pos)));
    }

    // ---- 9. copy flash ----
    if let Some(a) = find_kind(&vm.anims, AnimKind::CopyFlash) {
        let r = vm.selection.as_ref().map(|s| gr(&s.rect)).or(ghost_sel.as_ref().map(gr));
        if let Some(r) = r {
            cv.fill_rect(r, WHITE.alpha(a.frame(nowu).opacity as f32));
        }
    }

    // ---- 10. toast ----
    if let Some(t) = &v.toast {
        let el = now - t.since;
        let a = if reduce {
            if el < TOAST_MS { 1.0 } else { 0.0 }
        } else {
            (ease_out(el / 160.0) * (1.0 - ease_out((el - TOAST_MS) / 240.0))).clamp(0.0, 1.0)
        };
        if a > 0.0 {
            let st = Style::new(13.0, 600);
            let tw = cv.text_width(&t.text, st);
            let (pw, ph) = (tw + 36.0, 36.0);
            let rise = if reduce { 0.0 } else { (1.0 - ease_out(el / 200.0)) * 8.0 };
            let r = Rect::new((w - pw) / 2.0, h - 72.0 + rise, pw, ph);
            cv.with_opacity(a, |cv| {
                cv.shadow_rr(r, 18.0, 16.0, 4.0, BLACK.alpha(0.4));
                cv.fill_rr(r, 18.0, BLACK.alpha(0.88));
                cv.stroke_rr(r, 18.0, 1.0, WHITE.alpha(0.14));
                cv.text(r, &t.text, st.center(), WHITE);
            });
        }
    }
}

fn pt_f(x: f32, y: f32) -> cap::Pt {
    cap::Pt::new(x as f64, y as f64)
}

fn op_bounds(gpu: &Gpu, cv: &Canvas, cmds: &[DrawCmd]) -> Option<cap::Rect> {
    let mut acc: Option<cap::Rect> = None;
    for c in cmds {
        if let Some(b) = gpu.tools.cmd_bounds(cv.gfx, c) {
            acc = Some(match acc {
                None => b,
                Some(a) => {
                    let (l, t) = (a.x.min(b.x), a.y.min(b.y));
                    cap::Rect::new(l, t, a.right().max(b.right()) - l, a.bottom().max(b.bottom()) - t)
                }
            });
        }
    }
    acc
}

fn op_center(gpu: &Gpu, cv: &Canvas, cmds: &[DrawCmd]) -> Option<(f32, f32)> {
    op_bounds(gpu, cv, cmds).map(|b| (b.center().x as f32, b.center().y as f32))
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Toolbar
// ---------------------------------------------------------------------------------------------------------------------------------------

const ICON: Color = Color::rgb8(0xb9, 0xb9, 0xb9);

fn draw_bar(v: &CaptureView, gpu: &Gpu, cv: &Canvas, tb: &cap::toolbar::ToolbarVm, opacity: f32, dy: f32) {
    use cap::toolbar::Variant;
    let accent = Color::rgb8(0x8f, 0x96, 0xff);
    let r = gr(&tb.rect);
    cv.with_opacity(opacity, |cv| {
        cv.with_offset(0.0, dy, |cv| {
            cv.shadow_rr(r, tb.corner_radius as f32, 28.0, 8.0, BLACK.alpha(0.5));
            cv.fill_rr(r, tb.corner_radius as f32, Color::rgba(12.0 / 255.0, 12.0 / 255.0, 12.0 / 255.0, 0.96));
            cv.stroke_rr(r, tb.corner_radius as f32, 1.0, WHITE.alpha(0.1));
            for it in &tb.items {
                let ir = Rect::new(r.x + it.rect.x as f32, r.y + it.rect.y as f32, it.rect.w as f32, it.rect.h as f32);
                let fx = v.fx.get(&it.id);
                let hover = fx.map_or(0.0, |f| f.hover.value()).clamp(0.0, 1.0);
                let press = fx.map_or(1.0, |f| f.press.value());
                match it.variant {
                    Variant::Separator => cv.fill_rect(ir, WHITE.alpha(0.14)),
                    Variant::Swatch => {
                        let c = ir.center();
                        let pop = find(&v.vm.anims, AnimKind::SwatchPop, &AnimTarget::ToolbarItem(it.color.clone())).map(|a| a.frame(v.now as u64).scale as f32);
                        let k = (1.0 + 0.15 * hover).max(pop.unwrap_or(1.0)) * press.min(1.0).max(0.92);
                        cv.with_scale_about(c, k, |cv| {
                            let col = tools::parse_color(&it.color);
                            cv.fill_circle(c, 7.0, col);
                            if it.on {
                                cv.stroke_circle(c, 8.0, 2.0, WHITE);
                            } else {
                                cv.stroke_circle(c, 7.75, 1.5, WHITE.alpha(0.25));
                            }
                        });
                    }
                    _ => {
                        cv.with_opacity(if it.disabled { 0.35 } else { 1.0 }, |cv| {
                            cv.with_scale_about(ir.center(), press, |cv| {
                                let (bg, fg) = match it.variant {
                                    Variant::Go => (accent.mix(WHITE, 0.12 * hover), BLACK),
                                    Variant::Cancel => (WHITE.alpha(0.1 * hover), ICON.mix(Color::rgb8(0xff, 0x6b, 0x62), hover)),
                                    _ if it.on => (WHITE.alpha(0.16), WHITE),
                                    _ => (WHITE.alpha(0.1 * hover), ICON.mix(WHITE, hover)),
                                };
                                cv.fill_rr(ir, 9.0, bg);
                                gpu.icons.draw(cv, it.icon, Rect::new(ir.cx() - 9.0, ir.cy() - 9.0, 18.0, 18.0), fg);
                            });
                        });
                    }
                }
            }
        });
    });
}

/// The tooltip of the hovered toolbar item (the web page used the browser's `title`).
fn draw_tip(v: &CaptureView, cv: &Canvas, theme: &Theme, tb: &cap::toolbar::ToolbarVm, rect: Rect, now: f64, reduce: bool) {
    let Some(id) = &v.hover_id else { return };
    let Some(it) = tb.items.iter().find(|i| &i.id == id) else { return };
    let el = now - v.hover_since - TIP_DELAY_MS;
    if el < 0.0 || it.tip.is_empty() {
        return;
    }
    let a = if reduce { 1.0 } else { ease_out(el / 120.0) };
    let st = Style::new(12.0, 500);
    let tw = cv.text_width(&it.tip, st);
    let (pw, ph) = (tw + 20.0, 26.0);
    let bar = gr(&tb.rect);
    let ir = Rect::new(bar.x + it.rect.x as f32, bar.y, it.rect.w as f32, bar.h);
    let x = (ir.cx() - pw / 2.0).clamp(8.0, (rect.w - pw - 8.0).max(8.0));
    let below = bar.bottom() + 8.0 + ph <= rect.h - 4.0;
    let y = if below { bar.bottom() + 8.0 } else { bar.y - 8.0 - ph };
    let r = Rect::new(x, y, pw, ph);
    let rise = if reduce { 0.0 } else { (1.0 - a) * if below { -3.0 } else { 3.0 } };
    cv.with_opacity(a, |cv| {
        cv.with_offset(0.0, rise, |cv| {
            cv.shadow_rr(r, 8.0, 10.0, 3.0, BLACK.alpha(0.4));
            cv.fill_rr(r, 8.0, theme.tip_bg);
            cv.stroke_rr(r, 8.0, 1.0, WHITE.alpha(0.1));
            cv.text(r, &it.tip, st.center(), theme.tip_text);
        });
    });
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Loupe
// ---------------------------------------------------------------------------------------------------------------------------------------

pub(crate) const LENS_W: f32 = 147.0;
pub(crate) const LENS_H: f32 = 215.0;

fn draw_lens(cv: &Canvas, _theme: &Theme, l: &cap::loupe::LoupeView, pos: (f32, f32)) {
    let r = Rect::new(pos.0, pos.1, LENS_W, LENS_H);
    cv.shadow_rr(r, 16.0, 30.0, 10.0, BLACK.alpha(0.55));
    cv.fill_rr(r, 16.0, Color::rgba(10.0 / 255.0, 10.0 / 255.0, 10.0 / 255.0, 0.94));
    cv.stroke_rr(r, 16.0, 1.0, WHITE.alpha(0.14));
    // the 15 x 15 magnified grid, 135 px, rounded 10
    let g = Rect::new(r.x + 7.0, r.y + 7.0, 135.0, 135.0);
    let cell = cap::loupe::CELL as f32;
    cv.with_clip_rr(g, 10.0, |cv| {
        for c in &l.cells {
            let col = Color::rgb8(c.rgb[0], c.rgb[1], c.rgb[2]);
            let (x, y) = (g.x + c.i as f32 * cell, g.y + c.j as f32 * cell);
            cv.fill_rect(Rect::new(x, y, cell.ceil(), cell.ceil()), col);
        }
        for k in 1..cap::loupe::GRID {
            let o = k as f32 * cell;
            cv.fill_rect(Rect::new(g.x + o - 0.5, g.y, 1.0, g.h), BLACK.alpha(0.16));
            cv.fill_rect(Rect::new(g.x, g.y + o - 0.5, g.w, 1.0), BLACK.alpha(0.16));
        }
        let (bx, by, bw, bh) = l.centre_box;
        let cb = Rect::new(g.x + bx as f32, g.y + by as f32, bw as f32, bh as f32);
        cv.stroke_rr(cb.inset(-1.5), 0.0, 3.0, BLACK);
        cv.stroke_rr(cb.inset(-0.75), 0.0, 1.5, WHITE);
    });
    // colour chip + texts
    let iy = g.bottom() + 6.0;
    let chip = Rect::new(g.x, iy + 11.0, 22.0, 22.0);
    let sw = tools::parse_color(&l.swatch);
    cv.fill_rr(chip, 7.0, sw);
    cv.stroke_rr(chip.inset(-1.0), 8.0, 1.0, WHITE.alpha(0.25));
    let tx = g.x + 30.0;
    let tw = g.right() - tx;
    cv.text(Rect::new(tx, iy, tw, 16.0), &l.hex, Style::new(13.0, 600).mono(), WHITE);
    cv.text(Rect::new(tx, iy + 16.0, tw, 14.0), &l.rgb, Style::new(11.0, 500).mono(), Color::rgb8(0x9a, 0x9a, 0x9a));
    cv.text(Rect::new(tx, iy + 30.0, tw, 14.0), &l.hsl, Style::new(11.0, 500).mono(), Color::rgb8(0x9a, 0x9a, 0x9a));
    cv.text(Rect::new(g.x, iy + 46.0, g.w, 14.0), &l.keys, Style::new(11.0, 500).mono(), Color::rgb8(0x7a, 0x7a, 0x7a));
}
