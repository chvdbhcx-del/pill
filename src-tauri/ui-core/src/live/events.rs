//! Live reactions of the collapsed pill (port of live-events.js; Rust side watch.rs, clipboard.rs).
//!
//! * `kbd:layout {lang, name}` -> a "RU" chip for 1.4 s (priority 90). `pill:hide` / `pill:show` pause / resume the
//!   foreground-window poller in watch.rs.
//! * `clip:added {kind, fileCount, ...}` -> "Copied · text/image/N files" for 1.3 s (priority 80) and a liquid-edge pulse.
//!
//! PRIVACY: the copied text is never read here, only `kind` and `fileCount`.
//! Both rows belong to the quiet-hours sources (`kbd`, `copy`).

use std::any::Any;

use serde_json::Value;

use super::env::Env;
use super::hub::Provider;
use super::model::*;
use super::util::*;

pub const KBD_MS: u32 = 1400;
pub const COPY_MS: u32 = 1300;

/// The liquid-edge pulse, muted during quiet hours (`edge()` of the JS).
pub fn edge(env: &dyn Env, strength: f32) -> Vec<LiveCmd> {
    if env.quiet() { Vec::new() } else { vec![LiveCmd::EdgePulse(strength)] }
}

#[derive(Debug, Default)]
pub struct KbdLive;

