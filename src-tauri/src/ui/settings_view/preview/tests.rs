//! Headless tests of the live preview: the panel shows the island and follows a changed setting before anything is saved, shows sample data
//! for what is not on screen, stays away from a narrow window, respects Reduce motion, costs nothing once settled and never reaches the
//! user's data or the real island. `PILL_SNAP_DIR=<dir>` writes the PNGs.
use super::scene::{of_key, of_page, shareable, Flash, Look, Scene, Tab};
use super::*;
use crate::ui::host::testing::{motion_lock, motion_on};
use crate::ui::settings_view::pal::pal;
use crate::ui::settings_view::tests::{click, snap};
use crate::ui::settings_view::tests2::{boot, T};
use crate::ui::settings_view::settings_light;
use glasskit::input::InputEvent;
use glasskit::testkit::Image;
use glasskit::theme::Theme;
use serde_json::json;

const WIDE: (f32, f32) = (1240.0, 760.0);

/// Runs `f` on one long-lived thread of its own. The preview tests draw the real island and tabs, whose Direct2D objects live in thread-locals;
/// on a test thread those are released while the next test already draws on the pooled device. One thread that never ends (and keeps its own
/// device) takes that race away. A panic in `f` is the test's failure.
fn gfx(f: impl FnOnce() + Send + 'static) {
    use std::sync::{mpsc, Mutex, OnceLock};
    type Job = Box<dyn FnOnce() + Send>;
    static TX: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    let tx = TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new().name("preview-gfx".into()).spawn(move || rx.into_iter().for_each(|job| job())).expect("a thread");
        Mutex::new(tx)
    });
    let (done_tx, done_rx) = mpsc::channel();
    let job: Job = Box::new(move || {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        let _ = done_tx.send(r);
    });
    tx.lock().unwrap_or_else(|e| e.into_inner()).send(job).expect("the gfx thread is alive");
    if let Err(p) = done_rx.recv().expect("an answer") {
        std::panic::resume_unwind(p);
    }
}

/// Frames and replies until the window is quiet; a rendered frame is what gives the view its layout.
fn settle(t: &mut T, size: (f32, f32), th: &Theme) {
    for _ in 0..6 {
        t.sim.pump();
        t.sim.run(4000);
        let _ = t.sim.render(size, th);
        t.sim.frame.request();
    }
    t.sim.pump();
    t.sim.run(4000);
}

fn wide(page: &str, settings: serde_json::Value) -> T {
    let mut t = boot(settings, page);
    settle(&mut t, WIDE, &Theme::settings_dark());
    t
}

/// A change the way a control makes it: the model first, then the page follows.
fn set(t: &mut T, key: &str, v: serde_json::Value) {
    {
        let mut view = t.v.borrow_mut();
        view.model.set(key, v);
        view.sync_page();
    }
    t.sim.frame.request();
}

fn pv_rect(size: (f32, f32)) -> Rect {
    Rect::new(size.0 - 1.0 - PV_W, 31.0, PV_W, size.1 - 32.0)
}

fn shot(t: &mut T, name: &str, th: &Theme) -> Image {
    snap(&mut t.sim, name, th, WIDE)
}

/// Pixels of the black island body inside the panel.
fn island_px(img: &Image) -> usize {
    img.count(Some(pv_rect(WIDE)), |p| p[3] == 255 && p[0].max(p[1]).max(p[2]) < 10)
}

#[test]
fn the_panel_shows_the_island_and_follows_a_setting_before_it_is_saved() {
    gfx(|| {
        let _g = motion_on();
        let th = Theme::settings_dark();
        let mut t = wide("appearance", json!({"language": "en", "showSeconds": false}));
        assert!(t.v.borrow().pv.width() > PV_W - 0.5, "the panel is open on a wide window");
        let a = shot(&mut t, "preview-appearance-dark-1x", &th);
        assert!(island_px(&a) > 600, "the folded island is drawn in the panel: {}", island_px(&a));
        let saved = t.calls("settings_set").len();
        // 1: seconds on the clock
        set(&mut t, "showSeconds", json!(true));
        settle(&mut t, WIDE, &th);
        let b = shot(&mut t, "preview-seconds-dark-1x", &th);
        assert!(b.diff(&a, 3) > 40, "the clock grew by the seconds: {}", b.diff(&a, 3));
        assert_eq!(t.v.borrow().pv.scene(), Scene::Pill(Look::Clock));
        assert!(t.calls("settings_set").len() == saved, "nothing was sent to the backend by the test itself: the preview reads the model");
        // 2: the weather tab appears with its sample forecast, before any save
        set(&mut t, "net.weather", json!(true));
        settle(&mut t, WIDE, &th);
        let c = shot(&mut t, "preview-weather-dark-1x", &th);
        assert_eq!(t.v.borrow().pv.scene(), Scene::Panel(Tab::Weather));
        assert!(island_px(&c) > 3000 && c.diff(&b, 3) > 3000, "the open island is drawn: {} px, differs by {}", island_px(&c), c.diff(&b, 3));
        {
            let v = t.v.borrow();
            let (_, w, h) = v.pv.live.as_ref().expect("a sandbox").body();
            assert!(w > 400.0 && h > 200.0, "the open island: {w} x {h}");
            assert!(v.pv.stage_h.at() > 150.0, "the stage grew with it: {}", v.pv.stage_h.at());
        }
        // 3: a reaction of the folded island, with sample data
        set(&mut t, "live.copy", json!(false));
        settle(&mut t, WIDE, &th);
        assert_eq!(t.v.borrow().pv.scene(), Scene::Flash(Flash::Copy), "switching a reaction off shows the island without it");
        let off = shot(&mut t, "preview-copy-off-dark-1x", &th);
        set(&mut t, "live.copy", json!(true));
        t.sim.pump();
        t.sim.run(70); // mid-flash
        let _ = t.sim.render(WIDE, &th);
        t.sim.run(40);
        let d = shot(&mut t, "preview-copy-dark-1x", &th);
        assert!(d.diff(&off, 3) > 150, "the copy flash is on the island: {}", d.diff(&off, 3));
        settle(&mut t, WIDE, &th);
        assert_eq!(t.v.borrow().pv.scene(), Scene::Flash(Flash::Copy));
        {
            let v = t.v.borrow();
            let (_, w, h) = v.pv.live.as_ref().expect("a sandbox").body();
            assert!(w < 400.0 && h < 80.0, "back to the folded island: {w} x {h}");
        }
        let _ = d;
        // 4: clipboard rows show the clipboard tab
        set(&mut t, "clipClickPaste", json!(false));
        settle(&mut t, WIDE, &th);
        let e = shot(&mut t, "preview-clipboard-dark-1x", &th);
        assert_eq!(t.v.borrow().pv.scene(), Scene::Panel(Tab::Clipboard));
        assert!(e.diff(&c, 3) > 200, "a different tab is on show");

    });
}

#[test]
fn a_page_opens_with_its_own_scene_and_an_unmapped_setting_keeps_the_scene() {
    gfx(|| {
        let _g = motion_on();
        let th = Theme::settings_dark();
        let mut t = wide("clipboard", json!({"language": "en"}));
        assert_eq!(t.v.borrow().pv.scene(), Scene::Panel(Tab::Clipboard));
        set(&mut t, "phone.autoAccept", json!(true)); // a setting with nothing to show
        settle(&mut t, WIDE, &th);
        assert_eq!(t.v.borrow().pv.scene(), Scene::Panel(Tab::Clipboard), "an unmapped row keeps the last preview");
        let mut t = wide("live", json!({"language": "en"}));
        assert_eq!(t.v.borrow().pv.scene(), Scene::Flash(Flash::Notif));
        assert_eq!(of_page("appearance"), Scene::DEFAULT);

    });
}

#[test]
fn light_theme_and_double_scale_render() {
    gfx(|| {
        let _g = motion_on();
        let th = settings_light();
        let mut t = boot(json!({"language": "en"}), "appearance");
        t.sim.set_scale(2.0);
        settle(&mut t, WIDE, &th);
        let img = snap(&mut t.sim, "preview-appearance-light-2x", &th, WIDE);
        assert_eq!((img.w, img.h), (2480, 1520));
        // the panel is a stage of the light kind (not the dark desktop) with the black island on it
        let r = pv_rect(WIDE);
        let bright = img.count(Some(Rect::new(r.x + 20.0, r.y + 60.0, 40.0, 20.0)), |p| p[0].min(p[1]).min(p[2]) > 150);
        assert!(bright > 100, "light stage: {bright}");
        let dark = img.count(Some(r), |p| p[3] == 255 && p[0].max(p[1]).max(p[2]) < 10);
        assert!(dark > 1500, "the island at double scale: {dark}");

    });
}

