//! Home modules: which parts the Home tab shows (Settings > Modules > "What is on Home") and where each one goes.
//!
//! Twelve parts: the player, "recently copied" and the volume row on the left, the eight tiles, the launcher strip ("shortcuts") at the
//! bottom. A part is on when its settings key says so; a missing key means the part's default. [`plan`] turns the switches into
//! rectangles in the 648 px wide tab box, and [`content_h`] is the height that layout needs, so the island can size the panel to it.
//! With everything on the plan is exactly the classic grid (player 412x154, volume row 412x38, 2x4 tiles 110x44, strip 648x52).
//!
//! The same plan feeds the animated relayout of the Home tab and the little schematic in the settings row, so they cannot disagree.
use serde_json::Value;

/// Width of the tab box.
pub const W: f32 = 648.0;
pub const GAP: f32 = 8.0;
pub const PLAYER_W: f32 = 412.0;
pub const PLAYER_H: f32 = 154.0;
pub const VOLUME_H: f32 = 38.0;
pub const RECENT_H: f32 = 62.0;
pub const STRIP_H: f32 = 52.0;
/// The panel chrome around a tab body (60 px header + 11 px bottom): panel height = content height + this.
pub const CHROME_H: f32 = 71.0;
/// Width of the tile column next to the left stack.
pub const SIDE_W: f32 = W - PLAYER_W - GAP;
/// A tile never gets lower than this in the side column, nor higher than [`TILE_MAX_H`].
pub const TILE_MIN_H: f32 = 44.0;
pub const TILE_MAX_H: f32 = 72.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Player,
    Recent,
    Volume,
    /// Index into [`crate::panel::a::home::TILES`].
    Tile(usize),
    Shortcuts,
}

/// One switchable part. `key` is the exact settings.json key.
#[derive(Clone, Copy, Debug)]
pub struct Part {
    pub kind: Kind,
    pub key: &'static str,
    pub ru: &'static str,
    pub en: &'static str,
    pub default: bool,
}

pub const N: usize = 12;
/// Index of the shortcuts strip in [`PARTS`] and in a [`Plan`].
pub const STRIP: usize = 11;

/// Screen order of the settings chips: left column, tiles, bottom strip.
pub const PARTS: [Part; N] = [
    Part { kind: Kind::Player, key: "module.media", ru: "Плеер", en: "Player", default: true },
    Part { kind: Kind::Recent, key: "home.recent", ru: "Недавно скопировано", en: "Recently copied", default: false },
    Part { kind: Kind::Volume, key: "home.volume", ru: "Громкость", en: "Volume", default: true },
    Part { kind: Kind::Tile(0), key: "home.tile.area", ru: "Область", en: "Area", default: true },
    Part { kind: Kind::Tile(1), key: "home.tile.screen", ru: "Весь экран", en: "Full screen", default: true },
    Part { kind: Kind::Tile(2), key: "home.tile.ocr", ru: "Текст с экрана", en: "Text from screen", default: true },
    Part { kind: Kind::Tile(3), key: "home.tile.qr", ru: "QR-код", en: "QR code", default: true },
    Part { kind: Kind::Tile(4), key: "home.tile.picker", ru: "Пипетка", en: "Picker", default: true },
    Part { kind: Kind::Tile(5), key: "home.tile.plain", ru: "Чистый текст", en: "Clean text", default: true },
    Part { kind: Kind::Tile(6), key: "home.tile.record", ru: "Запись экрана", en: "Screen recording", default: true },
    Part { kind: Kind::Tile(7), key: "home.tile.command", ru: "Команда", en: "Command", default: true },
    Part { kind: Kind::Shortcuts, key: "module.launcher", ru: "Ярлыки", en: "Shortcuts", default: true },
];

/// Part index of tile `i` (0..8).
pub const fn tile_part(i: usize) -> usize {
    3 + i
}

