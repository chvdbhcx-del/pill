//! The leftover checklist of the uninstall panel (`listPage` / `group` / `recount` / `itemDone` of uninstall-ui.js) as a model: items split into the
//! Certain / Probable / Admin groups, ticks, the gliding counts and sizes of every group and of the selection, the deleting overlay and the
//! per-item results. The backend only ever receives item indexes (`id`), never paths.

use super::payload::{Item, ItemDone, Review, Tier};
use super::strings as s;
use crate::bar::anim::{self, Anims, Glide};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupKey {
    Certain,
    Probable,
    Admin,
}

impl GroupKey {
    pub fn title(self) -> s::S {
        match self {
            GroupKey::Certain => s::CERTAIN,
            GroupKey::Probable => s::PROBABLE,
            GroupKey::Admin => s::ADMIN,
        }
    }
    pub fn tip(self) -> Option<s::S> {
        match self {
            GroupKey::Certain => Some(s::CERTAIN_TIP),
            GroupKey::Probable => Some(s::PROBABLE_TIP),
            GroupKey::Admin => None,
        }
    }
}

/// One candidate row with its tick and fate.
#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    pub item: Item,
    pub checked: bool,
    /// deleted (the row is leaving / gone)
    pub gone: bool,
    /// the deletion failed: the backend's error replaces the note under the path
    pub fail: Option<String>,
    pub group: usize,
}

pub struct Group {
    pub key: GroupKey,
    pub open: bool,
    /// collapsed away because its last row is gone
    pub hidden: bool,
    /// indexes into `Checklist::picks`
    pub picks: Vec<usize>,
    pub count: Glide,
    pub size: Glide,
}

/// The deleting progress line: `n / total` with a gliding `n`.
pub struct DeleteProgress {
    pub n: Glide,
    pub total: u64,
}

/// What a restored panel needs to bring the ticks back: the payload generation they belong to and the tick per item id.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub seq: u64,
    pub ticks: HashMap<usize, bool>,
}

pub struct Checklist {
    pub review: Review,
    pub picks: Vec<Pick>,
    pub groups: Vec<Group>,
    pub admin_scanned: bool,
    /// deleting: every button rests, the progress line runs
    pub busy: bool,
    pub progress: Option<DeleteProgress>,
    pub sel_count: Glide,
    pub sel_size: Glide,
    pub reduce: bool,
}

impl Checklist {
    /// Builds the groups of a review payload (`listPage`). `carry_admin_scanned`: the administrator scan already ran for this job.
    /// `snapshot` restores ticks when it belongs to this very payload (`seq`).
    pub fn build(review: Review, carry_admin_scanned: bool, snapshot: Option<&Snapshot>, seq: u64, now: f64, reduce: bool) -> Checklist {
        let admin_scanned = carry_admin_scanned || review.admin_scanned;
        let mut picks: Vec<Pick> = review.items.iter().map(|it| Pick { checked: it.ticked, item: it.clone(), gone: false, fail: None, group: 0 }).collect();
        let (mut cert, mut prob, mut adm) = (Vec::new(), Vec::new(), Vec::new());
        for (i, p) in picks.iter().enumerate() {
            if p.item.admin {
                adm.push(i);
            } else if p.item.tier == Tier::Certain {
                cert.push(i);
            } else {
                prob.push(i);
            }
        }
        let mut groups = Vec::new();
        let mut push = |key: GroupKey, ids: Vec<usize>, open: bool, picks: &mut Vec<Pick>| {
            if ids.is_empty() {
                return;
            }
            let g = groups.len();
            for &i in &ids {
                picks[i].group = g;
            }
            groups.push(Group { key, open, hidden: false, picks: ids, count: Glide::new(0.0), size: Glide::new(0.0) });
        };
        let no_cert = cert.is_empty();
        push(GroupKey::Certain, cert, true, &mut picks);
        push(GroupKey::Probable, prob, no_cert, &mut picks);
        push(GroupKey::Admin, adm, true, &mut picks);
        if let Some(sn) = snapshot.filter(|sn| sn.seq == seq) {
            for p in picks.iter_mut() {
                if let Some(&c) = sn.ticks.get(&p.item.id) {
                    p.checked = c;
                }
            }
        }
        let mut c = Checklist { review, picks, groups, admin_scanned, busy: false, progress: None, sel_count: Glide::new(0.0), sel_size: Glide::new(0.0), reduce };
        c.recount_all(now, &mut Anims::new(true));
        c.update_selection(now);
        c
    }