#[test]
fn reduce_motion_lands_at_once_and_the_window_idles() {
    gfx(|| {
        let _g = motion_lock();
        glasskit::ease::force_reduced(Some(true));
        let th = Theme::settings_dark();
        let mut t = boot(json!({"language": "en", "reduceMotion": true}), "appearance");
        t.sim.cx.reduced_motion = true;
        settle(&mut t, WIDE, &th);
        assert!((t.v.borrow().pv.width() - PV_W).abs() < 0.01, "no slide-in");
        set(&mut t, "net.weather", json!(true));
        for _ in 0..3 {
            t.sim.pump();
            t.sim.run(3);
            let _ = t.sim.render(WIDE, &th);
        }
        // the camera stands at its target without a spring
        let (z, h) = {
            let v = t.v.borrow();
            (v.pv.zoom.moving(), v.pv.stage_h.moving())
        };
        assert!(!z && !h, "reduced motion: the camera does not glide");
        settle(&mut t, WIDE, &th);
        let img = shot(&mut t, "preview-weather-reduced", &th);
        assert!(island_px(&img) > 3000);
        glasskit::ease::force_reduced(None);
        assert!(!t.sim.frame.wants_frame(), "settled: the window is idle");

    });
}

#[test]
fn a_narrow_window_has_no_panel_and_the_page_keeps_its_room() {
    gfx(|| {
        let _g = motion_on();
        let th = Theme::settings_dark();
        let mut t = boot(json!({"language": "en"}), "appearance");
        settle(&mut t, (1000.0, 760.0), &th);
        {
            let v = t.v.borrow();
            assert_eq!(v.pv.width(), 0.0);
            assert!(!v.pv.exists(), "nothing is alive behind a hidden panel");
            assert!(!v.preview_wanted());
            let c = v.content_rect();
            assert!((c.w - (1000.0 - 2.0 - SIDE_W)).abs() < 0.01, "the page has all the room: {c:?}");
        }
        // widening shows it (animated), narrowing hides it again
        settle(&mut t, WIDE, &th);
        assert!(t.v.borrow().pv.exists() && t.v.borrow().pv.width() > PV_W - 0.5);
        let c = t.v.borrow().content_rect();
        assert!(c.w >= MIN_PAGE_W - 0.01, "the cards still fit next to the panel: {c:?}");
        settle(&mut t, (1000.0, 760.0), &th);
        assert!(t.v.borrow().pv.width() < 0.5 && !t.v.borrow().pv.exists());
        // exactly the threshold
        assert!(MIN_WINDOW_W > 1000.0 && MIN_WINDOW_W < 1200.0, "{MIN_WINDOW_W}");

    });
}

#[test]
fn the_switch_and_the_hide_button_turn_the_panel_off_and_it_is_saved() {
    gfx(|| {
        let _g = motion_on();
        let th = Theme::settings_dark();
        let mut t = wide("appearance", json!({"language": "en"}));
        assert!(t.v.borrow().pv.width() > PV_W - 0.5);
        let r = t.v.borrow().hit_rect(&Act::PvHide).expect("a Hide button");
        t.click_at(r);
        settle(&mut t, WIDE, &th);
        assert!(!t.v.borrow().model.get_bool("settings.preview"));
        assert!(t.calls("settings_set").iter().any(|a| a["patch"]["settings.preview"] == json!(false)), "{:?}", t.calls("settings_set"));
        assert_eq!(t.v.borrow().pv.width(), 0.0);
        assert!(!t.v.borrow().pv.exists(), "no sandbox, no threads");
        // back on from the row
        set(&mut t, "settings.preview", json!(true));
        settle(&mut t, WIDE, &th);
        assert!(t.v.borrow().pv.width() > PV_W - 0.5 && t.v.borrow().pv.exists());

    });
}

#[test]
fn replay_plays_the_reaction_again() {
    gfx(|| {
        let _g = motion_on();
        let th = Theme::settings_dark();
        let mut t = wide("live", json!({"language": "en"}));
        let frames = t.sim.frames_run;
        let r = t.v.borrow().hit_rect(&Act::PvReplay).expect("a Replay button");
        t.click_at(r);
        settle(&mut t, WIDE, &th);
        assert!(t.sim.frames_run > frames + 20, "the scene ran again");

    });
}

#[test]
fn once_settled_nothing_runs_and_the_sandbox_is_parked() {
    gfx(|| {
        let _g = motion_on();
        let th = Theme::settings_dark();
        // General has no animated picker of its own; the pet is the island's
        let mut t = wide("general", json!({"language": "en", "pet": "cat"}));
        set(&mut t, "showDate", json!(true));
        settle(&mut t, WIDE, &th);
        assert!(!t.sim.frame.wants_frame(), "the window is idle");
        let before = t.sim.frames_run;
        for _ in 0..20 {
            t.sim.now_ms += 500.0;
            t.sim.pump();
            t.sim.run(50);
        }
        assert_eq!(t.sim.frames_run, before, "no frames while nothing changes (a pet blinks only on a live island)");
        let v = t.v.borrow();
        assert!(!v.pv.active(), "parked");
        assert!(v.pv.live.as_ref().is_some_and(|l| !l.is_awake()), "its queue no longer wakes the window");

    });
}

/// Opaque black (island body) pixels in the upper and the lower half of the preview's stage.
fn stage_halves(t: &T, img: &Image) -> (usize, usize) {
    let p = pv_rect(WIDE);
    let h = t.v.borrow().pv.stage_h.at();
    let (x, y, w) = (p.x + 16.0, p.y + 48.0, PV_W - 32.0);
    let black = |r: Rect| img.count(Some(r), |px| px[3] == 255 && px[0].max(px[1]).max(px[2]) < 10);
    (black(Rect::new(x, y, w, h / 2.0)), black(Rect::new(x, y + h / 2.0, w, h / 2.0)))
}

#[test]
fn the_preview_island_stands_on_the_floor_of_its_stage_when_the_edge_is_the_bottom() {
    gfx(|| {
        let _g = motion_on();
        let th = Theme::settings_dark();
        let mut t = wide("appearance", json!({"language": "en"}));
        let a = shot(&mut t, "preview-edge-top-dark-1x", &th);
        let (up, down) = stage_halves(&t, &a);
        assert!(up > 600 && down * 3 < up, "the top island hangs from the ceiling of the stage: {up} / {down}");
        assert!(!t.v.borrow().pv.live.as_ref().unwrap().at_bottom());
        // the choice reaches the sandbox before it is saved; the island slides over and settles on the other edge
        set(&mut t, "edge.position", json!("bottom"));
        settle(&mut t, WIDE, &th);
        assert_eq!(t.v.borrow().pv.scene(), Scene::Pill(Look::Island), "the folded island is what shows it");
        assert!(t.v.borrow().pv.live.as_ref().unwrap().at_bottom());
        let b = shot(&mut t, "preview-edge-bottom-dark-1x", &th);
        let (up, down) = stage_halves(&t, &b);
        assert!(down > 600 && up * 3 < down, "the bottom island stands on the floor of the stage: {up} / {down}");
        assert!(t.calls("pill_edge").is_empty(), "the sandbox never moves the real island's window");
        // and back
        set(&mut t, "edge.position", json!("top"));
        settle(&mut t, WIDE, &th);
        let c = shot(&mut t, "preview-edge-top-again-dark-1x", &th);
        let (up, down) = stage_halves(&t, &c);
        assert!(up > 600 && down * 3 < up, "{up} / {down}");
        // the open island too: it grows upward from the floor
        set(&mut t, "edge.position", json!("bottom"));
        settle(&mut t, WIDE, &th);
        set(&mut t, "net.weather", json!(true));
        settle(&mut t, WIDE, &th);
        let d = shot(&mut t, "preview-edge-bottom-open-dark-1x", &th);
        assert_eq!(t.v.borrow().pv.scene(), Scene::Panel(Tab::Weather));
        let (up, down) = stage_halves(&t, &d);
        assert!(down > 1500 && up > 1500, "the open island fills the stage from the floor up: {up} / {down}");
        let fx = t.sim.take_fx();
        assert!(fx.edge_bottom.is_none(), "the sandbox's requests to the host are dropped: the real window is not told");
    });
}