impl Provider for KbdLive {
    fn name(&self) -> &'static str {
        "kbd"
    }
    fn events(&self) -> &'static [&'static str] {
        &["kbd:layout", "pill:hide", "pill:show"]
    }
    fn on_event(&mut self, env: &dyn Env, name: &str, p: &Value, _now: i64) -> Vec<LiveCmd> {
        match name {
            "kbd:layout" => {
                // Settings > Live events > Keyboard layout (watch.rs also stops polling)
                if p.is_null() || !truthy(p.get("lang")) || !env.collapsed() || !env.live_on("keyboardLayout") {
                    return Vec::new();
                }
                let code = u16_slice(&text(p.get("lang")), 0, 3);
                let row = RowSpec::new("kbd", 90)
                    .ttl(KBD_MS)
                    .tip(text(p.get("name")))
                    .anim(Anim::Pop, Anim::None, Anim::Fade)
                    .with(Elem::icon_c(IconName::Keyboard, 17.0, ColorRole::Dim))
                    .with(Elem::text(code, TextStyle::Code));
                vec![LiveCmd::set(row)]
            }
            // The poller in watch.rs slows down while the pill is hidden.
            "pill:hide" => vec![LiveCmd::Run(Action::invoke0("watch_pause"))],
            "pill:show" => vec![LiveCmd::Run(Action::invoke0("watch_resume"))],
            _ => Vec::new(),
        }
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[derive(Debug, Default)]
pub struct CopyLive;

fn kind_label(env: &dyn Env, p: &Value) -> String {
    match p.get("kind").and_then(Value::as_str) {
        Some("image") => env.tr("Изображение", "Image"),
        Some("files") => files_plural(i64::from(int32(p.get("fileCount")).max(1)), env.is_ru()),
        _ => env.tr("Текст", "Text"),
    }
}

impl Provider for CopyLive {
    fn name(&self) -> &'static str {
        "copy"
    }
    fn events(&self) -> &'static [&'static str] {
        &["clip:added"]
    }
    fn on_event(&mut self, env: &dyn Env, _name: &str, p: &Value, _now: i64) -> Vec<LiveCmd> {
        // nothing stored / Settings > Live events > Reaction to copying is off
        if p.is_null() || !truthy(p.get("kind")) || !env.live_on("copy") {
            return Vec::new();
        }
        let mut out = edge(env, 0.8);
        if !env.collapsed() {
            return out;
        }
        let row = RowSpec::new("copy", 80)
            .ttl(COPY_MS)
            .anim(Anim::Pop, Anim::None, Anim::Fade)
            .with(Elem::icon_c(IconName::Clipboard, 16.0, ColorRole::Accent))
            .with(Elem::text(env.tr("Скопировано", "Copied"), TextStyle::Label))
            .with(Elem::text(kind_label(env, p), TextStyle::Dim));
        out.push(LiveCmd::set(row));
        out
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::live::env::{MemEnv, PillState};

    const T0: i64 = 1_700_000_000_000;

    fn row(cmds: &[LiveCmd]) -> &RowSpec {
        cmds.iter().find_map(|c| if let LiveCmd::SetRow(r) = c { Some(r) } else { None }).expect("row")
    }
    fn txt(r: &RowSpec, i: usize) -> String {
        match &r.elements[i] {
            Elem::Text { text, .. } => text.clone(),
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn kbd_chip() {
        let mut k = KbdLive;
        let env = MemEnv::default();
        let cmds = k.on_event(&env, "kbd:layout", &json!({"lang": "RU", "name": "Russian"}), T0);
        let r = row(&cmds);
        assert_eq!((r.id.as_str(), r.priority, r.ttl_ms), ("kbd", 90, Some(1400)));
        assert_eq!(r.tip.as_deref(), Some("Russian"));
        assert_eq!(txt(r, 1), "RU");
        assert!(matches!(r.elements[1], Elem::Text { style: TextStyle::Code, .. }));
    }

    #[test]
    fn kbd_code_is_cut_to_three_units() {
        let mut k = KbdLive;
        let env = MemEnv::default();
        assert_eq!(txt(row(&k.on_event(&env, "kbd:layout", &json!({"lang": "0419x"}), T0)), 1), "041");
        assert_eq!(txt(row(&k.on_event(&env, "kbd:layout", &json!({"lang": "рус"}), T0)), 1), "рус");
        assert_eq!(txt(row(&k.on_event(&env, "kbd:layout", &json!({"lang": 7}), T0)), 1), "7");
        // no name: empty tooltip, not "undefined"
        assert_eq!(row(&k.on_event(&env, "kbd:layout", &json!({"lang": "EN"}), T0)).tip.as_deref(), Some(""));
    }

    #[test]
    fn kbd_gates() {
        let mut k = KbdLive;
        let mut env = MemEnv::default();
        assert!(k.on_event(&env, "kbd:layout", &Value::Null, T0).is_empty());
        assert!(k.on_event(&env, "kbd:layout", &json!({}), T0).is_empty());
        assert!(k.on_event(&env, "kbd:layout", &json!({"lang": ""}), T0).is_empty());
        env.state = PillState::Expanded;
        assert!(k.on_event(&env, "kbd:layout", &json!({"lang": "EN"}), T0).is_empty());
        env.state = PillState::Collapsed;
        env.set("live.keyboardLayout", json!(false));
        assert!(k.on_event(&env, "kbd:layout", &json!({"lang": "EN"}), T0).is_empty());
    }

    #[test]
    fn hide_show_pause_the_watcher() {
        let mut k = KbdLive;
        let env = MemEnv::default();
        assert_eq!(k.on_event(&env, "pill:hide", &Value::Null, T0), vec![LiveCmd::Run(Action::invoke0("watch_pause"))]);
        assert_eq!(k.on_event(&env, "pill:show", &Value::Null, T0), vec![LiveCmd::Run(Action::invoke0("watch_resume"))]);
    }

    #[test]
    fn copy_reaction_text_image_files() {
        let mut c = CopyLive;
        let env = MemEnv::default();
        let cmds = c.on_event(&env, "clip:added", &json!({"id": 1, "kind": "text"}), T0);
        assert_eq!(cmds[0], LiveCmd::EdgePulse(0.8));
        let r = row(&cmds);
        assert_eq!((r.id.as_str(), r.priority, r.ttl_ms), ("copy", 80, Some(1300)));
        assert_eq!((txt(r, 1), txt(r, 2)), ("Скопировано".to_string(), "Текст".to_string()));
        let cmds = c.on_event(&MemEnv::en(), "clip:added", &json!({"kind": "image"}), T0);
        assert_eq!((txt(row(&cmds), 1), txt(row(&cmds), 2)), ("Copied".to_string(), "Image".to_string()));
        let cmds = c.on_event(&env, "clip:added", &json!({"kind": "files", "fileCount": 3}), T0);
        assert_eq!(txt(row(&cmds), 2), "3 файла");
        let cmds = c.on_event(&MemEnv::en(), "clip:added", &json!({"kind": "files", "fileCount": 3}), T0);
        assert_eq!(txt(row(&cmds), 2), "3 files");
    }

    #[test]
    fn copy_file_count_edge_cases() {
        let mut c = CopyLive;
        let env = MemEnv::default();
        // 0 or missing count still says "1 файл" (Math.max(1, ...))
        assert_eq!(txt(row(&c.on_event(&env, "clip:added", &json!({"kind": "files", "fileCount": 0}), T0)), 2), "1 файл");
        assert_eq!(txt(row(&c.on_event(&env, "clip:added", &json!({"kind": "files"}), T0)), 2), "1 файл");
        assert_eq!(txt(row(&c.on_event(&env, "clip:added", &json!({"kind": "files", "fileCount": 11}), T0)), 2), "11 файлов");
        assert_eq!(txt(row(&c.on_event(&env, "clip:added", &json!({"kind": "files", "fileCount": 21}), T0)), 2), "21 файл");
        // an unknown kind is plain text, like the JS fall-through
        assert_eq!(txt(row(&c.on_event(&env, "clip:added", &json!({"kind": "rtf"}), T0)), 2), "Текст");
    }

    #[test]
    fn copy_pulses_even_when_expanded_but_shows_no_chip() {
        let mut c = CopyLive;
        let mut env = MemEnv::default();
        env.state = PillState::Expanded;
        assert_eq!(c.on_event(&env, "clip:added", &json!({"kind": "text"}), T0), vec![LiveCmd::EdgePulse(0.8)]);
    }

    #[test]
    fn copy_gates_and_quiet_pulse() {
        let mut c = CopyLive;
        let mut env = MemEnv::default();
        assert!(c.on_event(&env, "clip:added", &Value::Null, T0).is_empty());
        assert!(c.on_event(&env, "clip:added", &json!({}), T0).is_empty());
        env.quiet = true;
        let cmds = c.on_event(&env, "clip:added", &json!({"kind": "text"}), T0);
        assert!(!cmds.contains(&LiveCmd::EdgePulse(0.8)), "no pulse in quiet hours");
        assert_eq!(cmds.len(), 1, "the chip is left to the quiet filter of the hub");
        env.quiet = false;
        env.set("live.copy", json!(false));
        assert!(c.on_event(&env, "clip:added", &json!({"kind": "text"}), T0).is_empty());
    }
}
