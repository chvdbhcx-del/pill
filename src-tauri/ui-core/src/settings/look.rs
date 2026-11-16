//! Share your look: the appearance of the island as one small, versioned `.pilllook` file or a `pill-look:` text, and the way back in.
//!
//! The rule is a WHITELIST: [`WHITELIST`] names every key that may travel, nothing else is ever written or accepted (the unit tests freeze the
//! list, so a key cannot sneak in by accident). Only how the island LOOKS goes: theme, colours, size, transparency, where it stands, the clock,
//! the edge style, the pet, the player widget's PLACEMENT. Never a module or privacy switch (`module.*`, `live.*`, `media.widget` on/off), no
//! hotkey, account, token, path, address, calendar / mail / phone / relay setting, nothing that names the person or the PC.
//!
//! ```json
//! { "kind": "pill-look", "version": 1, "name": "Midnight", "author": "optional", "values": { "theme": "dark", "size": 100 } }
//! ```
//!
//! Import is strict: a file with a key outside the whitelist, a wrong kind, a newer version, a value that the settings model refuses (not one
//! of the options, wrong type) is refused as a whole; a slider value is clamped / snapped by the model like a click would. A look may hold only
//! some of the keys. Nothing here touches the settings: [`plan`] only validates and says what would change (the window shows it in the live
//! preview first), [`apply`] writes it through the model and hands back an [`Undo`].
use crate::settings::calls::BackendCall;
use crate::settings::model::SettingsModel;
use crate::settings::tr::{pick, Lang};
use serde_json::{json, Map, Value};

pub const KIND: &str = "pill-look";
pub const VERSION: u64 = 1;
pub const TEXT_PREFIX: &str = "pill-look:";
pub const FILE_EXT: &str = "pilllook";
/// A look is a few hundred bytes; anything bigger is not one.
pub const MAX_FILE_BYTES: usize = 8 * 1024;
/// The longest pasted text (the `pill-look:` prefix and the base64url body).
pub const MAX_TEXT_CHARS: usize = 4 * 1024;
pub const MAX_NAME_CHARS: usize = 40;

/// The only keys of a look, in the order they are applied (`edge.position` before `edge.align`: choosing the taskbar moves a centred side to the left,
/// the look's own side must come after). Adding a key means adding it to `FROZEN` in the tests too, on purpose.
pub const WHITELIST: &[&str] = &[
    // the island
    "theme", "island.color", "island.colorCustom", "shape", "size", "opacityCol", "opacityExp",
    // where it stands
    "edge.position", "edge.align", "underTaskbar", "offsetX",
    // the clock and the date
    "showTime", "showDate", "showSeconds", "clock24",
    // the edge style (its animation) and the pet
    "character", "character.liquid", "pet", "pet.side", "pet.size",
    // how the player looks on the island and on Home; where the player widget stands (not whether it is on)
    "musicPill", "home.player", "media.widget.edge", "media.widget.align", "media.widget.offset",
];

pub fn allowed(key: &str) -> bool {
    WHITELIST.contains(&key)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    pub name: String,
    pub author: String,
    pub values: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LookError {
    /// nothing to read (an empty clipboard, an empty file)
    Empty,
    TooBig,
    /// not JSON, not base64, not the shape of a look
    NotALook,
    /// made by a newer Pill
    Newer(u64),
    /// a key outside the whitelist
    Forbidden(String),
    /// a value the settings refuse
    Bad(String),
    /// no known key in it
    NoValues,
}

impl LookError {
    pub fn message(&self, lang: Lang) -> String {
        match self {
            LookError::Empty => pick(lang, "Нечего читать: пусто", "Nothing to read: it is empty").to_string(),
            LookError::TooBig => pick(lang, "Слишком большой для оформления", "Too big to be a look").to_string(),
            LookError::NotALook => pick(lang, "Это не оформление Pill", "This is not a Pill look").to_string(),
            LookError::Newer(_) => pick(lang, "Оформление сделано в более новой версии Pill", "This look was made by a newer Pill").to_string(),
            LookError::Forbidden(k) => format!("{}{}", pick(lang, "В оформлении лишняя настройка: ", "The look carries a setting it must not: "), shown(k)),
            LookError::Bad(k) => format!("{}{}", pick(lang, "Недопустимое значение: ", "Not a valid value: "), shown(k)),
            LookError::NoValues => pick(lang, "В оформлении нет известных настроек", "The look has no known settings").to_string(),
        }
    }
}

/// A key as it is quoted in a message (cut, no control characters: it came from a stranger's file).
fn shown(k: &str) -> String {
    k.chars().filter(|c| !c.is_control()).take(40).collect()
}

fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect::<String>().trim().chars().take(MAX_NAME_CHARS).collect()
}