#[test]
fn the_preview_island_stands_in_the_corner_of_its_stage_for_a_corner_alignment() {
    gfx(|| {
        let _g = motion_on();
        let th = Theme::settings_dark();
        let mut t = wide("appearance", json!({"language": "en", "edge.position": "bottom"}));
        let side = |t: &T, img: &Image| {
            let p = pv_rect(WIDE);
            let h = t.v.borrow().pv.stage_h.at();
            let (x, y, w) = (p.x + 16.0, p.y + 48.0, PV_W - 32.0);
            let black = |r: Rect| img.count(Some(r), |px| px[3] == 255 && px[0].max(px[1]).max(px[2]) < 10);
            (black(Rect::new(x, y, w / 4.0, h)), black(Rect::new(x + w * 0.75, y, w / 4.0, h)))
        };
        let a = shot(&mut t, "preview-align-centre-dark-1x", &th);
        let (l, r) = side(&t, &a);
        assert!(l > 100 && r > 100 && (l as f32 / r as f32 - 1.0).abs() < 0.25, "centred: {l} / {r}");
        set(&mut t, "edge.align", json!("left"));
        settle(&mut t, WIDE, &th);
        assert_eq!(t.v.borrow().pv.scene(), Scene::Pill(Look::Island));
        assert_eq!(t.v.borrow().pv.live.as_ref().unwrap().aligned(), crate::ui::host::geom::Align::Left);
        let b = shot(&mut t, "preview-align-left-dark-1x", &th);
        let (l, r) = side(&t, &b);
        assert!(l > 300 && r * 3 < l, "the island stands in the left corner of the stage: {l} / {r}");
        set(&mut t, "edge.align", json!("right"));
        settle(&mut t, WIDE, &th);
        let c = shot(&mut t, "preview-align-right-dark-1x", &th);
        let (l, r) = side(&t, &c);
        assert!(r > 300 && l * 3 < r, "and in the right one: {l} / {r}");
        assert!(t.calls("pill_edge").is_empty());
    });
}

#[test]
fn the_preview_island_stands_in_an_imaginary_taskbar_when_the_edge_is_the_taskbar() {
    gfx(|| {
        let _g = motion_on();
        let th = Theme::settings_dark();
        let mut t = wide("appearance", json!({"language": "en"}));
        assert!(!t.v.borrow().pv.live.as_ref().unwrap().in_bar());
        // the choice reaches the sandbox: the island slides over, then stands in the bar (the stage draws it under the island)
        set(&mut t, "edge.position", json!("taskbar"));
        settle(&mut t, WIDE, &th);
        {
            let v = t.v.borrow();
            let live = v.pv.live.as_ref().unwrap();
            assert!(live.at_bottom() && live.in_bar(), "the sandbox island has taken the bar");
            let (_, _, bh) = live.body();
            assert!(bh <= live::BAR_H - 8.0 + 0.6, "a capsule that fits the bar: {bh}");
        }
        assert!(t.calls("pill_edge").is_empty(), "the sandbox never moves the real island's window");
        // leaving the taskbar takes the bar away; a settings window that opens on the taskbar edge has it from the start
        set(&mut t, "edge.position", json!("bottom"));
        settle(&mut t, WIDE, &th);
        assert!(t.v.borrow().pv.live.as_ref().unwrap().at_bottom() && !t.v.borrow().pv.live.as_ref().unwrap().in_bar());
        let t2 = wide("appearance", json!({"language": "en", "edge.position": "taskbar"}));
        assert!(t2.v.borrow().pv.live.as_ref().unwrap().in_bar());
    });
}

#[test]
fn only_plain_choices_reach_the_preview_never_text_the_user_typed() {
    for k in ["showSeconds", "size", "shape", "theme", "pet", "character", "net.weather", "weather.units", "live.copy", "island.colorCustom", "clipClickPaste", "edge.position", "edge.align",
        "media.widget", "media.widget.edge", "media.widget.align", "media.widget.offset", "tabs.hidden", "home.volume", "home.tile.qr", "live.agents", "live.calendar"]
    {
        assert!(shareable(k), "{k} may go to the preview");
    }
    // where the island and the widget stand along their edge is the camera's business: the preview reads them, the sandbox never hears them
    for k in ["offsetX", "underTaskbar", "media.widget.offset"] {
        assert!(super::scene::handled(k), "{k} is shown by the camera");
    }
    for k in ["weather.city", "mail.user", "mail.host", "capture.folder", "capture.videoFolder", "hotkey.toggle", "search.customUrl", "clipboard.denyList", "hideInApps", "autoHide", "quiet.on",
        "monitor", "sound.volumeDevice", "settings.preview", "phone.downloadDir", "ai.baseUrl", "settingsWin.x", "nonsense", "offsetX", "underTaskbar"]
    {
        assert!(!shareable(k), "{k} stays in the window");
    }
}

#[test]
fn the_sandbox_answers_from_samples_and_never_calls_the_real_backend() {
    gfx(|| {
        let _g = motion_on();
        let th = Theme::settings_dark();
        let mut t = wide("clipboard", json!({"language": "en", "net.weather": true, "weather.city": "Realtown", "mail.user": "me@example.com"}));
        settle(&mut t, WIDE, &th);
        set(&mut t, "net.weather", json!(false));
        set(&mut t, "net.weather", json!(true));
        settle(&mut t, WIDE, &th);
        // the island of the preview asked for clipboard rows and a forecast: the window's backend (the fake api here) never heard of it
        for c in t.fakes.api.calls() {
            assert!(!["clip_list", "weather_get", "battery_state", "media_state", "launcher_list", "notif_list"].contains(&c.name.as_str()), "the real backend was called: {}", c.name);
        }
        // the sample forecast has an invented place and the sample clipboard invented lines
        let w = sample::weather(true);
        assert_eq!(w["city"], "Sample City");
        assert!(sample::clips(0).iter().all(|c| !c["preview"].as_str().unwrap_or("").contains("Realtown")));
        // and what the sandbox is told contains no text of the user's
        let snap = &t.v.borrow().pv.snap;
        assert!(!snap.contains_key("weather.city") && !snap.contains_key("mail.user"), "{:?}", snap.keys().collect::<Vec<_>>());

    });
}

#[test]
fn the_preview_takes_no_mouse_and_keys() {
    gfx(|| {
        let _g = motion_on();
        let th = Theme::settings_dark();
        let mut t = wide("appearance", json!({"language": "en"}));
        let r = pv_rect(WIDE);
        let before = t.calls("settings_set").len();
        let calls = t.fakes.api.calls().len();
        // click, wheel and hover on the stage: nothing happens to the model, the backend or the real island
        click(&mut t.sim, r.x + 150.0, r.y + 90.0);
        t.sim.send(&InputEvent::Wheel { pos: (r.x + 150.0, r.y + 90.0), delta: (0.0, -120.0), precise: false, mods: glasskit::input::Modifiers::NONE });
        settle(&mut t, WIDE, &th);
        assert_eq!(t.calls("settings_set").len(), before);
        assert_eq!(t.fakes.api.calls().len(), calls, "no backend call at all");
        assert_eq!(t.v.borrow().pv.scene(), Scene::DEFAULT);

    });
}

