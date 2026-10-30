//! Live layer of the native Pill: every collapsed-capsule row, event card and edge reaction as data.
//!
//! Each provider is the UI half of one live-event producer. It turns backend event payloads
//! (`serde_json::Value`, shapes in `docs/native-ui-contract.md`, field names from the Rust producers) into
//! [`model::LiveCmd`]s: set/clear a [`model::RowSpec`], react/unreact (edge presets), toast a [`model::CardSpec`],
//! pulse the edge, badge a tab. The renderer (Direct2D, `src-tauri/src/ui/*`) draws that data and plays the animation
//! kinds it carries ([`model::Anim`]); Reduce motion is the renderer's switch. Nothing here touches the clock, the
//! settings file, the network or any window: time comes in as `now` (epoch ms), the rest through [`env::Env`], backend
//! questions go out as [`model::Query`]/[`model::Action`] data, and tests use [`env::MemEnv`].
//!
//! # Wiring
//! [`hub::Hub::standard`] builds all providers; [`hub::Hub::dispatch`] routes an event, applies quiet hours
//! ([`board::filter_quiet`], sources in [`board::QUIET_SRC`]) and keeps the [`board::Board`] (highest priority wins, ttl expiry).
//! Tick with [`hub::Hub::tick`] at the period [`hub::Hub::tick_ms`] asks for. Answers to [`model::Query`]s are fed back as events.
//!
//! # Vocabulary ([`model`])
//! * `RowSpec { id, priority, ttl_ms, key, accent, tall, width, tip, elements, anim }`
//! * `Elem` = `Text` (named `TextStyle`) | `Clock` | `Icon` | `Eq` | `Cover` | `Dot` | `Spinner` | `Check` | `Ring` | `Bar` | `Levels` |
//!   `Spacer` | `Flex` | `Sep` | `Group` | `Badge` | `Button { action }`
//! * `CardSpec { id, icon, tone, ttl_ms, urgent, click_first, body: Text | Notification | Color, actions, enter, leave }`
//! * `Anim` = `None | Pop | Slide | Fade | Glide(ms)`; rows carry `enter / update / leave`.
//! * `LiveCmd` = `SetRow | ClearRow | React | Unreact | Toast | EdgePulse | TabBadge | PrivacyDots | RecTile | SysMeters | Run | Query | Broadcast`
//!
//! # What each source shows (every `setLive(` / `P.toast(` / `P.react(` of `src/*.js`)
//!
//! Rows (`setLive`; Q = quiet-hours source, muted in the quiet window):
//!
//! | source file | row id | event(s) | priority | ttl | notes |
//! |---|---|---|---|---|---|
//! | live-events.js | `kbd` Q | `kbd:layout` | 90 | 1.4 s | layout code, `live.keyboardLayout` |
//! | live-events.js | `copy` Q | `clip:added` | 80 | 1.3 s | "Copied · Text/Image/N files", edge pulse .8 |
//! | live-system.js | `volume` | `live:volume` | 88 | 1.6 s | in place within the HUD; wheel +-2 |
//! | live-system.js | `download` Q | `live:download` | 55 | 3.5 s | ring + name + size (+N) |
//! | live-system.js | (corner dots) | `live:privacy` | - | - | `PrivacyDots`, not a row |
//! | live-ext.js | `obs` | `live:obs` | 90 | none | REC / LIVE timers, red outline, opt-in |
//! | live-ext.js | `steam` Q | `live:steam` | 45 | none | ring + game + percent/speed |
//! | agents-live.js | `agents` Q | `agents:changed` | 70 waiting / 60 working | none | amber outline when waiting |
//! | agents-live.js | `agent-done` Q | `agents:done` | 75 | 1.6 s | green check flash, edge pulse .9 |
//! | capture-live.js | `rec` | `rec:state`, `capture:settings` | 100 | none | 258 px capsule, Stop button |
//! | voice-live.js | `voice` | `voice:state`, `voice:level` | 96 | none | 214 px, coral outline, level bars |
//! | sysmon-live.js | `sysmon` Q | `sysmon:tick` | 90 | 8 s | GPU >= 85 C / CPU >= 95 % for 10 s, 5 min cool-down |
//! | tab-home.js | `media` | `media:changed` | 50 | none | cover accent outline; modes standard/title/mini/lyrics |
//! | tab-note.js | `timer` | `timer:tick`, `timer:state` | 60 | none | key `run<len>`, edited per second |
//! | tab-shelf.js | `shelf` | `shelf:changed` | 30 | 5 s | count chip |
//! | tab-shelf.js | `shelf-drop` | drag over the capsule | 95 (tall) | 30 s | drop card |
//!
//! Cards (`P.toast`; ttl in s):
//!
//! | source file | card id | event(s) | tone | ttl |
//! |---|---|---|---|---|
//! | live-system.js | `download:<id>` | `live:download` done | green | 9 |
//! | live-system.js | `update` | `update:available` | green | 9 |
//! | live-ext.js | `obs-saved` | `live:obs` with `file` | green | 12 |
//! | live-ext.js | `steam-done:<appid>` | `live:steam` done | green | 8 |
//! | live-ext.js | `notif:<id>` | `live:notification` | - | 7 |
//! | live-ext.js | `uninstall:<name>` | `uninstall:ready` | green / none | 12 |
//! | live-plus.js | `filedone` | `live:file-done` (+1.5 s) | green | 8 |
//! | live-plus.js | `usb:<L>:` | `live:usb` | accent / none | 4 |
//! | live-plus.js | `battery` (urgent when low) | `live:battery` | red / amber / green | 8 low, 5 else |
//! | live-plus.js | `cal:<title>` | `live:calendar` (opt-in) | accent | 12 |
//! | live-plus.js | `mail` | `live:mail` (opt-in) | accent | 10 |
//! | agents-live.js | `agent-done:<session>` | `agents:done` | green | 9 |
//! | capture-live.js | `cap` | `capture:done` (screenshot/text/qr/color/record/error) | green / amber / accent / red | 4.5-12 |
//! | voice-live.js | `voice` | `voice:state` done / error | green / amber / red | 8 / 7 |
//! | whatsnew-live.js | `whatsnew` | `whatsnew_check` (+4 s) | - | 14 |
//! | tab-note.js | `timer-done`, `remind-<id>` (quiet-exempt) | `remind:due` | - | 20 / 30 |
//!
//! Edge reactions (`P.react(source, preset, {ttlMs, color})`): `agents` attention/thinking (standing), `agent-done` done 4 s,
//! `download` file 3 s, `media` music (cover colour, standing), `voice` attention 1.6 s / thinking / done 2.2 s,
//! `capture` file 3 s, `remind` attention 5 s, `shelf` file 3 s. Edge pulses: copy .8, agent-done .9 (both muted in quiet
//! hours), steam done .7 (not muted, as before).
//!
//! Not ported here (feedback toasts of panel tabs, owned by the panel module): ai-actions.js `clip-ai`, tab-clipboard.js
//! `clip-wipe` / `clip-paste`, tab-home.js `dev-settings` / `launcher-*`.
//!
//! # Deliberate differences from the JS
//! * Looping motion (dot pulse, spinner, equaliser, ring spin) is a flag on the element, not a timer here.
//! * Where the JS edited a DOM node in place every tick (timers, level bars) a provider re-sets the row with the *same key*.
//! * The media row key also tells whether the cover picture is there: the JS key (mode|playing|title|artist|cover id|accent) skipped the
//!   redraw when a missed cover arrived later for the same id, leaving the placeholder on screen.
//! * Row/obs/rec/voice clocks are edited only when the displayed text changes (the JS wrote the DOM node every tick).
//! * String lengths and slices are UTF-16 based (`util`), so cut points match the web UI, including emoji.

pub mod agents;
pub mod board;
pub mod capture;
pub mod env;
pub mod events;
pub mod ext;
pub mod hub;
pub mod media;
pub mod model;
pub mod plus;
pub mod shelf;
pub mod sysmon;
pub mod system;
pub mod timers;
pub mod util;
pub mod voice;
pub mod whatsnew;

pub use board::{filter_quiet, Board, QUIET_SRC};
pub use env::{Env, MemEnv, PillState, Tr};
pub use hub::{Hub, Provider};
pub use model::{Accent, Action, Anim, CardSpec, Elem, LiveCmd, Preset, Query, Rgba, RowSpec, TextStyle};