// ------------------------------------------------------------------------------------------------ export

/// The look of this settings model: every whitelisted key with its current value (the default while it was never changed).
pub fn export(model: &SettingsModel, name: &str) -> Look {
    let values = WHITELIST.iter().filter_map(|k| Some((k.to_string(), non_null(model.get(k))?))).collect();
    Look { name: clean(name), author: String::new(), values }
}

fn non_null(v: Value) -> Option<Value> {
    (!v.is_null()).then_some(v)
}

fn document(look: &Look) -> Value {
    let mut o = Map::new();
    o.insert("kind".into(), json!(KIND));
    o.insert("version".into(), json!(VERSION));
    o.insert("name".into(), json!(clean(&look.name)));
    if !clean(&look.author).is_empty() {
        o.insert("author".into(), json!(clean(&look.author)));
    }
    o.insert("values".into(), Value::Object(look.values.clone()));
    Value::Object(o)
}

pub fn to_json(look: &Look) -> String {
    serde_json::to_string_pretty(&document(look)).unwrap_or_default()
}

/// `pill-look:` + base64url of the compact JSON: one line that goes through any chat.
pub fn to_text(look: &Look) -> String {
    format!("{TEXT_PREFIX}{}", b64_encode(serde_json::to_string(&document(look)).unwrap_or_default().as_bytes()))
}

/// The file name of an export: `<name>.pilllook` with only safe characters.
pub fn file_name(look: &Look) -> String {
    let stem: String = look.name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect::<String>().trim_matches('-').to_string();
    format!("{}.{FILE_EXT}", if stem.is_empty() { "look" } else { &stem })
}

// ------------------------------------------------------------------------------------------------ import

/// A file (JSON) or a pasted text (`pill-look:...`, or a pasted JSON) into a [`Look`]. No setting is looked at yet.
pub fn parse(input: &str) -> Result<Look, LookError> {
    let t = input.trim_start_matches('\u{feff}').trim();
    if t.is_empty() {
        return Err(LookError::Empty);
    }
    if t.len() > MAX_FILE_BYTES.max(MAX_TEXT_CHARS) {
        return Err(LookError::TooBig);
    }
    if t.starts_with('{') {
        return from_json(t);
    }
    let body = t.strip_prefix(TEXT_PREFIX).ok_or(LookError::NotALook)?;
    let body: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    if body.len() + TEXT_PREFIX.len() > MAX_TEXT_CHARS {
        return Err(LookError::TooBig);
    }
    let bytes = b64_decode(&body).ok_or(LookError::NotALook)?;
    from_json(std::str::from_utf8(&bytes).map_err(|_| LookError::NotALook)?)
}

fn from_json(text: &str) -> Result<Look, LookError> {
    if text.len() > MAX_FILE_BYTES {
        return Err(LookError::TooBig);
    }
    let Ok(Value::Object(o)) = serde_json::from_str::<Value>(text) else { return Err(LookError::NotALook) };
    if o.get("kind").and_then(Value::as_str) != Some(KIND) {
        return Err(LookError::NotALook);
    }
    match o.get("version").and_then(Value::as_u64) {
        Some(v) if v > VERSION => return Err(LookError::Newer(v)),
        Some(v) if v >= 1 => {}
        _ => return Err(LookError::NotALook),
    }
    let text_of = |k: &str| match o.get(k) {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(s)) => Ok(clean(s)),
        Some(_) => Err(LookError::NotALook),
    };
    let values = match o.get("values") {
        Some(Value::Object(v)) => v.clone(),
        _ => return Err(LookError::NotALook),
    };
    Ok(Look { name: text_of("name")?, author: text_of("author")?, values })
}