#[test]
fn every_reaction_has_a_scene_and_the_pages_a_start() {
    let keys = [
        ("live.copy", Flash::Copy),
        ("live.volume", Flash::Volume),
        ("live.notifications", Flash::Notif),
        ("live.battery", Flash::Battery),
        ("live.usb", Flash::Usb),
        ("live.dlDone", Flash::Download),
        ("live.reminders", Flash::Reminder),
        ("live.recording", Flash::Recording),
        ("live.music", Flash::Music),
        ("live.micCam", Flash::Privacy),
        ("live.keyboardLayout", Flash::Kbd),
        ("live.virtualDesktopFlash", Flash::Vdesk),
        ("cards.duration", Flash::Toast),
        ("live.capsLock", Flash::Caps),
        ("live.numLock", Flash::Num),
        ("live.btBattery", Flash::Bluetooth),
        ("notify.respectFocus", Flash::Focus),
    ];
    for (k, f) in keys {
        assert_eq!(of_key(k), Some(Scene::Flash(f)), "{k}");
        // each one has events to play and the switch it hangs on exists in the schema
        assert!(!sample::flash_events(f, false).is_empty(), "{k}");
        if let Some(sw) = f.switch() {
            assert!(pill_ui_core::settings::schema::schema().row_for_key(sw).is_some(), "{sw}");
        }
    }
    assert_eq!(of_key("module.weather"), Some(Scene::Panel(Tab::Home))); // the old key is still a Home-tab setting for the camera
    assert_eq!(of_key("showTemp"), Some(Scene::Pill(Look::Clock)));
    assert_eq!(of_key("capture.encoder"), None);
}

#[test]
fn the_indicator_scenes_draw_a_chip_on_the_folded_island() {
    gfx(|| {
        let _g = motion_on();
        let th = Theme::settings_dark();
        let mut t = wide("notifications", json!({"language": "en"}));
        let base = shot(&mut t, "preview-ind-base", &th);
        for (k, f) in [("live.capsLock", Flash::Caps), ("live.btBattery", Flash::Bluetooth)] {
            set(&mut t, k, json!(false));
            settle(&mut t, WIDE, &th);
            set(&mut t, k, json!(true));
            t.sim.pump();
            t.sim.run(60);
            let _ = t.sim.render(WIDE, &th);
            t.sim.run(60);
            let img = shot(&mut t, &format!("preview-ind-{k}"), &th);
            assert_eq!(t.v.borrow().pv.scene(), Scene::Flash(f), "{k}");
            assert!(img.diff(&base, 3) > 150, "{k}: the chip is on the island");
        }
    });
}

#[test]
fn every_flash_scene_draws_something_on_the_folded_island() {
    gfx(|| {
        let _g = motion_on();
        let th = Theme::settings_dark();
        let mut t = wide("live", json!({"language": "en"}));
        let base = shot(&mut t, "preview-base", &th);
        let mut changed = 0;
        for (k, f) in [("live.notifications", Flash::Notif), ("live.battery", Flash::Battery), ("live.usb", Flash::Usb), ("live.volume", Flash::Volume), ("live.copy", Flash::Copy), ("live.reminders", Flash::Reminder)] {
            eprintln!("FLASH {k}");
            // off then on: the second change plays the reaction
            set(&mut t, k, json!(false));
            settle(&mut t, WIDE, &th);
            set(&mut t, k, json!(true));
            t.sim.pump();
            t.sim.run(60); // mid-flash: the rows and cards live for seconds, the copy flash for 1.3 s
            let _ = t.sim.render(WIDE, &th);
            t.sim.run(60);
            let img = shot(&mut t, &format!("preview-flash-{k}"), &th);
            assert_eq!(t.v.borrow().pv.scene(), Scene::Flash(f), "{k}");
            if img.diff(&base, 3) > 300 {
                changed += 1;
            }
        }
        assert!(changed >= 4, "most reactions are visible in the panel: {changed} of 6");

    });
}

#[test]
fn the_caption_says_what_is_shown_and_when_the_switch_is_off() {
    let ru = pill_ui_core::settings::tr::Lang::Ru;
    let en = pill_ui_core::settings::tr::Lang::En;
    for sc in [Scene::DEFAULT, Scene::Panel(Tab::Weather), Scene::Panel(Tab::Clipboard), Scene::Flash(Flash::Copy), Scene::Pill(Look::Pet)] {
        for l in [ru, en] {
            assert!(!sc.title(l).is_empty() && !sc.note(l, false).is_empty() && !sc.note(l, true).is_empty());
        }
        assert_ne!(sc.title(ru), sc.title(en));
    }
    let _ = pal; // the colours come from the same palette as the page
}

/// What each function page starts the panel with: Weather -> the weather tab, Clipboard -> the clipboard tab, Notifications -> a notification, ...
#[test]
fn every_function_page_opens_the_matching_scene() {
    gfx(|| {
        let _g = motion_on();
        let cases = [
            ("look", Scene::Pill(Look::Island)),
            ("edge", Scene::Pill(Look::Edge)),
            ("performance", Scene::Pill(Look::Motion)),
            ("tabs", Scene::Panel(Tab::Home)),
            ("tools", Scene::Panel(Tab::Home)),
            ("weather", Scene::Panel(Tab::Weather)),
            ("clipboard", Scene::Panel(Tab::Clipboard)),
            ("notifications", Scene::Flash(Flash::Notif)),
            ("media", Scene::Flash(Flash::Music)),
            ("capture", Scene::Flash(Flash::Toast)),
            ("hotkeys", Scene::DEFAULT),
            ("phone", Scene::DEFAULT),
            ("calendar", Scene::DEFAULT),
            ("ai", Scene::DEFAULT),
        ];
        for (page, scene) in cases {
            assert_eq!(of_page(page), scene, "{page}");
        }
        // opened for real, and the old names land on the same scenes
        // (an old page name opens the page scrolled to its function, and the panel opens with that function's scene)
        for (page, scene) in [
            ("weather", Scene::Panel(Tab::Weather)),
            ("live", Scene::Flash(Flash::Notif)),
            ("sound", Scene::Flash(Flash::Volume)),
            ("modules", Scene::Panel(Tab::Home)),
            ("appearance", Scene::Pill(Look::Island)),
            ("pet", Scene::Pill(Look::Pet)),
            ("clock", Scene::Pill(Look::Clock)),
            ("animation", Scene::Pill(Look::Motion)),
            ("volume", Scene::Flash(Flash::Volume)),
            ("recording", Scene::Flash(Flash::Recording)),
            ("downloads", Scene::Flash(Flash::Progress)),
            ("battery", Scene::Flash(Flash::Battery)),
            ("notes", Scene::Panel(Tab::Home)),
        ] {
            let t = wide(page, json!({"language": "en"}));
            assert_eq!(t.v.borrow().pv.scene(), scene, "{page}");
        }
        // every page of the schema has a scene (or the default island): the lookup never panics
        for p in &pill_ui_core::settings::schema::schema().pages {
            let _ = of_page(p.id);
        }
    });
}

/// The seventeen pages of the sidebar, in order.
const PAGES16: [&str; 17] = ["look", "edge", "tabs", "weather", "media", "voice", "clipboard", "capture", "tools", "phone", "ai", "notifications", "calendar", "general", "performance", "hotkeys", "updates"];

fn pages_with_the_panel(lang: &str, themes: &[(&str, Theme)]) {
    const SIZE: (f32, f32) = (1180.0, 760.0);
    for (tn, th) in themes {
        for page in PAGES16 {
            let mut t = boot(json!({ "language": lang }), page);
            settle(&mut t, SIZE, th);
            assert!(t.v.borrow().pv.width() > PV_W - 0.5, "{page}: the panel is open at 1180");
            let c = t.v.borrow().content_rect();
            assert!(c.w >= MIN_PAGE_W - 0.01, "{page}: the cards fit next to the panel");
            let img = snap(&mut t.sim, &format!("fn-{page}-{lang}-{tn}-1180"), th, SIZE);
            assert!(img.count(None, |p| p[3] == 255) > 100_000, "{page} {tn}");
        }
    }
}

/// `PILL_SNAP_DIR=<dir> cargo test --release --locked -p pill the_function_pages_with_the_panel` writes the seventeen pages as PNGs (1180 x 760, English, dark and light).
#[test]
fn the_function_pages_with_the_panel() {
    gfx(|| {
        let _g = motion_on();
        pages_with_the_panel("en", &[("dark", Theme::settings_dark()), ("light", settings_light())]);
    });
}

