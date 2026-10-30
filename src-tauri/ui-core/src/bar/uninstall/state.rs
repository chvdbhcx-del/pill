//! The uninstall panel's state machine, pure (port of src/uninstall-state.js, with every vector of uninstall-state.test.mjs below).
//!
//! Two things move independently:
//!   * the JOB, which lives in the backend (uninstall.rs): waiting for the official uninstaller, the leftover scan, the checklist, the deletion,
//!     the elevated step, the summary. Its phase arrives as `uninstall:state` events. The panel being open or closed never changes it.
//!   * the TAKEOVER: whether the full panel is on screen instead of the normal search results.
//!
//! phases:    idle | confirm | running | scanning | checklist | deleting | admin | done | still | error
//! events:    Open{name}      the user picked an uninstall row: a job exists -> its panel comes back; otherwise the confirm card
//!            Server(payload) an `uninstall:state` event (never opens the panel by itself)
//!            Typing          any keystroke / input that is not the panel's own control: the panel steps aside, the job stays
//!            Esc             Esc inside the panel: like typing, but a confirm card is dropped and a finished summary is dismissed
//!            Focus           strip click, Ctrl+U, `u <name>`, the island card: the panel comes back where it was
//!            Close           the explicit Close / Cancel button: everything is dropped (the UI also tells the backend)
//! selectors: `strip(&state)` what the status strip shows: None (nothing) | Strip { kind: Busy | Ready, phase, name }

use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Phase {
    Idle,
    Confirm,
    Running,
    Scanning,
    Checklist,
    Deleting,
    Admin,
    Done,
    Still,
    Error,
}

/// Phases while the backend is working (`RUNNING`).
pub const RUNNING: [Phase; 4] = [Phase::Running, Phase::Scanning, Phase::Deleting, Phase::Admin];
/// Phases that wait to be seen (`FINISHED`).
pub const FINISHED: [Phase; 4] = [Phase::Checklist, Phase::Done, Phase::Still, Phase::Error];