/// The keys this module owns beyond `module.media` / `module.launcher` (which the Modules page has rows for): the settings window
/// accepts writes to them although no row has them.
pub fn is_home_key(key: &str) -> bool {
    key == "home.recent" || key == "home.volume" || key.starts_with("home.tile.") && PARTS.iter().any(|p| p.key == key)
}

/// Which parts are on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct On(pub [bool; N]);

impl Default for On {
    fn default() -> On {
        On(std::array::from_fn(|i| PARTS[i].default))
    }
}

impl On {
    /// `get(key)`: the stored value, `None` = never written (the part's default applies). Only a bool counts.
    pub fn from_get(get: impl Fn(&str) -> Option<Value>) -> On {
        On(std::array::from_fn(|i| get(PARTS[i].key).and_then(|v| v.as_bool()).unwrap_or(PARTS[i].default)))
    }
    pub fn tiles(&self) -> usize {
        (0..8).filter(|i| self.0[tile_part(*i)]).count()
    }
    pub fn any(&self) -> bool {
        self.0.iter().any(|b| *b)
    }
}

/// A rectangle in tab-local px.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct R {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl R {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> R {
        R { x, y, w, h }
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
}

/// Where every part goes (`None` = off) and how tall the whole tab is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    pub rects: [Option<R>; N],
    pub h: f32,
}

/// Tile rectangles for `n` tiles in `region`. Up to two tiles stack in one column, more fill two columns and a lone last tile takes the
/// whole row. In a full-width region (nothing on the left) the tiles are 156x56 in rows of four, each row centred.
fn tile_rects(n: usize, region: R, wide: bool) -> Vec<R> {
    if n == 0 {
        return vec![];
    }
    if wide {
        let (tw, th) = ((W - 3.0 * GAP) / 4.0, 56.0);
        return (0..n)
            .map(|i| {
                let (row, col) = (i / 4, i % 4);
                let in_row = (n - row * 4).min(4);
                let x0 = region.x + (region.w - (in_row as f32 * tw + (in_row - 1) as f32 * GAP)) / 2.0;
                R::new(x0 + col as f32 * (tw + GAP), region.y + row as f32 * (th + GAP), tw, th)
            })
            .collect();
    }
    let cols = if n <= 2 { 1 } else { 2 };
    let rows = n.div_ceil(cols);
    let h = ((region.h - (rows - 1) as f32 * GAP) / rows as f32).clamp(TILE_MIN_H, TILE_MAX_H);
    let total = rows as f32 * h + (rows - 1) as f32 * GAP;
    let y0 = region.y + (region.h - total) / 2.0;
    let cw = (region.w - (cols - 1) as f32 * GAP) / cols as f32;
    (0..n)
        .map(|i| {
            let (row, col) = (i / cols, i % cols);
            let lone = cols == 2 && i == n - 1 && n % 2 == 1;
            R::new(region.x + if lone { 0.0 } else { col as f32 * (cw + GAP) }, y0 + row as f32 * (h + GAP), if lone { region.w } else { cw }, h)
        })
        .collect()
}