/// Every page in one picture, as tall as it is (a 1180 x 2600 window: no scrolling; the page's real length shows), English dark; the Advanced
/// section opened. `PILL_SNAP_DIR=<dir> cargo test --release --locked -p pill the_whole_pages_tall`.
#[test]
fn the_whole_pages_tall() {
    gfx(|| {
        let _g = motion_on();
        let th = Theme::settings_dark();
        const SIZE: (f32, f32) = (1180.0, 2600.0);
        for page in PAGES16 {
            let mut t = boot(json!({"language": "en"}), page);
            if t.v.borrow().page.adv.is_some() {
                t.v.borrow_mut().set_advanced(true, true);
            }
            settle(&mut t, SIZE, &th);
            snap(&mut t.sim, &format!("tall-{page}-en-dark"), &th, SIZE);
        }
    });
}

/// The same in Russian (long titles), dark.
#[test]
fn the_function_pages_in_russian() {
    gfx(|| {
        let _g = motion_on();
        pages_with_the_panel("ru", &[("dark", Theme::settings_dark())]);
    });
}

/// Narrow window (no panel), double scale, the sidebar scrolled to its end, a closed and an open Advanced.
#[test]
fn the_function_pages_narrow_2x_scrolled_and_advanced() {
    gfx(|| {
        let _g = motion_on();
        let th = Theme::settings_dark();
        let mut t = boot(json!({"language": "ru"}), "look");
        settle(&mut t, (900.0, 760.0), &th);
        assert!(t.v.borrow().pv.width() < 0.5, "no panel in a 900 px window");
        snap(&mut t.sim, "fn-look-ru-900", &th, (900.0, 760.0));
        // the sidebar scrolled to the bottom: the last function is reachable and the group names stay readable
        t.v.borrow_mut().side.scroll.scroll_to(99999.0, false);
        settle(&mut t, (900.0, 760.0), &th);
        snap(&mut t.sim, "fn-sidebar-bottom-ru-900", &th, (900.0, 760.0));
        // Advanced closed, then open
        let mut t = boot(json!({"language": "en"}), "media");
        settle(&mut t, (1180.0, 760.0), &th);
        t.v.borrow_mut().page.scroll.scroll_to(99999.0, false);
        settle(&mut t, (1180.0, 760.0), &th);
        snap(&mut t.sim, "fn-media-adv-closed", &th, (1180.0, 760.0));
        t.v.borrow_mut().set_advanced(true, true);
        settle(&mut t, (1180.0, 760.0), &th);
        t.v.borrow_mut().page.scroll.scroll_to(99999.0, false);
        settle(&mut t, (1180.0, 760.0), &th);
        snap(&mut t.sim, "fn-media-adv-open", &th, (1180.0, 760.0));
        // double scale, light, a long Russian title
        let mut t = boot(json!({"language": "ru"}), "edge");
        t.sim.set_scale(2.0);
        let light = settings_light();
        settle(&mut t, (1180.0, 760.0), &light);
        snap(&mut t.sim, "fn-edge-ru-light-2x", &light, (1180.0, 760.0));
        let mut t = boot(json!({"language": "ru"}), "calendar");
        t.sim.set_scale(2.0);
        settle(&mut t, (1180.0, 760.0), &th);
        snap(&mut t.sim, "fn-calendar-ru-dark-2x", &th, (1180.0, 760.0));
    });
}

// ------------------------------------------------------------------------------------------------ sidebar icons and the General page

/// One PNG of `page`: `lang`, theme, scale, window size; the sidebar scrolled to its end and / or the page's Advanced open and scrolled down.
/// Run ONE of the `general_snap_*` tests per cargo invocation (PNG-writing tests in one process can crash the GPU thread).
#[allow(clippy::too_many_arguments)]
fn general_snap(name: &str, lang: &str, page: &str, th: Theme, scale: f32, size: (f32, f32), side_end: bool, adv: bool) -> Image {
    let mut t = boot(json!({ "language": lang }), page);
    t.sim.set_scale(scale);
    settle(&mut t, size, &th);
    if side_end {
        t.v.borrow_mut().side.scroll.scroll_to(99999.0, false);
    }
    if adv {
        t.v.borrow_mut().set_advanced(true, true);
        settle(&mut t, size, &th);
        t.v.borrow_mut().page.scroll.scroll_to(99999.0, false);
    }
    settle(&mut t, size, &th);
    snap(&mut t.sim, name, &th, size)
}

/// Every visible page item of the sidebar has ink in its icon box, and the box is where the text starts after it.
fn icons_are_drawn(t: &mut T, img: &Image) {
    let v = t.v.borrow();
    let (list, off) = (v.side.list, v.side.scroll.offset());
    let mut seen = 0;
    for n in pill_ui_core::settings::nav::NAV {
        let pill_ui_core::settings::nav::NavItem::Page(id) = n else { continue };
        let y = list.y - off + crate::ui::settings_view::chrome::Side::item_y(id);
        if y < list.y || y + 30.0 > list.bottom() {
            continue;
        }
        let ib = Rect::new(list.x + 10.0 + 14.0, y + 7.0, 16.0, 16.0);
        let bg = img.at(ib.x - 4.0, ib.cy());
        let ink = img.count(Some(ib), |p| p.iter().zip(bg).any(|(a, b)| a.abs_diff(b) > 40));
        assert!(ink >= 12, "{id}: its icon is drawn ({ink} px of ink)");
        seen += 1;
    }
    assert!(seen >= 10, "{seen} items were checked");
}

#[test]
fn general_snap_sidebar_top_ru_dark_900() {
    gfx(|| {
        let _g = motion_on();
        let mut t = boot(json!({ "language": "ru" }), "general");
        let th = Theme::settings_dark();
        settle(&mut t, (900.0, 760.0), &th);
        let img = snap(&mut t.sim, "icons-sidebar-top-ru-dark-900", &th, (900.0, 760.0));
        icons_are_drawn(&mut t, &img);
    });
}

#[test]
fn general_snap_sidebar_bottom_ru_dark_900() {
    gfx(|| {
        let _g = motion_on();
        let mut t = boot(json!({ "language": "ru" }), "updates");
        let th = Theme::settings_dark();
        settle(&mut t, (900.0, 760.0), &th);
        t.v.borrow_mut().side.scroll.scroll_to(99999.0, false);
        settle(&mut t, (900.0, 760.0), &th);
        let img = snap(&mut t.sim, "icons-sidebar-bottom-ru-dark-900", &th, (900.0, 760.0));
        icons_are_drawn(&mut t, &img);
    });
}

#[test]
fn general_snap_sidebar_top_en_light_1180() {
    gfx(|| {
        let _g = motion_on();
        let mut t = boot(json!({ "language": "en" }), "hotkeys");
        let th = settings_light();
        settle(&mut t, (1180.0, 760.0), &th);
        let img = snap(&mut t.sim, "icons-sidebar-top-en-light-1180", &th, (1180.0, 760.0));
        icons_are_drawn(&mut t, &img);
    });
}

#[test]
fn general_snap_sidebar_bottom_en_light_2x() {
    gfx(|| {
        let _g = motion_on();
        general_snap("icons-sidebar-bottom-en-light-2x", "en", "ai", settings_light(), 2.0, (1180.0, 760.0), true, false);
    });
}

#[test]
fn general_snap_page_en_dark_1180() {
    gfx(|| {
        let _g = motion_on();
        let img = general_snap("general-en-dark-1180", "en", "general", Theme::settings_dark(), 1.0, (1180.0, 760.0), false, false);
        assert!(img.count(None, |p| p[3] == 255) > 100_000);
    });
}

#[test]
fn general_snap_page_ru_light_900() {
    gfx(|| {
        let _g = motion_on();
        general_snap("general-ru-light-900", "ru", "general", settings_light(), 1.0, (900.0, 760.0), false, false);
    });
}

#[test]
fn general_snap_page_en_light_2x_1180() {
    gfx(|| {
        let _g = motion_on();
        general_snap("general-en-light-2x-1180", "en", "general", settings_light(), 2.0, (1180.0, 760.0), false, false);
    });
}

#[test]
fn general_snap_advanced_ru_dark_1180() {
    gfx(|| {
        let _g = motion_on();
        general_snap("general-adv-ru-dark-1180", "ru", "general", Theme::settings_dark(), 1.0, (1180.0, 760.0), false, true);
    });
}

