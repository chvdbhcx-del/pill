//! The chips beside the time on the folded island: the keyboard layout ("RU", always there when `live.keyboardLayoutAlways` is on) on the
//! left, the virtual desktop (`live.virtualDesktop`: number or name) on the right. The backend announces the values (`kbd:now`,
//! `vdesk:now`, and the answers of `kbd_state` / `vdesk_state` at start); this keeps the last ones and turns them into the texts the
//! shell's clock row shows. Nothing here polls or draws.
use serde_json::Value;

type Pair = (Vec<String>, Vec<String>);

#[derive(Default)]
pub struct Chips {
    layout: Option<String>,
    /// (index, total, name) of the current desktop
    desktop: Option<(usize, usize, Option<String>)>,
    sent: Pair,
}

/// Longest name shown on the chip (characters); longer ones end in an ellipsis.
const NAME_MAX: usize = 12;

fn cut(s: &str) -> String {
    if s.chars().count() <= NAME_MAX {
        s.to_string()
    } else {
        s.chars().take(NAME_MAX - 1).collect::<String>() + "…"
    }
}

impl Chips {
    /// A backend value (`kbd:now`, `vdesk:now`). True when it was one of ours.
    pub fn feed(&mut self, name: &str, p: &Value) -> bool {
        match name {
            "kbd:now" => {
                self.layout = p.get("lang").and_then(Value::as_str).map(|s| s.chars().take(3).collect::<String>()).filter(|s| !s.is_empty());
                true
            }
            "vdesk:now" => {
                let n = |k: &str| p.get(k).and_then(Value::as_u64).map(|v| v as usize);
                self.desktop = n("index").zip(n("total")).map(|(i, t)| (i.max(1), t.max(1), p.get("name").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(String::from)));
                true
            }
            _ => false,
        }
    }

    pub fn has_layout(&self) -> bool {
        self.layout.is_some()
    }

    pub fn has_desktop(&self) -> bool {
        self.desktop.is_some()
    }

    /// The texts for the left and the right column under these settings.
    pub fn texts(&self, settings: &Value, ru: bool) -> Pair {
        let left = match (&self.layout, settings.get("live.keyboardLayoutAlways")) {
            (Some(l), Some(Value::Bool(true))) => vec![l.clone()],
            _ => Vec::new(),
        };
        let right = match (&self.desktop, settings.get("live.virtualDesktop").and_then(Value::as_str)) {
            (Some((i, t, _)), Some("number")) => vec![if *t > 1 { format!("{i}/{t}") } else { i.to_string() }],
            (Some((_, _, Some(name))), Some("name")) => vec![cut(name)],
            (Some((i, _, None)), Some("name")) => vec![format!("{} {i}", if ru { "Рабочий стол" } else { "Desktop" })],
            _ => Vec::new(),
        };
        (left, right)
    }

    /// The texts when they differ from what the shell was last told.
    pub fn update(&mut self, settings: &Value, ru: bool) -> Option<Pair> {
        let now = self.texts(settings, ru);
        (now != self.sent).then(|| {
            self.sent = now.clone();
            now
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_layout_chip_needs_the_switch_and_a_value() {
        let mut c = Chips::default();
        let on = json!({"live.keyboardLayoutAlways": true});
        assert_eq!(c.update(&on, false), None, "no value yet");
        assert!(c.feed("kbd:now", &json!({"lang": "RU", "name": "Russian"})));
        assert_eq!(c.update(&on, false), Some((vec!["RU".into()], vec![])));
        assert_eq!(c.update(&on, false), None, "unchanged");
        assert_eq!(c.update(&json!({"live.keyboardLayoutAlways": false}), false), Some((vec![], vec![])), "switched off: the chip goes");
        assert_eq!(c.texts(&json!({}), false).0.len(), 0, "off by default");
        c.feed("kbd:now", &json!({"lang": "ENGL"}));
        assert_eq!(c.texts(&on, false).0, ["ENG"], "three units at most");
    }

    #[test]
    fn the_desktop_chip_shows_number_or_name() {
        let mut c = Chips::default();
        c.feed("vdesk:now", &json!({"index": 2, "total": 3, "name": null}));
        let (num, name) = (json!({"live.virtualDesktop": "number"}), json!({"live.virtualDesktop": "name"}));
        assert_eq!(c.texts(&num, false).1, ["2/3"]);
        assert_eq!(c.texts(&name, false).1, ["Desktop 2"]);
        assert_eq!(c.texts(&name, true).1, ["Рабочий стол 2"]);
        assert!(c.texts(&json!({"live.virtualDesktop": "off"}), false).1.is_empty());
        assert!(c.texts(&json!({}), false).1.is_empty());
        c.feed("vdesk:now", &json!({"index": 1, "total": 1, "name": "Work on the long project"}));
        assert_eq!(c.texts(&num, false).1, ["1"]);
        assert_eq!(c.texts(&name, false).1, ["Work on the…"]);
        assert!(!c.feed("pill:show", &json!({})));
    }
}