/// Layout of the tab for the switches `on`.
pub fn plan(on: &On) -> Plan {
    let mut rects = [None; N];
    let n = on.tiles();
    let left: Vec<(usize, f32)> = [(0usize, PLAYER_H), (1, RECENT_H), (2, VOLUME_H)].into_iter().filter(|(i, _)| on.0[*i]).collect();
    let left_h = if left.is_empty() { 0.0 } else { left.iter().map(|(_, h)| h).sum::<f32>() + (left.len() - 1) as f32 * GAP };
    let cols = if n <= 2 { 1 } else { 2 };
    let side_need = if n == 0 || left.is_empty() { 0.0 } else { n.div_ceil(cols) as f32 * TILE_MIN_H + (n.div_ceil(cols) - 1) as f32 * GAP };
    let wide_h = if n > 0 && left.is_empty() { n.div_ceil(4) as f32 * 56.0 + (n.div_ceil(4) - 1) as f32 * GAP } else { 0.0 };
    let body = left_h.max(side_need).max(wide_h);
    // the left stack: next to the tiles it hugs the left edge, alone it is centred
    let lx = if n > 0 { 0.0 } else { (W - PLAYER_W) / 2.0 };
    let mut y = (body - left_h) / 2.0;
    for (i, h) in &left {
        rects[*i] = Some(R::new(lx, y, PLAYER_W, *h));
        y += h + GAP;
    }
    let region = if left.is_empty() { R::new(0.0, 0.0, W, body) } else { R::new(PLAYER_W + GAP, 0.0, SIDE_W, body) };
    let tiles: Vec<usize> = (0..8).filter(|i| on.0[tile_part(*i)]).collect();
    for (k, r) in tile_rects(n, region, left.is_empty()).into_iter().enumerate() {
        rects[tile_part(tiles[k])] = Some(r);
    }
    let mut h = body;
    if on.0[STRIP] {
        if body > 0.0 {
            h += GAP;
        }
        rects[STRIP] = Some(R::new(0.0, h, W, STRIP_H));
        h += STRIP_H;
    }
    Plan { rects, h }
}