#[test]
fn general_page_holds_the_general_settings_and_opens_the_folded_island() {
    gfx(|| {
        let _g = motion_on();
        let t = boot(json!({ "language": "en" }), "quiet");
        assert_eq!(t.current(), "general", "an old page id opens General");
        assert_eq!(of_page("general"), Scene::Pill(Look::Island));
        assert_eq!(t.v.borrow().pv.scene(), Scene::Pill(Look::Island));
        // the sections, in the order the page promises
        let sc = pill_ui_core::settings::schema::schema();
        let g = sc.page("general").unwrap();
        let titles: Vec<&str> = g.groups.iter().map(|g| g.title.en).collect();
        assert_eq!(
            titles,
            ["Start and language", "Hiding the pill", "Quiet hours", "Command bar search", "Settings window", "Details and program lists", "Pass-through mode", "File search: Everything", "Settings backup", "First run"]
        );
        assert!(g.groups[..5].iter().all(|g| !g.advanced) && g.groups[5..].iter().all(|g| g.advanced));
    });
}

#[test]
fn general_snap_page_end_closed_en_dark_900() {
    gfx(|| {
        let _g = motion_on();
        let th = Theme::settings_dark();
        let mut t = boot(json!({ "language": "en" }), "general");
        settle(&mut t, (900.0, 760.0), &th);
        t.v.borrow_mut().page.scroll.scroll_to(99999.0, false);
        settle(&mut t, (900.0, 760.0), &th);
        snap(&mut t.sim, "general-end-closed-en-dark-900", &th, (900.0, 760.0));
    });
}


/// Settings that show nothing in the preview, and why. The test below holds every other keyed row of the schema to a scene, so a new visual
/// setting cannot be added without either wiring it (`scene::of_key` + `scene::shareable` / `CAMERA`) or being listed here with its reason.
const NO_PREVIEW: &[(&str, &str)] = &[
    // text the user typed or a device: never handed to the sandbox, and a sample stands in where it would show (the tab has an invented city)
    ("weather.city", "typed text; the weather tab shows an invented city"),
    ("clipboard.denyList", "typed list of programs"),
    ("capture.folder", "a folder"),
    ("capture.videoFolder", "a folder"),
    ("timer.soundFile", "a file"),
    ("phone.downloadDir", "a folder"),
    ("phone.apkUrl", "an address"),
    ("phone.relayUrl", "an address"),
    ("search.customUrl", "an address"),
    ("ai.baseUrl", "an address"),
    ("ai.provider", "which AI service"),
    ("ai.model", "a model name"),
    ("voice.provider", "which recognition service"),
    ("voice.endpoint", "an address"),
    ("voice.model", "a model name"),
    ("sound.volumeDevice", "a sound device"),
    ("sound.micDevice", "a sound device"),
    ("mail.host", "an address"),
    ("mail.port", "a port"),
    ("mail.user", "a user name"),
    ("mail.mailbox", "a folder of the mail account"),
    ("live.calendar.path", "a file"),
    ("live.obs.port", "a port"),
    ("live.dlIgnoreExt", "typed list of extensions"),
    ("vpn.own.name", "typed text"),
    ("vpn.own.connect", "typed command"),
    ("vpn.own.disconnect", "typed command"),
    ("hideInApps", "list of programs"),
    ("showInApps", "list of programs"),
    ("aiSetupDone", "a flag of the local AI set-up"),
    // which monitor: the preview has no real screens
    ("monitor", "which monitor the island is on"),
    // behaviour that would hide, mute or fade the preview itself (BLOCKED)
    ("autoHide", "would hide the preview island"),
    ("autoHideKeepDesktop", "would hide the preview island"),
    ("hideInFullscreen", "depends on the user's programs"),
    ("fullscreenPeek", "depends on the user's programs"),
    ("fullscreenPeekEdge", "depends on the user's programs"),
    ("passThrough", "click-through: the preview takes no clicks anyway"),
    ("passThroughOpacity", "click-through: the preview takes no clicks anyway"),
    ("showInCapture", "whether screenshots of the screen show the island"),
    ("quiet.on", "quiet hours would mute the sample reactions"),
    ("command.shortcuts", "the command bar is its own window: its prefixes show nowhere on the island"),
    ("edge.monitor", "which monitor the island stands on: the preview has one sandbox screen"),
    ("energy.saver", "an energy rule: it would slow or pause the preview itself"),
    ("energy.battery", "an energy rule: it would slow or pause the preview itself"),
    ("energy.pauseLocked", "an energy rule: it would slow or pause the preview itself"),
    ("energy.pauseDisplayOff", "an energy rule: it would slow or pause the preview itself"),
    ("energy.pauseRdp", "an energy rule: it would slow or pause the preview itself"),
    ("quiet.from", "quiet hours"),
    ("quiet.to", "quiet hours"),
    ("quiet.days", "quiet hours"),
    ("settings.preview", "the switch of the preview itself"),
    // behaviour without a look
    ("ui.gpu", "which renderer draws the windows"),
    ("live.sysmon", "the load chip needs ten seconds of sustained load and then rests five minutes, and the meters are in the Controls tab"),
    ("audio.perApp", "which sources the player list reads"),
    ("media.legacyPlayers", "which players are read"),
    ("capture.annotate", "opens the editor after a screenshot"),
    ("capture.encoder", "which video encoder"),
    ("timer.sound", "the sound of a timer"),
    ("vpn.admin", "how the VPN client is started"),
    ("history.mail", "the notification history is not a preview tab"),
    ("search.engine", "the command bar is a window of its own"),
    ("everything.autostart", "starts a program"),
    ("updateAuto", "updates"),
    ("whatsNew", "a window that opens after an update"),
    ("net.ai", "a privacy gate of the chat"),
    ("voice.enabled", "dictation: the header button of the open island is not in the preview"),
    ("voice.mode", "dictation"),
    ("voice.language", "dictation"),
    ("voice.paste", "dictation"),
    ("voice.autostop", "dictation"),
    ("phone.enabled", "the link itself: the phone's cards are `live.phone`"),
    ("phone.clipboard", "phone clipboard sync"),
    ("phone.clipToPhone", "phone clipboard sync"),
    ("phone.clipFromPhone", "phone clipboard sync"),
    ("phone.autoAccept", "phone files"),
    ("phone.relay", "phone relay"),
    ("pc.enabled", "the PC link: no island scene"),
    ("phone.maxFileMB", "phone files"),
    ("phone.relayMaxFileMB", "phone relay"),
];

#[test]
fn every_visual_setting_is_shown_by_the_preview_or_listed_as_having_nothing_to_show() {
    let sc = pill_ui_core::settings::schema::schema();
    let (mut unlisted, mut both) = (Vec::new(), Vec::new());
    let mut keys = std::collections::BTreeSet::new();
    for p in &sc.pages {
        for r in p.rows().filter(|r| !r.key.is_empty()) {
            keys.insert(r.key);
            let shown = super::scene::handled(r.key) && of_key(r.key).is_some();
            let listed = NO_PREVIEW.iter().any(|(k, _)| *k == r.key) || r.key.starts_with("hotkey.");
            match (shown, listed) {
                (false, false) => unlisted.push(format!("{} ({})", r.key, p.id)),
                (true, true) => both.push(r.key),
                _ => {}
            }
        }
    }
    assert!(unlisted.is_empty(), "settings the preview does not show and nobody explained (wire them in preview/scene.rs or list them in NO_PREVIEW): {unlisted:?}");
    assert!(both.is_empty(), "shown by the preview but still listed as having nothing to show: {both:?}");
    // the list holds no key that is gone
    let gone: Vec<_> = NO_PREVIEW.iter().map(|(k, _)| *k).filter(|k| !keys.contains(k)).collect();
    assert!(gone.is_empty(), "NO_PREVIEW lists keys the schema does not have: {gone:?}");
    // the Home chips and the tab strip write keys that no schema row has: the preview takes them too
    for part in pill_ui_core::panel::a::home_mods::PARTS.iter() {
        assert!(super::scene::handled(part.key) && of_key(part.key) == Some(Scene::Panel(Tab::Home)), "{}", part.key);
    }
    assert!(super::scene::handled("tabs.hidden") && of_key("tabs.hidden") == Some(Scene::Panel(Tab::Home)));
}