/// What importing a look would do: the checked values (as the settings model keeps them), in the order they are applied.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub name: String,
    pub author: String,
    pub set: Vec<(&'static str, Value)>,
    /// how many of them differ from what the settings have now
    pub changes: usize,
}

impl Plan {
    pub fn values(&self) -> Map<String, Value> {
        self.set.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }
}

/// Validates `look` against the whitelist and the settings model's own validators.
pub fn plan(look: &Look, model: &SettingsModel) -> Result<Plan, LookError> {
    if let Some(k) = look.values.keys().find(|k| !allowed(k)) {
        return Err(LookError::Forbidden(k.clone()));
    }
    let mut set = Vec::new();
    for key in WHITELIST {
        let Some(v) = look.values.get(*key) else { continue };
        let patch = model.normalize(key, v).map_err(|_| LookError::Bad(key.to_string()))?;
        let v = patch.get(*key).cloned().ok_or_else(|| LookError::Bad(key.to_string()))?;
        if *key == "island.colorCustom" && !v.as_str().is_some_and(|s| s.is_empty() || (s.len() <= 9 && crate::shell::tint::parse_hex(s).is_some())) {
            return Err(LookError::Bad(key.to_string()));
        }
        set.push((*key, v));
    }
    if set.is_empty() {
        return Err(LookError::NoValues);
    }
    let changes = set.iter().filter(|(k, v)| model.get(k) != *v).count();
    Ok(Plan { name: look.name.clone(), author: look.author.clone(), set, changes })
}

// ------------------------------------------------------------------------------------------------ apply and undo