/// Height of the Home tab body (content + panel chrome) for the settings `get`, in css px at 100 %. Everything on = 331, the classic height.
pub fn body_h(get: impl Fn(&str) -> Option<Value>) -> f64 {
    (plan(&On::from_get(get)).h + CHROME_H) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(keys: &[&str]) -> On {
        On(std::array::from_fn(|i| keys.contains(&PARTS[i].key)))
    }

    #[test]
    fn everything_on_is_the_classic_grid() {
        let p = plan(&On::default());
        assert_eq!(p.h, 260.0);
        assert_eq!(p.rects[0], Some(R::new(0.0, 0.0, 412.0, 154.0)));
        assert_eq!(p.rects[2], Some(R::new(0.0, 162.0, 412.0, 38.0)));
        assert_eq!(p.rects[STRIP], Some(R::new(0.0, 208.0, 648.0, 52.0)));
        assert_eq!(p.rects[1], None, "recently copied is opt-in");
        for i in 0..8 {
            let t = p.rects[tile_part(i)].unwrap();
            assert_eq!((t.x, t.y, t.w, t.h), (420.0 + (i % 2) as f32 * 118.0, (i / 2) as f32 * 52.0, 110.0, 44.0), "tile {i}");
        }
        assert_eq!(body_h(|_| None), 331.0);
    }

    #[test]
    fn a_missing_key_is_the_default_and_only_a_bool_counts() {
        let o = On::from_get(|k| if k == "home.volume" { Some(Value::Bool(false)) } else if k == "home.recent" { Some(Value::String("x".into())) } else { None });
        assert!(!o.0[2] && !o.0[1] && o.0[0]);
        assert!(On::from_get(|k| (k == "home.recent").then_some(Value::Bool(true))).0[1]);
    }

    #[test]
    fn only_a_player_is_centred_and_the_tab_shrinks_to_it() {
        let p = plan(&on(&["module.media"]));
        assert_eq!(p.rects[0], Some(R::new(118.0, 0.0, 412.0, 154.0)));
        assert_eq!(p.h, 154.0);
        assert!(p.rects[STRIP].is_none() && p.rects[3].is_none());
    }

    #[test]
    fn player_plus_one_tile_gives_the_tile_the_side_column() {
        let p = plan(&on(&["module.media", "home.tile.area"]));
        assert_eq!(p.rects[0], Some(R::new(0.0, 0.0, 412.0, 154.0)));
        let t = p.rects[3].unwrap();
        assert_eq!((t.x, t.w), (420.0, 228.0));
        assert!(t.h <= TILE_MAX_H && (t.y + t.h / 2.0 - 77.0).abs() < 0.01, "centred next to the player: {t:?}");
        assert_eq!(p.h, 154.0);
    }

    #[test]
    fn three_tiles_fill_two_columns_and_the_odd_one_spans() {
        let p = plan(&on(&["module.media", "home.volume", "home.tile.area", "home.tile.qr", "home.tile.command"]));
        let (a, b, c) = (p.rects[3].unwrap(), p.rects[6].unwrap(), p.rects[10].unwrap());
        assert!(a.y == b.y && a.x < b.x && c.y > a.y && c.x == 420.0 && c.w == 228.0);
        assert_eq!(p.h, 200.0);
    }

    #[test]
    fn recently_copied_makes_the_tab_taller_and_stays_inside_it() {
        let p = plan(&on(&["module.media", "home.recent", "home.volume", "module.launcher"]));
        assert_eq!(p.h, 154.0 + 62.0 + 38.0 + 16.0 + 8.0 + 52.0);
        let (pl, re, vo, st) = (p.rects[0].unwrap(), p.rects[1].unwrap(), p.rects[2].unwrap(), p.rects[STRIP].unwrap());
        assert_eq!((re.y, vo.y), (pl.bottom() + 8.0, re.bottom() + 8.0));
        assert_eq!(st.bottom(), p.h);
        assert_eq!(body_h(|k| (k == "home.recent").then_some(Value::Bool(true))), 401.0, "the classic 331 plus the card and one gap");
    }

    #[test]
    fn tiles_alone_use_the_full_width_in_rows_of_four() {
        let p = plan(&on(&["home.tile.area", "home.tile.screen", "home.tile.ocr", "home.tile.qr", "home.tile.picker"]));
        let r: Vec<R> = (0..5).map(|i| p.rects[tile_part(i)].unwrap()).collect();
        assert_eq!((r[0].w, r[0].h, r[0].x), (156.0, 56.0, 0.0));
        assert_eq!(r[4].x, (W - 156.0) / 2.0, "the lone tile of the second row is centred");
        assert_eq!(p.h, 56.0 * 2.0 + 8.0);
    }

    #[test]
    fn only_shortcuts_is_just_the_strip() {
        let p = plan(&on(&["module.launcher"]));
        assert_eq!(p.rects[STRIP], Some(R::new(0.0, 0.0, 648.0, 52.0)));
        assert_eq!(p.h, 52.0);
    }

    #[test]
    fn nothing_on_is_empty_and_no_part_overlaps_another() {
        let p = plan(&On([false; N]));
        assert!(p.rects.iter().all(Option::is_none) && p.h == 0.0 && !On([false; N]).any());
        // every subset of the switches yields disjoint rectangles inside the box
        for mask in 0u32..(1 << N) {
            let o = On(std::array::from_fn(|i| mask & (1 << i) != 0));
            let p = plan(&o);
            let rs: Vec<R> = p.rects.iter().flatten().copied().collect();
            for (i, a) in rs.iter().enumerate() {
                assert!(a.x >= -0.01 && a.right() <= W + 0.01 && a.y >= -0.01 && a.bottom() <= p.h + 0.01, "{mask:b}: {a:?} outside {}", p.h);
                for b in &rs[i + 1..] {
                    let apart = a.right() <= b.x + 0.01 || b.right() <= a.x + 0.01 || a.bottom() <= b.y + 0.01 || b.bottom() <= a.y + 0.01;
                    assert!(apart, "{mask:b}: {a:?} overlaps {b:?}");
                }
            }
            assert_eq!(rs.len(), mask.count_ones() as usize);
        }
    }

    #[test]
    fn only_the_home_keys_without_a_settings_row_are_writable() {
        assert!(is_home_key("home.recent") && is_home_key("home.volume") && is_home_key("home.tile.qr"));
        assert!(!is_home_key("home.tile.nope") && !is_home_key("module.media") && !is_home_key("home"));
    }
}