impl Phase {
    /// The backend's phase word -> panel phase (`MAP`): running, scanning, review -> checklist, deleting, elevating -> admin, done, still, error.
    pub fn from_server(word: &str) -> Option<Phase> {
        Some(match word {
            "running" => Phase::Running,
            "scanning" => Phase::Scanning,
            "review" => Phase::Checklist,
            "deleting" => Phase::Deleting,
            "elevating" => Phase::Admin,
            "done" => Phase::Done,
            "still" => Phase::Still,
            "error" => Phase::Error,
            _ => return None,
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Idle => "idle",
            Phase::Confirm => "confirm",
            Phase::Running => "running",
            Phase::Scanning => "scanning",
            Phase::Checklist => "checklist",
            Phase::Deleting => "deleting",
            Phase::Admin => "admin",
            Phase::Done => "done",
            Phase::Still => "still",
            Phase::Error => "error",
        }
    }
    pub fn is_busy(self) -> bool {
        RUNNING.contains(&self)
    }
    pub fn is_finished(self) -> bool {
        FINISHED.contains(&self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub phase: Phase,
    /// the full panel is on screen instead of the search results
    pub takeover: bool,
    /// a job exists in the backend (None while only a confirm card is up)
    pub job: Option<Job>,
    pub name: String,
    /// the last backend payload: the panel is redrawn from it when it comes back
    pub last: Option<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Ev {
    Open { name: String },
    Server(Value),
    Typing,
    Esc,
    Focus,
    Close,
    /// anything else (the bar hiding / showing, say): the machine does not depend on the window being visible
    Other,
}

pub fn init() -> State {
    State { phase: Phase::Idle, takeover: false, job: None, name: String::new(), last: None }
}

pub fn reduce(s: &State, ev: &Ev) -> State {
    match ev {
        Ev::Open { name } => {
            if s.job.is_some() {
                // a job exists (the same app, or another one: finish or close it first): its panel comes back
                return State { takeover: true, ..s.clone() };
            }
            State { phase: Phase::Confirm, takeover: true, name: name.clone(), ..init() }
        }
        Ev::Server(p) => {
            let Some(phase) = p.get("phase").and_then(Value::as_str).and_then(Phase::from_server) else { return s.clone() };
            let name = p.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).map(str::to_string).unwrap_or_else(|| s.name.clone());
            State { phase, takeover: s.takeover, job: Some(Job { name: name.clone() }), name, last: Some(p.clone()) }
        }
        Ev::Typing => {
            if !s.takeover {
                return s.clone();
            }
            // a confirm card is not a job: typing drops it
            if s.job.is_some() {
                State { takeover: false, ..s.clone() }
            } else {
                init()
            }
        }
        Ev::Esc => {
            if !s.takeover {
                return s.clone();
            }
            if s.job.is_none() || matches!(s.phase, Phase::Done | Phase::Still | Phase::Error) {
                return init(); // dismissed: nothing is left to come back to
            }
            State { takeover: false, ..s.clone() }
        }
        Ev::Focus => {
            if s.job.is_some() {
                State { takeover: true, ..s.clone() }
            } else {
                s.clone()
            }
        }
        Ev::Close => init(),
        Ev::Other => s.clone(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StripKind {
    Busy,
    Ready,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Strip {
    pub kind: StripKind,
    pub phase: Phase,
    pub name: String,
}

/// The status strip: visible only while a job exists and the full panel is not on screen.
pub fn strip(s: &State) -> Option<Strip> {
    let job = s.job.as_ref()?;
    if s.takeover {
        return None;
    }
    Some(Strip { kind: if s.phase.is_busy() { StripKind::Busy } else { StripKind::Ready }, phase: s.phase, name: job.name.clone() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(s: &State, evs: &[Ev]) -> State {
        evs.iter().fold(s.clone(), |a, e| reduce(&a, e))
    }
    fn srv(phase: &str) -> Ev {
        srv_with(phase, json!({}))
    }
    fn srv_with(phase: &str, extra: Value) -> Ev {
        let mut p = json!({ "phase": phase, "name": "qBittorrent" });
        if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        Ev::Server(p)
    }
    fn open(name: &str) -> Ev {
        Ev::Open { name: name.into() }
    }
    fn confirm() -> State {
        run(&init(), &[open("qBittorrent")])
    }

    #[test]
    fn idle_to_confirm() {
        let s = init();
        assert_eq!(s.phase, Phase::Idle);
        assert_eq!(strip(&s), None);
        let s = run(&s, &[open("qBittorrent")]);
        assert_eq!((s.phase, s.takeover, s.job.clone()), (Phase::Confirm, true, None));
        assert_eq!(strip(&s), None, "a confirm card is not a job: no strip");
        // typing in the confirm card drops it (nothing was started)
        assert_eq!(run(&s, &[Ev::Typing]), init());
        assert_eq!(run(&s, &[Ev::Esc]), init());
    }

    #[test]
    fn confirm_through_done_keeps_the_panel_up() {
        let path = [("running", Phase::Running), ("scanning", Phase::Scanning), ("review", Phase::Checklist), ("deleting", Phase::Deleting), ("elevating", Phase::Admin), ("done", Phase::Done)];
        let mut t = confirm();
        for (server, phase) in path {
            t = run(&t, &[srv(server)]);
            assert_eq!(t.phase, phase, "{server}");
            assert!(t.takeover, "server events never move the panel by themselves");
            assert_eq!(strip(&t), None, "panel on screen: no strip");
        }
        assert_eq!(run(&init(), &[srv("still")]).phase, Phase::Still);
        assert_eq!(run(&init(), &[srv("error")]).phase, Phase::Error);
        assert_eq!(run(&init(), &[Ev::Server(json!({ "phase": "bogus" }))]).phase, Phase::Idle, "unknown phases are ignored");
        assert_eq!(run(&init(), &[Ev::Server(Value::Null)]).phase, Phase::Idle);
    }

    #[test]
    fn typing_leaves_the_takeover_but_keeps_the_job() {
        let s = confirm();
        for (server, mapped) in [("running", Phase::Running), ("scanning", Phase::Scanning), ("deleting", Phase::Deleting), ("elevating", Phase::Admin)] {
            let a = run(&s, &[srv(server), Ev::Typing]);
            assert!(!a.takeover, "{server}");
            assert_eq!(a.job.as_ref().map(|j| j.name.as_str()), Some("qBittorrent"), "{server}: the job is kept");
            assert_eq!(strip(&a), Some(Strip { kind: StripKind::Busy, phase: mapped, name: "qBittorrent".into() }), "{server}");
            // the job keeps moving while the user searches elsewhere, and the panel does not come back by itself
            let b = run(&a, &[srv("scanning"), srv("scanning"), srv("review")]);
            assert!(!b.takeover);
            assert_eq!(b.phase, Phase::Checklist);
            assert_eq!(strip(&b), Some(Strip { kind: StripKind::Ready, phase: Phase::Checklist, name: "qBittorrent".into() }), "finished while away: the strip turns into \"review\"");
            // coming back restores the panel (the UI redraws it from `last`)
            let c = run(&b, &[Ev::Focus]);
            assert_eq!((c.takeover, c.phase, strip(&c)), (true, Phase::Checklist, None));
            assert_eq!(c.last.as_ref().unwrap()["phase"], "review");
        }
    }

    #[test]
    fn the_checklist_survives_typing() {
        let s = confirm();
        let mut c = run(&s, &[srv("scanning"), srv_with("review", json!({ "items": [{ "id": 0 }] })), Ev::Typing]);
        assert!(c.job.is_some() && c.phase == Phase::Checklist && c.last.as_ref().unwrap()["items"].as_array().unwrap().len() == 1);
        assert_eq!(strip(&c), Some(Strip { kind: StripKind::Ready, phase: Phase::Checklist, name: "qBittorrent".into() }));
        c = run(&c, &[Ev::Typing, Ev::Typing]);
        assert_eq!(c.phase, Phase::Checklist, "more typing changes nothing");
    }

    #[test]
    fn esc_keeps_running_jobs_and_dismisses_finished_ones() {
        let s = confirm();
        for server in ["running", "scanning", "deleting", "elevating", "review"] {
            let a = run(&s, &[srv(server), Ev::Esc]);
            assert!(a.job.is_some() && !a.takeover, "Esc keeps the job: {server}");
        }
        for server in ["done", "still", "error"] {
            assert_eq!(run(&s, &[srv(server), Ev::Esc]), init(), "Esc dismisses {server}");
        }
        // Esc / typing with the panel already away does nothing
        let away = run(&s, &[srv("scanning"), Ev::Esc]);
        assert_eq!(run(&away, &[Ev::Esc]), away);
        assert_eq!(run(&away, &[Ev::Typing]), away);
    }

    #[test]
    fn focus_open_and_close() {
        let s = confirm();
        let away = run(&s, &[srv("scanning"), Ev::Esc]);
        // focus (strip click, Ctrl+U, `u qBittorrent`, the island card) without a job does nothing
        assert_eq!(run(&init(), &[Ev::Focus]), init());
        // `u <name>` with a job brings the panel back instead of a second confirm card
        let again = run(&away, &[open("qBittorrent")]);
        assert_eq!((again.takeover, again.phase, again.job.unwrap().name.as_str()), (true, Phase::Scanning, "qBittorrent"));
        // another app's row while a job exists: the same, the job is not replaced
        assert_eq!(run(&away, &[open("Other")]).job.unwrap().name, "qBittorrent");
        // explicit Close / Cancel drops everything, also when the strip is showing
        assert_eq!(run(&away, &[Ev::Close]), init());
        assert_eq!(strip(&run(&away, &[Ev::Close])), None);
    }

    #[test]
    fn strip_visible_only_with_a_job_and_the_panel_away() {
        let s = confirm();
        for server in ["running", "scanning", "deleting", "elevating", "review", "done", "still", "error"] {
            let up = run(&s, &[srv(server)]);
            assert_eq!(strip(&up), None, "{server} with the panel up");
            assert!(strip(&run(&up, &[Ev::Typing])).is_some(), "{server} with the panel away");
        }
        assert_eq!(strip(&init()), None);
        assert_eq!(strip(&run(&init(), &[open("x"), Ev::Typing])), None, "a dropped confirm card leaves no strip");
        // a job that was started while the panel was already away (the bar page reloaded, the state comes from the backend)
        let recovered = run(&init(), &[srv("scanning")]);
        assert_eq!((recovered.takeover, strip(&recovered).unwrap().kind), (false, StripKind::Busy));
        assert!(run(&recovered, &[Ev::Focus]).takeover);
    }

    #[test]
    fn bar_hide_and_show_do_not_exist_for_the_machine() {
        let away = run(&confirm(), &[srv("scanning"), Ev::Esc]);
        assert_eq!(run(&away, &[Ev::Other]), away);
    }

    #[test]
    fn the_machine_never_mutates_its_input() {
        let frozen = init();
        let copy = frozen.clone();
        let _ = reduce(&frozen, &open("x"));
        assert_eq!(frozen, copy);
        let busy = run(&confirm(), &[srv("scanning")]);
        let before = busy.clone();
        let _ = reduce(&busy, &Ev::Typing);
        let _ = reduce(&busy, &Ev::Close);
        assert_eq!(busy, before);
    }

    #[test]
    fn name_falls_back_to_the_previous_one() {
        let s = run(&confirm(), &[Ev::Server(json!({ "phase": "scanning" }))]);
        assert_eq!(s.job.unwrap().name, "qBittorrent");
        let s2 = run(&init(), &[Ev::Server(json!({ "phase": "scanning", "name": "" }))]);
        assert_eq!(s2.job.unwrap().name, "");
    }

    #[test]
    fn busy_and_finished_sets() {
        for p in [Phase::Running, Phase::Scanning, Phase::Deleting, Phase::Admin] {
            assert!(p.is_busy() && !p.is_finished());
        }
        for p in [Phase::Checklist, Phase::Done, Phase::Still, Phase::Error] {
            assert!(p.is_finished() && !p.is_busy());
        }
        assert!(!Phase::Confirm.is_busy() && !Phase::Idle.is_finished());
    }
}