/// The values a look replaced, to put them back.
#[derive(Clone, Debug, PartialEq)]
pub struct Undo {
    pub name: String,
    pub prev: Vec<(&'static str, Value)>,
}

/// Writes the plan through the model (as clicks would) and returns the backend calls (one `settings_set`) plus the way back.
pub fn apply(model: &mut SettingsModel, plan: &Plan) -> (Vec<BackendCall>, Undo) {
    let prev = plan.set.iter().map(|(k, _)| (*k, model.get(k))).collect();
    let mut calls = Vec::new();
    for (k, v) in &plan.set {
        if let Ok(c) = model.try_set(k, v.clone()) {
            calls.extend(c);
        }
    }
    (one_patch(calls), Undo { name: plan.name.clone(), prev })
}

pub fn undo(model: &mut SettingsModel, u: &Undo) -> Vec<BackendCall> {
    let mut calls = Vec::new();
    for (k, v) in &u.prev {
        if let Ok(c) = model.try_set(k, v.clone()) {
            calls.extend(c);
        }
    }
    one_patch(calls)
}

/// Every `settings_set` of the list as ONE patch (later values win), at the place of the first.
fn one_patch(calls: Vec<BackendCall>) -> Vec<BackendCall> {
    let mut all = Map::new();
    let mut out = Vec::new();
    let mut at = None;
    for c in calls {
        match c {
            BackendCall::SettingsSet { patch } => {
                all.extend(patch);
                if at.is_none() {
                    at = Some(out.len());
                    out.push(BackendCall::SettingsSet { patch: Map::new() });
                }
            }
            other => out.push(other),
        }
    }
    if let Some(i) = at {
        out[i] = BackendCall::SettingsSet { patch: all };
    }
    out
}

// ------------------------------------------------------------------------------------------------ the built-in looks

/// A look that ships with Pill. It goes through [`parse`] and [`plan`] like anything pasted.
pub struct Sample {
    pub name: &'static str,
    pub json: &'static str,
}

pub const SAMPLES: [Sample; 4] = [
    Sample {
        name: "Midnight",
        json: r##"{"kind":"pill-look","version":1,"name":"Midnight","author":"Pill","values":{"theme":"dark","island.color":"#0b1b3d","island.colorCustom":"","shape":"notch","size":100,"opacityCol":0,"opacityExp":0,"showTime":true,"showDate":false,"showSeconds":false,"clock24":true,"character":"liquid","pet":"ghost","pet.side":"right","pet.size":"m"}}"##,
    },
    Sample {
        name: "Paper",
        json: r##"{"kind":"pill-look","version":1,"name":"Paper","author":"Pill","values":{"theme":"light","island.color":"#f6f6f6","island.colorCustom":"","shape":"capsule","size":95,"opacityCol":0,"opacityExp":10,"showTime":true,"showDate":true,"showSeconds":false,"clock24":false,"character":"none","pet":"none"}}"##,
    },
    Sample {
        name: "Neon",
        json: r##"{"kind":"pill-look","version":1,"name":"Neon","author":"Pill","values":{"theme":"dark","island.color":"#2c1448","island.colorCustom":"#7a2bff","shape":"capsule","size":110,"opacityCol":0,"opacityExp":0,"showTime":true,"showDate":false,"showSeconds":true,"clock24":true,"character":"glow","pet":"robot","pet.side":"left","pet.size":"l"}}"##,
    },
    Sample {
        name: "Quiet",
        json: r##"{"kind":"pill-look","version":1,"name":"Quiet","author":"Pill","values":{"theme":"auto","island.color":"","island.colorCustom":"","shape":"notch","size":90,"opacityCol":20,"opacityExp":10,"showTime":true,"showDate":false,"showSeconds":false,"clock24":true,"character":"none","pet":"none"}}"##,
    },
];

// ------------------------------------------------------------------------------------------------ base64url (no padding)

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub fn b64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..=c.len() {
            out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

/// base64url, or the plain alphabet (`+` `/`), with or without `=` padding; None for anything else.
pub fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.trim_end_matches('=');
    if s.len() % 4 == 1 {
        return None;
    }
    let val = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'-' | b'+' => Some(62),
        b'_' | b'/' => Some(63),
        _ => None,
    };
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    for c in s.as_bytes().chunks(4) {
        let mut n = 0u32;
        for (i, b) in c.iter().enumerate() {
            n |= (val(*b)? as u32) << (18 - 6 * i);
        }
        for i in 0..c.len() - 1 {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole whitelist, written out a second time on purpose: widening what a shared look may carry must be a visible, reviewed edit of this list.
    const FROZEN: [&str; 25] = [
        "theme", "island.color", "island.colorCustom", "shape", "size", "opacityCol", "opacityExp", "edge.position", "edge.align", "underTaskbar", "offsetX", "showTime", "showDate",
        "showSeconds", "clock24", "character", "character.liquid", "pet", "pet.side", "pet.size", "musicPill", "home.player", "media.widget.edge", "media.widget.align", "media.widget.offset",
    ];

    fn model() -> SettingsModel {
        let mut m = SettingsModel::new("en");
        // a settings.json full of everything a look must never carry
        let mut v = Map::new();
        for (k, x) in [
            ("theme", json!("light")), ("size", json!(120)), ("module.media", json!(false)), ("media.widget", json!(true)), ("hotkey.toggle", json!("Ctrl+Q")),
            ("phone.relayUrl", json!("wss://x.example")), ("mail.user", json!("me@example.com")), ("weather.city", json!("Paris")), ("live.calendar.path", json!("C:\\me\\cal.ics")),
            ("ai.apiKeyEnc", json!("blob")), ("capture.folder", json!("D:\\clips")), ("language", json!("ru")), ("monitor", json!("2")),
        ] {
            v.insert(k.to_string(), x);
        }
        m.begin(v, None);
        m
    }

    fn text_of(look: &Look) -> String {
        to_text(look)
    }

    #[test]
    fn the_whitelist_is_frozen_and_every_key_is_a_plain_stored_look_setting() {
        assert_eq!(WHITELIST, FROZEN, "the whitelist changed: say so here too");
        let sc = crate::settings::schema();
        for k in WHITELIST {
            let row = sc.row_for_key(k).unwrap_or_else(|| panic!("{k} has no row"));
            assert!(row.is_stored(), "{k}");
            assert!(!k.starts_with("module.") && !k.starts_with("live.") && !k.starts_with("hotkey.") && !k.starts_with("phone.") && !k.starts_with("mail."), "{k}");
            assert!(!k.contains("key") && !k.contains("token") && !k.contains("path") && !k.contains("Enc"), "{k}");
            // free text is only the colour; a user name, a path, an address is never a look
            if matches!(row.control, crate::settings::schema::Control::Text { .. }) {
                assert_eq!(*k, "island.colorCustom");
            }
        }
        assert!(!allowed("media.widget"), "the widget's on / off is a privacy gate, only its place travels");
        assert!(!allowed("reduceMotion") && !allowed("language") && !allowed("monitor") && !allowed("fps"));
    }

    #[test]
    fn export_has_only_whitelisted_keys_whatever_the_settings_hold() {
        let m = model();
        let look = export(&m, "Mine");
        assert!(look.values.keys().all(|k| allowed(k)));
        assert_eq!(look.values.len(), WHITELIST.len(), "every look key is in (the default where it was never set)");
        assert_eq!(look.values["theme"], json!("light"));
        assert_eq!(look.values["size"], json!(120));
        // not one stranger in the file or the text, in any spelling
        for s in [to_json(&look), text_of(&look)] {
            let l = parse(&s).unwrap();
            assert_eq!(l.values.keys().collect::<Vec<_>>().len(), WHITELIST.len());
        }
        let file = to_json(&look);
        for needle in ["module.", "hotkey", "relay", "example.com", "Paris", "cal.ics", "apiKey", "clips", "\"language\"", "media.widget\"", "C:\\"] {
            assert!(!file.contains(needle), "{needle} leaked into {file}");
        }
    }

    #[test]
    fn import_refuses_anything_outside_the_whitelist() {
        let m = model();
        for k in ["module.media", "media.widget", "hotkey.toggle", "phone.relayUrl", "mail.user", "weather.city", "ai.apiKeyEnc", "capture.folder", "language", "reduceMotion", "live.mail", "profiles"] {
            let mut l = parse(SAMPLES[0].json).unwrap();
            l.values.insert(k.to_string(), json!(true));
            assert_eq!(plan(&l, &m), Err(LookError::Forbidden(k.to_string())), "{k}");
        }
    }

    #[test]
    fn import_validates_the_file_the_version_and_the_values() {
        let m = model();
        let look = |values: Value| to_json(&Look { name: "x".into(), author: String::new(), values: values.as_object().unwrap().clone() });
        let plan_of = |s: &str| parse(s).and_then(|l| plan(&l, &m));
        assert_eq!(parse(""), Err(LookError::Empty));
        assert_eq!(parse("  \n"), Err(LookError::Empty));
        assert_eq!(parse("hello"), Err(LookError::NotALook));
        assert_eq!(parse("pill-look:!!!"), Err(LookError::NotALook));
        assert_eq!(parse("{\"theme\":\"dark\"}"), Err(LookError::NotALook), "no kind");
        assert_eq!(parse(r#"{"kind":"other","version":1,"values":{}}"#), Err(LookError::NotALook));
        assert_eq!(parse(r#"{"kind":"pill-look","version":2,"values":{}}"#), Err(LookError::Newer(2)));
        assert_eq!(parse(r#"{"kind":"pill-look","version":0,"values":{}}"#), Err(LookError::NotALook));
        assert_eq!(parse(r#"{"kind":"pill-look","version":1}"#), Err(LookError::NotALook), "no values");
        assert_eq!(parse(&"x".repeat(MAX_FILE_BYTES + 1)), Err(LookError::TooBig));
        assert_eq!(parse(&format!("{TEXT_PREFIX}{}", "A".repeat(MAX_TEXT_CHARS))), Err(LookError::TooBig));
        assert_eq!(plan_of(&look(json!({}))), Err(LookError::NoValues));
        assert_eq!(plan_of(&look(json!({ "theme": "neon" }))), Err(LookError::Bad("theme".into())), "not an option");
        assert_eq!(plan_of(&look(json!({ "size": "big" }))), Err(LookError::Bad("size".into())), "wrong type");
        assert_eq!(plan_of(&look(json!({ "pet": "dragon" }))), Err(LookError::Bad("pet".into())));
        assert_eq!(plan_of(&look(json!({ "character": "x" }))), Err(LookError::Bad("character".into())));
        assert_eq!(plan_of(&look(json!({ "island.colorCustom": "red" }))), Err(LookError::Bad("island.colorCustom".into())));
        assert_eq!(plan_of(&look(json!({ "island.colorCustom": "C:\\Users\\me" }))), Err(LookError::Bad("island.colorCustom".into())));
        assert_eq!(plan_of(&look(json!({ "theme": ["dark"] }))), Err(LookError::Bad("theme".into())));
        // a slider is clamped and snapped by the model, like a click
        let p = plan_of(&look(json!({ "size": 500, "opacityExp": 33, "media.widget.offset": -400 }))).unwrap();
        assert_eq!(p.values()["size"], json!(200));
        assert_eq!(p.values()["opacityExp"], json!(35));
        assert_eq!(p.values()["media.widget.offset"], json!(-50));
        // a good colour, with or without the hash, and the empty one
        assert!(plan_of(&look(json!({ "island.colorCustom": "#3a2a5c" }))).is_ok() && plan_of(&look(json!({ "island.colorCustom": "" }))).is_ok());
        // the plan is in the application order whatever the file's order was
        let p = plan_of(&look(json!({ "edge.align": "left", "edge.position": "bottom", "theme": "dark" }))).unwrap();
        assert_eq!(p.set.iter().map(|(k, _)| *k).collect::<Vec<_>>(), ["theme", "edge.position", "edge.align"]);
        // changes counts what differs from now (theme light, size 120 in this model)
        assert_eq!(plan_of(&look(json!({ "theme": "light", "size": 120, "shape": "notch" }))).unwrap().changes, 0);
        assert_eq!(plan_of(&look(json!({ "theme": "dark", "size": 120 }))).unwrap().changes, 1);
    }

    #[test]
    fn a_look_survives_the_file_and_the_text() {
        let m = model();
        let look = export(&m, "  My\u{7} look  ");
        assert_eq!(look.name, "My look", "control characters and spaces are cleaned");
        let from_file = parse(&to_json(&look)).unwrap();
        let from_text = parse(&text_of(&look)).unwrap();
        assert_eq!(from_file, look);
        assert_eq!(from_text, look);
        assert!(text_of(&look).starts_with(TEXT_PREFIX) && !text_of(&look).contains(char::is_whitespace));
        assert!(text_of(&look).len() < MAX_TEXT_CHARS, "{} chars", text_of(&look).len());
        // a chat wraps lines and adds spaces; a pasted file works too; the plain base64 alphabet and the padding are tolerated
        let t = text_of(&look);
        let wrapped = format!("  {}\n {}  ", &t[..40], &t[40..]);
        assert_eq!(parse(&wrapped).unwrap(), look);
        assert_eq!(parse(&to_json(&look)).unwrap(), look);
        let body = &t[TEXT_PREFIX.len()..];
        let padded = format!("{TEXT_PREFIX}{body}{}", "=".repeat((4 - body.len() % 4) % 4));
        assert_eq!(parse(&padded).unwrap(), look);
        // author is optional and survives
        let l = Look { author: "Ann".into(), ..look.clone() };
        assert_eq!(parse(&to_text(&l)).unwrap().author, "Ann");
        assert!(!to_json(&look).contains("author"));
        // the plan of an export is the same settings, so applying your own look changes nothing
        assert_eq!(plan(&look, &m).unwrap().changes, 0);
        assert_eq!(file_name(&look), "My-look.pilllook");
        assert_eq!(file_name(&Look { name: "../..".into(), ..look }), "look.pilllook");
    }

    #[test]
    fn base64url_roundtrips_every_length() {
        for n in 0..40usize {
            let b: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(b64_decode(&b64_encode(&b)).unwrap(), b, "{n}");
        }
        assert_eq!(b64_encode(&[0xfb, 0xff]), "-_8", "url alphabet");
        assert_eq!(b64_decode("a"), None);
        assert_eq!(b64_decode("a b"), None);
    }

    #[test]
    fn apply_writes_one_patch_and_undo_puts_every_value_back() {
        let mut m = model();
        let before = WHITELIST.iter().map(|k| (*k, m.get(k))).collect::<Vec<_>>();
        let l = parse(SAMPLES[2].json).unwrap();
        let p = plan(&l, &m).unwrap();
        assert!(p.changes > 5);
        let (calls, u) = apply(&mut m, &p);
        assert_eq!(calls.len(), 1, "one settings_set");
        let BackendCall::SettingsSet { patch } = &calls[0] else { panic!("{calls:?}") };
        assert!(patch.keys().all(|k| allowed(k)), "{patch:?}");
        assert_eq!((m.get("theme"), m.get("island.colorCustom"), m.get("character")), (json!("dark"), json!("#7a2bff"), json!("glow")));
        assert_eq!(u.name, "Neon");
        let calls = undo(&mut m, &u);
        assert_eq!(calls.len(), 1);
        let after = WHITELIST.iter().map(|k| (*k, m.get(k))).collect::<Vec<_>>();
        assert_eq!(before, after);
        // the values the window holds are what the settings file gets
        let BackendCall::SettingsSet { patch } = &calls[0] else { panic!() };
        assert_eq!(patch.get("theme"), Some(&json!("light")));
    }

    #[test]
    fn the_taskbar_side_effect_does_not_beat_the_looks_own_side() {
        let mut m = SettingsModel::new("en");
        m.begin(Map::new(), None);
        let l = Look { name: "t".into(), author: String::new(), values: json!({ "edge.position": "taskbar", "edge.align": "center" }).as_object().unwrap().clone() };
        let p = plan(&l, &m).unwrap();
        let (_, u) = apply(&mut m, &p);
        assert_eq!((m.get("edge.position"), m.get("edge.align")), (json!("taskbar"), json!("center")), "the look said centre");
        undo(&mut m, &u);
        assert_eq!((m.get("edge.position"), m.get("edge.align")), (json!("top"), json!("center")));
    }

    #[test]
    fn the_four_built_in_looks_are_valid_and_named() {
        let m = model();
        assert_eq!(SAMPLES.iter().map(|s| s.name).collect::<Vec<_>>(), ["Midnight", "Paper", "Neon", "Quiet"]);
        for s in &SAMPLES {
            let l = parse(s.json).unwrap_or_else(|e| panic!("{}: {e:?}", s.name));
            assert_eq!(l.name, s.name);
            let p = plan(&l, &m).unwrap_or_else(|e| panic!("{}: {e:?}", s.name));
            // what they set is exactly what they say (nothing was snapped or clamped on the way)
            assert_eq!(p.values(), l.values, "{} is stored as the model would keep it", s.name);
            assert!(s.json.len() < 600);
        }
    }

    #[test]
    fn messages_are_in_both_languages() {
        for e in [LookError::Empty, LookError::TooBig, LookError::NotALook, LookError::Newer(3), LookError::Forbidden("a\nb".into()), LookError::Bad("size".into()), LookError::NoValues] {
            let (ru, en) = (e.message(Lang::Ru), e.message(Lang::En));
            assert!(!ru.is_empty() && !en.is_empty() && ru != en, "{e:?}");
            assert!(!ru.contains('\n') && !en.contains('\n'));
        }
    }
}