    pub fn snapshot(&self, seq: u64) -> Snapshot {
        Snapshot { seq, ticks: self.picks.iter().filter(|p| !p.gone).map(|p| (p.item.id, p.checked)).collect() }
    }

    fn live(&self, g: usize) -> impl Iterator<Item = &Pick> {
        self.groups[g].picks.iter().map(|&i| &self.picks[i]).filter(|p| !p.gone)
    }

    /// `recount(g)`: the group's live count and size glide; an emptied group collapses away.
    pub fn recount(&mut self, g: usize, now: f64, anims: &mut Anims) {
        let (count, size) = {
            let l: Vec<&Pick> = self.live(g).collect();
            (l.len() as f64, l.iter().map(|p| p.item.size).sum::<u64>() as f64)
        };
        let reduce = self.reduce;
        let grp = &mut self.groups[g];
        grp.count.set(count, now, reduce);
        grp.size.set(size, now, reduce);
        if count == 0.0 && !grp.hidden && !grp.picks.is_empty() {
            grp.hidden = true;
            anims.push(anim::group_vanish(g));
        }
    }
    pub fn recount_all(&mut self, now: f64, anims: &mut Anims) {
        for g in 0..self.groups.len() {
            self.recount(g, now, anims);
        }
    }

    /// Ticked, not yet deleted user (non-admin) items.
    pub fn picked_user(&self) -> Vec<&Pick> {
        self.picks.iter().filter(|p| !p.item.admin && p.checked && !p.gone).collect()
    }
    pub fn picked_admin(&self) -> Vec<&Pick> {
        self.picks.iter().filter(|p| p.item.admin && p.checked && !p.gone).collect()
    }
    pub fn user_ids(&self) -> Vec<usize> {
        self.picked_user().iter().map(|p| p.item.id).collect()
    }
    pub fn admin_ids(&self) -> Vec<usize> {
        self.picked_admin().iter().map(|p| p.item.id).collect()
    }

    /// `S.upd()`: the "Selected: N · size" text glides.
    pub fn update_selection(&mut self, now: f64) {
        let (n, sz) = {
            let u = self.picked_user();
            (u.len() as f64, u.iter().map(|p| p.item.size).sum::<u64>() as f64)
        };
        self.sel_count.set(n, now, self.reduce);
        self.sel_size.set(sz, now, self.reduce);
    }

    pub fn set_checked(&mut self, pick: usize, checked: bool, now: f64) {
        if let Some(p) = self.picks.get_mut(pick) {
            if !p.gone {
                p.checked = checked;
            }
        }
        self.update_selection(now);
    }
    pub fn toggle(&mut self, pick: usize, now: f64) {
        if let Some(p) = self.picks.get(pick) {
            let c = !p.checked;
            self.set_checked(pick, c, now);
        }
    }

    /// «Все "точно"»: ticks every live certain item (the JS tests the tier only, so a certain administrator item is ticked too).
    pub fn select_all_certain(&mut self, now: f64) {
        for p in self.picks.iter_mut() {
            if p.item.tier == Tier::Certain && !p.gone {
                p.checked = true;
            }
        }
        self.update_selection(now);
    }

    /// Group header click: expand / collapse the body.
    pub fn toggle_group(&mut self, g: usize, anims: &mut Anims) {
        let Some(grp) = self.groups.get_mut(g) else { return };
        grp.open = !grp.open;
        anims.push(if grp.open { anim::group_expand(g) } else { anim::group_collapse(g) });
    }

    pub fn has_certain(&self) -> bool {
        self.review.items.iter().any(|i| i.tier == Tier::Certain)
    }
    pub fn has_user(&self) -> bool {
        self.review.items.iter().any(|i| !i.admin)
    }
    pub fn has_admin(&self) -> bool {
        self.review.items.iter().any(|i| i.admin)
    }

    /// `busy(true)`: every button rests and the progress line drops in (only the first time).
    pub fn begin_busy(&mut self, anims: &mut Anims) {
        self.busy = true;
        if self.progress.is_none() {
            self.progress = Some(DeleteProgress { n: Glide::new(0.0), total: 0 });
            anims.push(anim::delete_progress_enter());
        }
    }

