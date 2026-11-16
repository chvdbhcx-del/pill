//! Keyboard-first use and UI Automation, one model: a window's view lists its controls (`View::controls`), and everything else is derived
//! from that list - the focus order and the arrow-key moves here, the UIA children in `host/uia.rs`, the focus ring in `ring.rs`. The
//! lists are built from the very rectangles the view hit-tests with, so a new control that is hit-testable shows up for the keyboard and
//! for screen readers without a second table.
//!
//! Everything in this file is pure (no window, no GPU): coordinates are the view's own logical client px.
use glasskit::geom::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Button,
    /// a tab strip entry (selection item: `Value::Selected`)
    Tab,
    Slider,
    ListItem,
    CheckBox,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    None,
    /// a switch or a pinned state: on / off
    Toggled(bool),
    /// a tab or a list item: the current one
    Selected(bool),
    /// a slider (UIA range value); `step` is what an arrow key moves it by
    Range { min: f64, max: f64, cur: f64, step: f64 },
}

/// What a keyboard or a screen reader asks a control to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Act {
    /// press it (UIA Invoke)
    Invoke,
    /// flip it (UIA Toggle)
    Toggle,
    /// make it the current one (UIA SelectionItem.Select)
    Select,
    /// a slider to this value (UIA RangeValue.SetValue)
    Set(f64),
    /// a slider by this many steps (arrow keys)
    Step(i32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Control {
    /// stable while the control exists (the window's own numbering; see the id ranges of each view)
    pub id: u32,
    pub name: String,
    pub role: Role,
    /// where it is, in the window's logical client px
    pub rect: Rect,
    pub value: Value,
    pub enabled: bool,
}

impl Control {
    pub fn new(id: u32, name: impl Into<String>, role: Role, rect: Rect) -> Control {
        Control { id, name: name.into(), role, rect, value: Value::None, enabled: true }
    }

    pub fn with(mut self, v: Value) -> Control {
        self.value = v;
        self
    }

    /// What Enter / Space does to it.
    pub fn default_act(&self) -> Act {
        match (self.role, self.value) {
            (Role::Tab | Role::ListItem, _) => Act::Select,
            (Role::CheckBox, _) | (_, Value::Toggled(_)) => Act::Toggle,
            _ => Act::Invoke,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

/// The ids in reading order: rows from the top (controls whose centres are within half a row of each other share a row), each row left to right.
pub fn reading_order(cs: &[Control]) -> Vec<u32> {
    let mut v: Vec<&Control> = cs.iter().filter(|c| c.enabled && c.rect.w > 0.0 && c.rect.h > 0.0).collect();
    v.sort_by(|a, b| a.rect.cy().total_cmp(&b.rect.cy()));
    let mut rows: Vec<Vec<&Control>> = Vec::new();
    for c in v {
        match rows.last_mut() {
            Some(r) if (c.rect.cy() - r[0].rect.cy()).abs() <= c.rect.h.min(r[0].rect.h) / 2.0 => r.push(c),
            _ => rows.push(vec![c]),
        }
    }
    rows.into_iter()
        .flat_map(|mut r| {
            r.sort_by(|a, b| a.rect.cx().total_cmp(&b.rect.cx()));
            r.into_iter().map(|c| c.id)
        })
        .collect()
}

/// Tab / Shift+Tab: the next (or previous) control in reading order, wrapping. Without a current one: the first (the last going back).
pub fn tab_step(cs: &[Control], cur: Option<u32>, back: bool) -> Option<u32> {
    let order = reading_order(cs);
    if order.is_empty() {
        return None;
    }
    let n = order.len();
    Some(match cur.and_then(|c| order.iter().position(|&x| x == c)) {
        Some(i) => order[if back { (i + n - 1) % n } else { (i + 1) % n }],
        None if back => order[n - 1],
        None => order[0],
    })
}

/// An arrow key: the control lying in that direction from the current one. A control that lies clear of the current one along the arrow
/// (its rectangle starts where the current one ends) and overlaps it across the arrow (the same row for Left / Right, the same column for
/// Up / Down) wins, the nearest of those; with none, the nearest of the rest by the distance along the arrow plus twice the sideways offset.
/// Without a current control: the first in reading order. `None` = nothing lies that way (the focus stays).
pub fn move_dir(cs: &[Control], cur: Option<u32>, dir: Dir) -> Option<u32> {
    const TOL: f32 = 2.0;
    let Some(from) = cur.and_then(|c| cs.iter().find(|x| x.id == c)) else { return reading_order(cs).first().copied() };
    let (f, fc) = (from.rect, from.rect.center());
    let mut aligned: Option<(f32, u32)> = None;
    let mut loose: Option<(f32, u32)> = None;
    for c in cs.iter().filter(|c| c.enabled && c.id != from.id && c.rect.w > 0.0 && c.rect.h > 0.0) {
        let (r, cc) = (c.rect, c.rect.center());
        let (clear, overlap, along, side) = match dir {
            Dir::Right => (r.x >= f.right() - TOL, r.y < f.bottom() && r.bottom() > f.y, cc.0 - fc.0, (cc.1 - fc.1).abs()),
            Dir::Left => (r.right() <= f.x + TOL, r.y < f.bottom() && r.bottom() > f.y, fc.0 - cc.0, (cc.1 - fc.1).abs()),
            Dir::Down => (r.y >= f.bottom() - TOL, r.x < f.right() && r.right() > f.x, cc.1 - fc.1, (cc.0 - fc.0).abs()),
            Dir::Up => (r.bottom() <= f.y + TOL, r.x < f.right() && r.right() > f.x, fc.1 - cc.1, (cc.0 - fc.0).abs()),
        };
        if !clear {
            continue;
        }
        let (slot, d) = if overlap { (&mut aligned, along) } else { (&mut loose, along + 2.0 * side) };
        if slot.map_or(true, |(b, _)| d < b) {
            *slot = Some((d, c.id));
        }
    }
    aligned.or(loose).map(|(_, id)| id)
}

/// Alt+N: the id of the N-th (1-based) tab of the strip, left to right.
pub fn nth_tab(cs: &[Control], n: usize) -> Option<u32> {
    let mut tabs: Vec<&Control> = cs.iter().filter(|c| c.role == Role::Tab && c.enabled && c.rect.w > 0.0).collect();
    tabs.sort_by(|a, b| a.rect.x.total_cmp(&b.rect.x));
    n.checked_sub(1).and_then(|i| tabs.get(i)).map(|c| c.id)
}

/// The tab next to `cur` (left / right, no wrap); without a current tab, the selected one's neighbour, or the first.
pub fn neighbour_tab(cs: &[Control], cur: Option<u32>, right: bool) -> Option<u32> {
    let mut tabs: Vec<&Control> = cs.iter().filter(|c| c.role == Role::Tab && c.enabled && c.rect.w > 0.0).collect();
    tabs.sort_by(|a, b| a.rect.x.total_cmp(&b.rect.x));
    let at = cur
        .and_then(|c| tabs.iter().position(|t| t.id == c))
        .or_else(|| tabs.iter().position(|t| t.value == Value::Selected(true)));
    match at {
        Some(i) if right => tabs.get(i + 1),
        Some(i) => i.checked_sub(1).and_then(|j| tabs.get(j)),
        None => tabs.first(),
    }
    .map(|c| c.id)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavKey {
    Tab { back: bool },
    Arrow(Dir),
    Enter,
    Space,
    Esc,
    Home,
    End,
    /// Alt + a digit 1..9
    AltDigit(u8),
}

/// What the view does with a key in keyboard mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NavOut {
    /// move the ring (and the accessibility focus) to this control
    Focus(u32),
    /// act on this control (the ring follows it)
    Act(u32, Act),
    /// leave keyboard mode: hide the ring, give the focus back
    Close,
    /// not ours: let the key through to the window's own handling
    Pass,
}

/// The one routing rule of keyboard mode.
///  * Tab / Shift+Tab walk the reading order; Home / End jump to its ends;
///  * Left / Right on a tab (or with nothing focused) switch to the neighbouring tab, on a slider they step it, elsewhere they move spatially;
///  * Up / Down always move spatially;
///  * Enter / Space press the focused control (a slider ignores them); Esc leaves;
///  * Alt+1..9 jump to the N-th tab.
pub fn route(cs: &[Control], cur: Option<u32>, key: NavKey) -> NavOut {
    let focus = |id: Option<u32>| id.map_or(NavOut::Pass, NavOut::Focus);
    let cur_c = cur.and_then(|c| cs.iter().find(|x| x.id == c));
    match key {
        NavKey::Esc => NavOut::Close,
        NavKey::Tab { back } => focus(tab_step(cs, cur, back)),
        NavKey::Home => focus(reading_order(cs).first().copied()),
        NavKey::End => focus(reading_order(cs).last().copied()),
        NavKey::AltDigit(n) => nth_tab(cs, n as usize).map_or(NavOut::Pass, |id| NavOut::Act(id, Act::Select)),
        NavKey::Enter | NavKey::Space => match cur_c {
            Some(c) if c.role == Role::Slider => NavOut::Pass,
            Some(c) => NavOut::Act(c.id, c.default_act()),
            None => NavOut::Pass,
        },
        NavKey::Arrow(d @ (Dir::Left | Dir::Right)) => {
            let right = d == Dir::Right;
            match cur_c {
                Some(c) if c.role == Role::Slider => NavOut::Act(c.id, Act::Step(if right { 1 } else { -1 })),
                Some(c) if c.role != Role::Tab => focus(move_dir(cs, cur, d)),
                _ => neighbour_tab(cs, cur, right).map_or_else(|| focus(move_dir(cs, cur, d)), |id| NavOut::Act(id, Act::Select)),
            }
        }
        NavKey::Arrow(d) => focus(move_dir(cs, cur, d)),
    }
}

/// The key as keyboard mode reads it: from a glasskit key event. `None` = not a navigation key.
pub fn nav_key(code: glasskit::input::KeyCode, m: glasskit::input::Modifiers) -> Option<NavKey> {
    use glasskit::input::KeyCode as K;
    if m.ctrl || m.meta {
        return None;
    }
    if m.alt {
        return match code {
            K::Char(c) => c.to_digit(10).filter(|d| (1..=9).contains(d)).map(|d| NavKey::AltDigit(d as u8)),
            _ => None,
        };
    }
    Some(match code {
        K::Tab => NavKey::Tab { back: m.shift },
        K::Left => NavKey::Arrow(Dir::Left),
        K::Right => NavKey::Arrow(Dir::Right),
        K::Up => NavKey::Arrow(Dir::Up),
        K::Down => NavKey::Arrow(Dir::Down),
        K::Enter => NavKey::Enter,
        K::Space => NavKey::Space,
        K::Escape => NavKey::Esc,
        K::Home => NavKey::Home,
        K::End => NavKey::End,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(id: u32, x: f32, sel: bool) -> Control {
        Control::new(id, format!("tab{id}"), Role::Tab, Rect::new(x, 0.0, 30.0, 24.0)).with(Value::Selected(sel))
    }
    fn btn(id: u32, x: f32, y: f32) -> Control {
        Control::new(id, format!("b{id}"), Role::Button, Rect::new(x, y, 30.0, 24.0))
    }
    fn strip() -> Vec<Control> {
        // listed out of order on purpose: 3 tabs on the first row, two header buttons on it too, a slider and a button below
        vec![
            btn(20, 300.0, 2.0),
            tab(2, 40.0, true),
            tab(1, 0.0, false),
            tab(3, 80.0, false),
            btn(21, 340.0, 0.0),
            Control::new(30, "vol", Role::Slider, Rect::new(0.0, 60.0, 200.0, 20.0)).with(Value::Range { min: 0.0, max: 100.0, cur: 30.0, step: 5.0 }),
            btn(31, 220.0, 60.0),
        ]
    }

    #[test]
    fn reading_order_is_rows_then_columns() {
        assert_eq!(reading_order(&strip()), [1, 2, 3, 20, 21, 30, 31]);
        let mut cs = strip();
        cs[1].enabled = false;
        assert!(!reading_order(&cs).contains(&2), "a disabled control is skipped");
    }

    #[test]
    fn tab_wraps_both_ways() {
        let cs = strip();
        assert_eq!(tab_step(&cs, None, false), Some(1));
        assert_eq!(tab_step(&cs, None, true), Some(31));
        assert_eq!(tab_step(&cs, Some(31), false), Some(1));
        assert_eq!(tab_step(&cs, Some(1), true), Some(31));
        assert_eq!(tab_step(&cs, Some(3), false), Some(20));
        assert_eq!(tab_step(&[], Some(3), false), None);
        assert_eq!(tab_step(&cs, Some(999), false), Some(1), "a control that is gone starts over");
    }

    #[test]
    fn arrows_pick_the_nearest_ahead() {
        let cs = strip();
        assert_eq!(move_dir(&cs, Some(1), Dir::Right), Some(2));
        assert_eq!(move_dir(&cs, Some(3), Dir::Right), Some(20));
        assert_eq!(move_dir(&cs, Some(1), Dir::Left), None, "nothing to the left of the first");
        assert_eq!(move_dir(&cs, Some(1), Dir::Down), Some(30), "straight down: the slider under it, not the far button");
        assert_eq!(move_dir(&cs, Some(30), Dir::Right), Some(31), "same row first");
        assert_eq!(move_dir(&cs, Some(31), Dir::Up), Some(20), "nothing above it in its column: the nearest header button");
        assert_eq!(move_dir(&cs, None, Dir::Down), Some(1));
    }

    #[test]
    fn alt_digits_and_neighbours() {
        let cs = strip();
        assert_eq!(nth_tab(&cs, 1), Some(1));
        assert_eq!(nth_tab(&cs, 3), Some(3));
        assert_eq!(nth_tab(&cs, 4), None);
        assert_eq!(nth_tab(&cs, 0), None);
        assert_eq!(neighbour_tab(&cs, Some(2), true), Some(3));
        assert_eq!(neighbour_tab(&cs, Some(3), true), None, "no wrap");
        assert_eq!(neighbour_tab(&cs, None, false), Some(1), "from the selected tab 2 to its left");
    }

    #[test]
    fn route_rules() {
        let cs = strip();
        assert_eq!(route(&cs, Some(2), NavKey::Arrow(Dir::Right)), NavOut::Act(3, Act::Select), "Right on a tab switches tabs");
        assert_eq!(route(&cs, Some(1), NavKey::Arrow(Dir::Left)), NavOut::Pass, "the first tab has no left neighbour: the key passes");
        assert_eq!(route(&cs, Some(30), NavKey::Arrow(Dir::Right)), NavOut::Act(30, Act::Step(1)));
        assert_eq!(route(&cs, Some(30), NavKey::Arrow(Dir::Left)), NavOut::Act(30, Act::Step(-1)));
        assert_eq!(route(&cs, Some(30), NavKey::Enter), NavOut::Pass);
        assert_eq!(route(&cs, Some(21), NavKey::Enter), NavOut::Act(21, Act::Invoke));
        assert_eq!(route(&cs, Some(1), NavKey::Space), NavOut::Act(1, Act::Select));
        assert_eq!(route(&cs, Some(20), NavKey::Arrow(Dir::Left)), NavOut::Focus(3), "off the buttons back along the strip");
        assert_eq!(route(&cs, Some(1), NavKey::Arrow(Dir::Down)), NavOut::Focus(30));
        assert_eq!(route(&cs, Some(1), NavKey::AltDigit(3)), NavOut::Act(3, Act::Select));
        assert_eq!(route(&cs, Some(1), NavKey::AltDigit(9)), NavOut::Pass);
        assert_eq!(route(&cs, None, NavKey::Enter), NavOut::Pass);
        assert_eq!(route(&cs, None, NavKey::Tab { back: false }), NavOut::Focus(1));
        assert_eq!(route(&cs, Some(3), NavKey::Esc), NavOut::Close);
        assert_eq!(route(&cs, Some(3), NavKey::End), NavOut::Focus(31));
        let toggle = vec![Control::new(5, "pin", Role::Button, Rect::new(0.0, 0.0, 20.0, 20.0)).with(Value::Toggled(false))];
        assert_eq!(route(&toggle, Some(5), NavKey::Enter), NavOut::Act(5, Act::Toggle));
    }

    #[test]
    fn keys_map_and_modifiers_gate() {
        use glasskit::input::{KeyCode as K, Modifiers as M};
        assert_eq!(nav_key(K::Tab, M::SHIFT), Some(NavKey::Tab { back: true }));
        assert_eq!(nav_key(K::Char('3'), M::ALT), Some(NavKey::AltDigit(3)));
        assert_eq!(nav_key(K::Char('0'), M::ALT), None);
        assert_eq!(nav_key(K::Char('a'), M::NONE), None, "typing is not navigation");
        assert_eq!(nav_key(K::Left, M::CTRL), None, "Ctrl+arrows stay with the app");
        assert_eq!(nav_key(K::Escape, M::NONE), Some(NavKey::Esc));
    }
}