#[test]
fn every_section_of_every_page_brings_something_into_view_or_has_nothing_to_show() {
    let sc = pill_ui_core::settings::schema::schema();
    for p in &sc.pages {
        for g in &p.groups {
            let keyed: Vec<_> = g.rows.iter().filter(|r| !r.key.is_empty()).collect();
            if keyed.is_empty() {
                continue; // buttons and the page's own widgets
            }
            let shows = keyed.iter().any(|r| super::scene::handled(r.key) && of_key(r.key).is_some());
            let silent = keyed.iter().all(|r| NO_PREVIEW.iter().any(|(k, _)| *k == r.key) || r.key.starts_with("hotkey."));
            assert!(shows || silent, "section {:?} of {} has rows nobody explained", g.title.en, p.id);
        }
    }
}

#[test]
fn every_reaction_is_wired_to_a_key_a_switch_and_events() {
    let sc = pill_ui_core::settings::schema::schema();
    for f in Flash::ALL {
        assert!(!sample::flash_events(f, false).is_empty(), "{f:?} has events to play");
        let title = Scene::Flash(f).title(pill_ui_core::settings::tr::Lang::En);
        assert!(!title.is_empty(), "{f:?}");
        // some row of the schema brings this reaction up
        assert!(sc.pages.iter().flat_map(|p| p.rows()).any(|r| of_key(r.key) == Some(Scene::Flash(f))), "{f:?} is reached from no setting");
        if let Some(sw) = f.switch() {
            assert!(sc.row_for_key(sw).is_some_and(|r| of_key(r.key) == Some(Scene::Flash(f))), "{sw} brings up {f:?}");
        }
    }
}

// ------------------------------------------------------------------------------------------------ the music widget and the island's place

/// The stage of the panel on the window's pixels.
fn stage_of(t: &T) -> Rect {
    let p = pv_rect(WIDE);
    Rect::new(p.x + 16.0, p.y + 48.0, PV_W - 32.0, t.v.borrow().pv.stage_h.at())
}

/// Opaque black (the body of an island) or the widget's body inside `r`: the folded capsule runs from the cover's deep tone into near black, and the
/// sandbox's track has no cover (its accent, a blue, tints it), so the saturated blues of that ramp count too (the stage's own backdrop is another colour);
/// the open card is near black (the island's card colour, #0f0f0f).
fn black_in(img: &Image, r: Rect) -> usize {
    // the ramp's blues are saturated and darker than any backdrop of the sketch (about rgb(22, 98, 169) down to rgb(6, 25, 44))
    img.count(Some(r), |px| px[3] == 255 && (px[0].max(px[1]).max(px[2]) < 20 || (px[2] as i32 - px[0] as i32 >= 36 && px[1] as i32 - px[0] as i32 >= 14)))
}

/// The black of the stage in its four quarters: (top left, top right, bottom left, bottom right).
fn quarters(t: &T, img: &Image) -> (usize, usize, usize, usize) {
    let s = stage_of(t);
    let (w, h) = (s.w / 2.0, s.h / 2.0);
    (black_in(img, Rect::new(s.x, s.y, w, h)), black_in(img, Rect::new(s.x + w, s.y, w, h)), black_in(img, Rect::new(s.x, s.y + h, w, h)), black_in(img, Rect::new(s.x + w, s.y + h, w, h)))
}

#[test]
fn the_music_widget_stands_on_the_sketch_of_the_monitor_where_the_settings_put_it() {
    gfx(|| {
        let _g = motion_on();
        let th = Theme::settings_dark();
        // the island at the top in the middle, the widget at the bottom left (the defaults of the widget)
        let mut t = wide("media", json!({"language": "en", "media.widget": true}));
        assert_eq!(t.v.borrow().pv.scene(), Scene::Widget, "the music reaction is the widget while the widget is on");
        {
            let v = t.v.borrow();
            let g = v.pv.live.as_ref().unwrap().player().expect("the widget is in the sandbox");
            assert_eq!((g.dock, g.align), (Dock::Bottom, Side::Left));
            assert!(v.pv.mode.at01() > 0.99, "the camera looks at the sketch of the monitor");
        }
        let a = shot(&mut t, "widget-bottom-left", &th);
        let (tl, tr, bl, br) = quarters(&t, &a);
        assert!(bl > 150 && br * 6 < bl, "the capsule stands in the bottom left corner: {bl} / {br}");
        assert!(tl + tr > 20, "the island hangs from the top: {tl} {tr}");
        // the other side, the other edge
        set(&mut t, "media.widget.align", json!("right"));
        settle(&mut t, WIDE, &th);
        let b = shot(&mut t, "widget-bottom-right", &th);
        let (_, _, bl, br) = quarters(&t, &b);
        assert!(br > 150 && bl * 6 < br, "and in the bottom right one: {bl} / {br}");
        set(&mut t, "media.widget.edge", json!("top"));
        settle(&mut t, WIDE, &th);
        let c = shot(&mut t, "widget-top-right", &th);
        let (tl, tr, bl, br) = quarters(&t, &c);
        assert!(tr > 150 && bl + br < 100 && tl < tr, "at the top of the screen now: {tl} {tr} {bl} {br}");
        // the middle, and shifted along the edge
        set(&mut t, "media.widget.align", json!("center"));
        settle(&mut t, WIDE, &th);
        let img = shot(&mut t, "widget-top-centre", &th);
        let (tl, tr, ..) = quarters(&t, &img);
        assert!(tl > 100 && tr > 100, "the capsule sits across the middle: {tl} {tr}");
        set(&mut t, "media.widget.offset", json!(30));
        settle(&mut t, WIDE, &th);
        let d = shot(&mut t, "widget-top-centre-shifted", &th);
        let (tl2, tr2, ..) = quarters(&t, &d);
        // (the left count is not compared: the widget's tinted body over the black island hid part of it, and it shows again when the widget moves away)
        assert!(tr2 > tr + 60, "30 % of the screen to the right: {tl} {tr} -> {tl2} {tr2}");
        // the bigger the size, the bigger the widget (it follows the island's size)
        set(&mut t, "media.widget.offset", json!(0));
        settle(&mut t, WIDE, &th);
        let img = shot(&mut t, "widget-top-centre-100", &th);
        let (a1, a2, ..) = quarters(&t, &img);
        set(&mut t, "size", json!(200));
        settle(&mut t, WIDE, &th);
        let img = shot(&mut t, "widget-top-centre-200", &th);
        let (b1, b2, ..) = quarters(&t, &img);
        assert!(b1 + b2 > (a1 + a2) * 2, "twice the size: {} -> {}", a1 + a2, b1 + b2);
        assert!(t.calls("pill_edge").is_empty() && t.calls("settings_set").len() == 0, "nothing of it reaches the real windows");
    });
}

#[test]
fn the_music_widget_in_the_taskbar_goes_when_it_is_off_and_steps_aside_for_an_island_in_the_bar() {
    gfx(|| {
        let _g = motion_on();
        let th = Theme::settings_dark();
        let mut t = wide("media", json!({"language": "en", "media.widget": true, "media.widget.edge": "taskbar", "media.widget.align": "left"}));
        {
            let v = t.v.borrow();
            let live = v.pv.live.as_ref().unwrap();
            assert!(live.player_in_bar() && live.player().unwrap().dock == Dock::Taskbar);
            assert!(v.pv.strip.at01() > 0.99, "the taskbar is on the stage");
        }
        let a = shot(&mut t, "widget-taskbar-left", &th);
        let (_, _, bl, br) = quarters(&t, &a);
        assert!(bl > 100 && br < bl / 3, "the capsule is inside the strip at the left: {bl} / {br}");
        // the island stands in the bar at the left too: the widget is put at the right
        let mut t2 = wide("media", json!({"language": "en", "media.widget": true, "media.widget.edge": "taskbar", "media.widget.align": "left", "edge.position": "taskbar", "edge.align": "left"}));
        assert_eq!(t2.v.borrow().pv.live.as_ref().unwrap().player().unwrap().align, Side::Right);
        let b = shot(&mut t2, "widget-taskbar-clear-of-the-island", &th);
        let (_, _, bl, br) = quarters(&t2, &b);
        assert!(bl > 20 && br > 100, "the island at the left, the widget at the right: {bl} / {br}");
        // the switch off: the widget slides out (the caption says it is off); on again, it comes back
        set(&mut t, "media.widget", json!(false));
        settle(&mut t, WIDE, &th);
        let c = shot(&mut t, "widget-switched-off", &th);
        let (_, _, bl, _) = quarters(&t, &c);
        assert!(bl < 40, "the widget has slid out: {bl}");
        set(&mut t, "media.widget", json!(true));
        settle(&mut t, WIDE, &th);
        let d = shot(&mut t, "widget-switched-on-again", &th);
        let (_, _, bl, _) = quarters(&t, &d);
        assert!(bl > 100, "and slides in again: {bl}");
    });
}

