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
        set(&mut t, "capture.fps", json!(60)); // a setting with nothing to show
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

#[test]
fn only_plain_choices_reach_the_preview_never_text_the_user_typed() {
    for k in ["showSeconds", "size", "shape", "theme", "pet", "character", "net.weather", "weather.units", "live.copy", "island.colorCustom", "clipClickPaste"] {
        assert!(shareable(k), "{k} may go to the preview");
    }
    for k in ["weather.city", "mail.user", "mail.host", "capture.folder", "capture.videoFolder", "hotkey.toggle", "search.customUrl", "clipboard.denyList", "hideInApps", "autoHide", "quiet.on",
        "monitor", "sound.volumeDevice", "settings.preview", "phone.downloadDir", "ai.baseUrl", "settingsWin.x", "nonsense"]
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
    ];
    for (k, f) in keys {
        assert_eq!(of_key(k), Some(Scene::Flash(f)), "{k}");
        // each one has events to play and the switch it hangs on exists in the schema
        assert!(!sample::flash_events(f, false).is_empty(), "{k}");
        if let Some(sw) = f.switch() {
            assert!(pill_ui_core::settings::schema::schema().row_for_key(sw).is_some(), "{sw}");
        }
    }
    assert_eq!(of_key("module.weather"), Some(Scene::Panel(Tab::Home)));
    assert_eq!(of_key("showTemp"), Some(Scene::Pill(Look::Clock)));
    assert_eq!(of_key("capture.encoder"), None);
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
            ("pet", Scene::Pill(Look::Pet)),
            ("clock", Scene::Pill(Look::Clock)),
            ("animation", Scene::Pill(Look::Motion)),
            ("tabs", Scene::Panel(Tab::Home)),
            ("weather", Scene::Panel(Tab::Weather)),
            ("clipboard", Scene::Panel(Tab::Clipboard)),
            ("notifications", Scene::Flash(Flash::Notif)),
            ("media", Scene::Flash(Flash::Music)),
            ("volume", Scene::Flash(Flash::Volume)),
            ("recording", Scene::Flash(Flash::Recording)),
            ("downloads", Scene::Flash(Flash::Download)),
            ("hotkeys", Scene::DEFAULT),
            ("phone", Scene::DEFAULT),
        ];
        for (page, scene) in cases {
            assert_eq!(of_page(page), scene, "{page}");
        }
        // opened for real, and the old names land on the same scenes
        for (page, scene) in [("weather", Scene::Panel(Tab::Weather)), ("live", Scene::Flash(Flash::Notif)), ("sound", Scene::Flash(Flash::Volume)), ("modules", Scene::Panel(Tab::Home)), ("appearance", Scene::Pill(Look::Island))] {
            let t = wide(page, json!({"language": "en"}));
            assert_eq!(t.v.borrow().pv.scene(), scene, "{page}");
        }
        // every page of the schema has a scene (or the default island): the lookup never panics
        for p in &pill_ui_core::settings::schema::schema().pages {
            let _ = of_page(p.id);
        }
    });
}

/// `PILL_SNAP_DIR=<dir> cargo test --release --locked -p pill the_function_pages_with_the_panel` writes the pages as PNGs (1180 x 760, dark and light).
#[test]
fn the_function_pages_with_the_panel() {
    gfx(|| {
        let _g = motion_on();
        const SIZE: (f32, f32) = (1180.0, 760.0);
        for (tn, th) in [("dark", Theme::settings_dark()), ("light", settings_light())] {
            for page in ["look", "edge", "pet", "clock", "weather", "clipboard", "notifications", "phone", "voice", "hotkeys", "updates"] {
                let mut t = boot(json!({"language": "en"}), page);
                settle(&mut t, SIZE, &th);
                assert!(t.v.borrow().pv.width() > PV_W - 0.5, "{page}: the panel is open at 1180");
                let c = t.v.borrow().content_rect();
                assert!(c.w >= MIN_PAGE_W - 0.01, "{page}: the cards fit next to the panel");
                let img = snap(&mut t.sim, &format!("fn-{page}-{tn}-1180"), &th, SIZE);
                assert!(img.count(None, |p| p[3] == 255) > 100_000, "{page} {tn}");
            }
        }
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
        let mut t = boot(json!({"language": "en"}), "voice");
        settle(&mut t, (1180.0, 760.0), &th);
        t.v.borrow_mut().page.scroll.scroll_to(99999.0, false);
        settle(&mut t, (1180.0, 760.0), &th);
        snap(&mut t.sim, "fn-voice-adv-closed", &th, (1180.0, 760.0));
        t.v.borrow_mut().set_advanced(true, true);
        settle(&mut t, (1180.0, 760.0), &th);
        t.v.borrow_mut().page.scroll.scroll_to(99999.0, false);
        settle(&mut t, (1180.0, 760.0), &th);
        snap(&mut t.sim, "fn-voice-adv-open", &th, (1180.0, 760.0));
        // double scale, light, a long Russian title
        let mut t = boot(json!({"language": "ru"}), "edge");
        t.sim.set_scale(2.0);
        let light = settings_light();
        settle(&mut t, (1180.0, 760.0), &light);
        snap(&mut t.sim, "fn-edge-ru-light-2x", &light, (1180.0, 760.0));
        let mut t = boot(json!({"language": "ru"}), "mail");
        t.sim.set_scale(2.0);
        settle(&mut t, (1180.0, 760.0), &th);
        snap(&mut t.sim, "fn-mail-ru-dark-2x", &th, (1180.0, 760.0));
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