    /// One `uninstall:item` result. Returns false when no live row has that path.
    pub fn item_done(&mut self, d: &ItemDone, now: f64, anims: &mut Anims) -> bool {
        let reduce = self.reduce;
        if let Some(pr) = self.progress.as_mut() {
            pr.n.set(d.n as f64, now, reduce);
            pr.total = d.total;
        }
        let Some(i) = self.picks.iter().position(|p| p.item.path == d.shown && !p.gone) else { return false };
        if d.ok {
            self.picks[i].gone = true;
            anims.push(anim::checklist_row_exit(i));
            anims.push(anim::checklist_row_collapse(i).with_delay(180));
            self.recount_all(now, anims);
        } else {
            let p = &mut self.picks[i];
            p.fail = Some(d.error.clone());
            p.checked = false;
            anims.push(anim::shake(anim::Target::ChecklistRow(i)));
            self.update_selection(now);
        }
        true
    }

    /// Advance every glide to `now`; true while any is still moving (the renderer should keep redrawing).
    pub fn tick(&mut self, now: f64) -> bool {
        let mut running = false;
        for g in self.groups.iter_mut() {
            g.count.value(now);
            g.size.value(now);
            running |= g.count.running() || g.size.running();
        }
        self.sel_count.value(now);
        self.sel_size.value(now);
        running |= self.sel_count.running() || self.sel_size.running();
        if let Some(p) = self.progress.as_mut() {
            p.n.value(now);
            running |= p.n.running();
        }
        running
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn review(items: serde_json::Value) -> Review {
        Review::from_value(&json!({ "phase": "review", "name": "App", "items": items }))
    }
    fn item(id: usize, path: &str, tier: &str, admin: bool, size: u64, ticked: bool) -> serde_json::Value {
        json!({ "id": id, "kind": "folder", "path": path, "size": size, "ticked": ticked, "note": "", "admin": admin, "tier": tier })
    }
    fn sample() -> Checklist {
        Checklist::build(
            review(json!([item(0, "C:\\a", "certain", false, 1000, true), item(1, "C:\\b", "certain", false, 2000, true), item(2, "C:\\c", "probable", false, 500, false), item(3, "HKLM\\d", "certain", true, 0, false)])),
            false,
            None,
            1,
            0.0,
            true,
        )
    }

    #[test]
    fn groups_split_by_tier_and_admin() {
        let c = sample();
        assert_eq!(c.groups.iter().map(|g| g.key).collect::<Vec<_>>(), vec![GroupKey::Certain, GroupKey::Probable, GroupKey::Admin]);
        assert_eq!((c.groups[0].open, c.groups[1].open, c.groups[2].open), (true, false, true), "Probable opens only when there is no Certain group");
        assert_eq!(c.groups[0].picks, vec![0, 1]);
        assert_eq!(c.user_ids(), vec![0, 1], "certain items are pre-ticked");
        assert!(c.admin_ids().is_empty());
        let only_prob = Checklist::build(review(json!([item(0, "x", "probable", false, 1, false)])), false, None, 1, 0.0, true);
        assert_eq!(only_prob.groups.len(), 1);
        assert!(only_prob.groups[0].open);
        assert!(Checklist::build(review(json!([])), false, None, 1, 0.0, true).groups.is_empty());
    }

    #[test]
    fn counts_and_selection_follow_the_ticks() {
        let mut c = sample();
        assert_eq!((c.sel_count.target(), c.sel_size.target()), (2.0, 3000.0));
        assert_eq!((c.groups[0].count.target(), c.groups[0].size.target()), (2.0, 3000.0));
        c.toggle(2, 0.0);
        assert_eq!((c.sel_count.target(), c.sel_size.target()), (3.0, 3500.0));
        c.toggle(0, 0.0);
        assert_eq!(c.sel_count.target(), 2.0);
        c.select_all_certain(0.0);
        assert_eq!(c.user_ids(), vec![0, 1, 2]);
        assert_eq!(c.admin_ids(), vec![3], "the JS ticks certain administrator items too");
        c.set_checked(3, false, 0.0);
        assert!(c.admin_ids().is_empty());
    }

    #[test]
    fn only_user_items_are_sent_for_deletion_and_only_admin_ones_for_the_elevated_step() {
        let mut c = sample();
        c.set_checked(3, true, 0.0);
        assert_eq!(c.user_ids(), vec![0, 1]);
        assert_eq!(c.admin_ids(), vec![3]);
    }

    #[test]
    fn deleted_rows_leave_and_empty_groups_collapse() {
        let mut c = sample();
        let mut a = Anims::new(false);
        c.begin_busy(&mut a);
        assert!(c.busy && c.progress.is_some());
        assert_eq!(a.len(), 1, "the progress line drops in");
        c.begin_busy(&mut a);
        assert_eq!(a.len(), 1, "only the first time");
        let ok = |p: &str, n| ItemDone { shown: p.into(), ok: true, error: String::new(), n, total: 2 };
        assert!(c.item_done(&ok("C:\\a", 1), 0.0, &mut a));
        assert!(c.picks[0].gone);
        assert_eq!(c.groups[0].count.target(), 1.0);
        assert_eq!(c.user_ids(), vec![1], "a deleted row is no longer a candidate");
        assert!(!c.item_done(&ok("C:\\a", 1), 0.0, &mut a), "already gone");
        assert!(c.item_done(&ok("C:\\b", 2), 0.0, &mut a));
        assert!(c.groups[0].hidden, "the emptied group collapses away");
        assert_eq!(c.progress.as_ref().unwrap().n.target(), 2.0);
        assert_eq!(c.progress.as_ref().unwrap().total, 2);
        let kinds: Vec<_> = a.drain().into_iter().map(|x| x.kind).collect();
        assert!(kinds.contains(&anim::Kind::Exit) && kinds.contains(&anim::Kind::Collapse));
    }

    #[test]
    fn a_failed_item_stays_unticked_with_its_error() {
        let mut c = sample();
        let mut a = Anims::new(false);
        let bad = ItemDone { shown: "C:\\b".into(), ok: false, error: "in use".into(), n: 1, total: 2 };
        assert!(c.item_done(&bad, 0.0, &mut a));
        assert!(!c.picks[1].gone && !c.picks[1].checked);
        assert_eq!(c.picks[1].fail.as_deref(), Some("in use"));
        assert_eq!(c.user_ids(), vec![0]);
        assert_eq!(c.sel_count.target(), 1.0);
        assert!(a.drain().iter().any(|x| x.kind == anim::Kind::Shake));
    }

    #[test]
    fn snapshots_restore_ticks_only_for_the_same_payload() {
        let mut c = sample();
        c.toggle(0, 0.0);
        c.set_checked(2, true, 0.0);
        let sn = c.snapshot(7);
        let r = review(json!([item(0, "C:\\a", "certain", false, 1000, true), item(1, "C:\\b", "certain", false, 2000, true), item(2, "C:\\c", "probable", false, 500, false)]));
        let back = Checklist::build(r.clone(), false, Some(&sn), 7, 0.0, true);
        assert_eq!(back.user_ids(), vec![1, 2], "ticks come back");
        let other = Checklist::build(r, false, Some(&sn), 8, 0.0, true);
        assert_eq!(other.user_ids(), vec![0, 1], "a newer payload starts from the backend's own ticks");
    }

    #[test]
    fn group_toggle_animates_and_tick_reports_motion() {
        let mut c = Checklist::build(review(json!([item(0, "a", "certain", false, 10, true)])), false, None, 1, 0.0, false);
        assert!(c.groups[0].count.running(), "counts glide from 0 on a fresh page");
        assert!(c.tick(100.0));
        assert!(!c.tick(1000.0));
        let mut a = Anims::new(false);
        c.toggle_group(0, &mut a);
        assert!(!c.groups[0].open);
        c.toggle_group(0, &mut a);
        let kinds: Vec<_> = a.drain().into_iter().map(|x| x.kind).collect();
        assert_eq!(kinds, vec![anim::Kind::Collapse, anim::Kind::Expand]);
        c.toggle_group(9, &mut a);
        assert!(a.is_empty());
    }

    #[test]
    fn admin_scanned_is_carried_over() {
        let c = Checklist::build(review(json!([])), true, None, 1, 0.0, true);
        assert!(c.admin_scanned);
        let r = Review::from_value(&json!({ "items": [], "adminScanned": true }));
        assert!(Checklist::build(r, false, None, 1, 0.0, true).admin_scanned);
        assert!(!Checklist::build(review(json!([])), false, None, 1, 0.0, true).admin_scanned);
    }
}