#[test]
fn the_island_stands_where_offset_and_the_taskbar_rule_put_it_on_the_sketch() {
    gfx(|| {
        let _g = motion_on();
        let th = Theme::settings_dark();
        let mut t = wide("look", json!({"language": "en", "edge.position": "bottom"}));
        set(&mut t, "offsetX", json!(10));
        settle(&mut t, WIDE, &th);
        set(&mut t, "offsetX", json!(0));
        settle(&mut t, WIDE, &th);
        assert_eq!(t.v.borrow().pv.scene(), Scene::Pill(Look::Place), "a position row shows where the island stands on the screen");
        let (lo, hi) = {
            let s = stage_of(&t);
            let a = shot(&mut t, "place-above-the-taskbar", &th);
            // the rows of the island's body from the floor up: it stands above the strip
            let strip = live::BAR_H * t.v.borrow().pv.zoom.at();
            (black_in(&a, Rect::new(s.x, s.bottom() - strip, s.w, strip)), black_in(&a, Rect::new(s.x, s.bottom() - strip - 14.0, s.w, 14.0)))
        };
        assert!(hi > 100 && lo < hi / 2, "above the taskbar, not on it: {hi} over / {lo} on");
        set(&mut t, "underTaskbar", json!(false));
        settle(&mut t, WIDE, &th);
        let s = stage_of(&t);
        let b = shot(&mut t, "place-over-the-taskbar", &th);
        let strip = live::BAR_H * t.v.borrow().pv.zoom.at();
        let on = black_in(&b, Rect::new(s.x, s.bottom() - strip, s.w, strip));
        assert!(on > lo + 80, "at the very edge, over the bar: {on}");
        // the shift along the edge moves a centred island, and not a corner one
        let (l0, r0) = (black_in(&b, Rect::new(s.x, s.y, s.w / 2.0, s.h)), black_in(&b, Rect::new(s.x + s.w / 2.0, s.y, s.w / 2.0, s.h)));
        set(&mut t, "offsetX", json!(-40));
        settle(&mut t, WIDE, &th);
        let c = shot(&mut t, "place-shifted-left", &th);
        let (l1, r1) = (black_in(&c, Rect::new(s.x, s.y, s.w / 2.0, s.h)), black_in(&c, Rect::new(s.x + s.w / 2.0, s.y, s.w / 2.0, s.h)));
        assert!(l1 > l0 + 30 && r1 < r0, "40 % to the left: {l0} {r0} -> {l1} {r1}");
    });
}

/// How many pixels of the stage differ between two shots (the caption under it changes with every scene, so the whole window cannot tell).
fn stage_diff(t: &T, a: &Image, b: &Image) -> usize {
    let s = stage_of(t);
    let mut n = 0;
    for y in s.y as i32..(s.y + s.h) as i32 {
        for x in s.x as i32..(s.x + s.w) as i32 {
            let (p, q) = (a.at(x as f32, y as f32), b.at(x as f32, y as f32));
            if (0..3).any(|i| p[i].abs_diff(q[i]) > 3) {
                n += 1;
            }
        }
    }
    n
}

/// The reactions that had no scene before the preview showed them: each one is switched off and on in the window and plays with invented data
/// (a download, a game, a file on the shelf, an agent, a phone message, a meeting, a mail, an OBS recording).
#[test]
fn the_other_live_events_play_on_the_folded_island_with_sample_data() {
    gfx(|| {
        let _g = motion_on();
        let th = Theme::settings_dark();
        let mut t = wide("look", json!({"language": "en"}));
        let mut weak = Vec::new();
        for (k, f) in [
            ("live.downloads", Flash::Progress),
            ("live.steam", Flash::Steam),
            ("live.shelf", Flash::Shelf),
            ("live.agents", Flash::Agents),
            ("live.phone", Flash::Phone),
            ("live.calendar", Flash::Calendar),
            ("live.mail", Flash::Mail),
            ("live.obs", Flash::Obs),
        ] {
            set(&mut t, k, json!(false));
            settle(&mut t, WIDE, &th);
            let base = shot(&mut t, &format!("preview-live-{k}-off"), &th);
            set(&mut t, k, json!(true));
            t.sim.pump();
            t.sim.run(60);
            let _ = t.sim.render(WIDE, &th);
            t.sim.run(500);
            let img = shot(&mut t, &format!("preview-live-{k}"), &th);
            assert_eq!(t.v.borrow().pv.scene(), Scene::Flash(f), "{k}");
            let d = stage_diff(&t, &img, &base);
            eprintln!("LIVE {k}: {d}");
            if d < 120 {
                weak.push((k, d));
            }
        }
        assert!(weak.is_empty(), "these reactions put nothing on the island: {weak:?}");
    });
}

#[test]
fn the_music_widget_costs_nothing_once_it_has_settled_and_respects_reduce_motion() {
    gfx(|| {
        let _g = motion_on();
        let th = Theme::settings_dark();
        let mut t = wide("media", json!({"language": "en", "media.widget": true}));
        assert_eq!(t.v.borrow().pv.scene(), Scene::Widget);
        settle(&mut t, WIDE, &th);
        assert!(!t.sim.frame.wants_frame(), "the window is idle");
        let before = t.sim.frames_run;
        for _ in 0..20 {
            t.sim.now_ms += 500.0;
            t.sim.pump();
            t.sim.run(50);
        }
        assert_eq!(t.sim.frames_run, before, "no frames, and the widget's one-second tick has stopped with the park");
        {
            let v = t.v.borrow();
            assert!(!v.pv.active() && v.pv.live.as_ref().is_some_and(|l| !l.is_awake() && l.player().is_some()), "parked, the widget still on the stage");
        }
        // a change wakes it again and it parks again
        set(&mut t, "media.widget.align", json!("right"));
        settle(&mut t, WIDE, &th);
        assert!(!t.v.borrow().pv.active());
        // reduced motion: the camera and the shift stand at their targets without a spring
        glasskit::ease::force_reduced(Some(true));
        set(&mut t, "reduceMotion", json!(true));
        t.sim.cx.reduced_motion = true;
        set(&mut t, "media.widget.offset", json!(20));
        set(&mut t, "media.widget.align", json!("center"));
        for _ in 0..3 {
            t.sim.pump();
            t.sim.run(3);
            let _ = t.sim.render(WIDE, &th);
        }
        {
            let v = t.v.borrow();
            assert!(!v.pv.mode.moving() && !v.pv.dx_w.moving() && !v.pv.lift_w.moving() && !v.pv.strip.moving(), "reduced motion: nothing glides");
        }
        glasskit::ease::force_reduced(None);
    });
}

/// What is on Home follows the chips of "What is on Home" and the tab strip at once (they write keys no schema row has).
#[test]
fn the_home_chips_and_the_tab_strip_change_the_open_island_in_the_preview() {
    gfx(|| {
        let _g = motion_on();
        let th = Theme::settings_dark();
        let mut t = wide("tabs", json!({"language": "en"}));
        assert_eq!(t.v.borrow().pv.scene(), Scene::Panel(Tab::Home));
        let a = shot(&mut t, "preview-home", &th);
        for (k, v) in [("home.volume", json!(false)), ("home.tile.qr", json!(false)), ("tabs.hidden", json!(["weather", "chat"]))] {
            let before = shot(&mut t, "preview-home-before", &th);
            set(&mut t, k, v);
            settle(&mut t, WIDE, &th);
            let after = shot(&mut t, &format!("preview-home-{k}"), &th);
            assert_eq!(t.v.borrow().pv.scene(), Scene::Panel(Tab::Home), "{k}");
            assert!(stage_diff(&t, &after, &before) > 60, "{k} changes the open island: {}", stage_diff(&t, &after, &before));
        }
        let end = shot(&mut t, "preview-home-end", &th);
        assert!(stage_diff(&t, &end, &a) > 200);
    });
}
