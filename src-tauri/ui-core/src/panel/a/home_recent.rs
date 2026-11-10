//! The "Recently copied" card of the Home tab: the newest few clipboard entries as chips, a click copies one back.
//! It reads the clipboard history through the same `clip_list` the Clipboard tab uses (so private mode, the deny list and the retention
//! rules apply) and asks for it only while the card is on and the tab is shown, and again when `clip:added` says something new arrived.
use super::clipboard::Clip;
use super::{BackendCall, Tr};
use serde_json::{json, Value};

/// How many chips the card holds.
pub const SHOWN: usize = 3;
/// Longest label kept (the chip clips it to its width anyway).
const LABEL_MAX: usize = 60;

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: i64,
    pub label: String,
    /// `text` | `image` | `files`
    pub kind: String,
}

/// `clip_list`: the history is newest first except that pins come on top, so ask for a few more than shown and sort by use.
pub fn list_call() -> BackendCall {
    BackendCall::new("clip_list", json!({ "query": null, "kind": null, "limit": SHOWN * 4, "offset": 0 }))
}

/// Copy an entry back to the clipboard without pasting it anywhere.
pub fn copy_call(id: i64) -> BackendCall {
    BackendCall::new("clip_paste", json!({ "id": id, "copyOnly": true, "plain": false }))
}

/// One line for a chip: whitespace collapsed, long text cut with an ellipsis; pictures and files get a word.
pub fn label(c: &Clip, tr: &dyn Tr) -> String {
    match c.kind.as_str() {
        "image" => tr.t("Картинка", "Image"),
        "files" => {
            let n = c.file_count.max(1);
            if tr.is_ru() {
                format!("Файлы: {n}")
            } else {
                format!("{n} file{}", if n == 1 { "" } else { "s" })
            }
        }
        _ => {
            let one: String = c.preview.split_whitespace().collect::<Vec<_>>().join(" ");
            if one.is_empty() {
                "…".into()
            } else if one.chars().count() > LABEL_MAX {
                format!("{}…", one.chars().take(LABEL_MAX).collect::<String>().trim_end())
            } else {
                one
            }
        }
    }
}

/// The newest [`SHOWN`] entries of a `clip_list` answer.
pub fn items(res: &Value, tr: &dyn Tr) -> Vec<Item> {
    let mut all: Vec<Clip> = res.as_array().map(|a| a.iter().map(Clip::from_json).collect()).unwrap_or_default();
    all.sort_by_key(|c| std::cmp::Reverse(c.used_at));
    all.iter().take(SHOWN).map(|c| Item { id: c.id, label: label(c, tr), kind: c.kind.clone() }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::a::{EnTr, RuTr};

    fn row(id: i64, kind: &str, preview: &str, used: i64) -> Value {
        json!({ "id": id, "kind": kind, "preview": preview, "fileCount": 2, "pinned": false, "usedAt": used })
    }

    #[test]
    fn the_newest_three_by_use_win_over_pins_listed_first() {
        let res = json!([row(1, "text", "pinned long ago", 10), row(5, "text", "e", 50), row(4, "text", "d", 40), row(3, "text", "c", 30), row(2, "text", "b", 20)]);
        let it = items(&res, &EnTr);
        assert_eq!(it.iter().map(|i| i.id).collect::<Vec<_>>(), [5, 4, 3]);
        assert!(items(&Value::Null, &EnTr).is_empty());
    }

    #[test]
    fn labels_are_one_short_line() {
        let c = |kind: &str, p: &str| Clip { id: 1, kind: kind.into(), preview: p.into(), truncated: false, file_count: 2, pinned: false, source: String::new(), used_at: 0 };
        assert_eq!(label(&c("text", "  two\n  lines\t here "), &EnTr), "two lines here");
        assert_eq!(label(&c("text", ""), &EnTr), "…");
        let long = "x".repeat(200);
        assert_eq!(label(&c("text", &long), &EnTr).chars().count(), LABEL_MAX + 1);
        assert_eq!(label(&c("image", ""), &RuTr), "Картинка");
        assert_eq!(label(&c("files", ""), &EnTr), "2 files");
        assert_eq!(label(&c("files", ""), &RuTr), "Файлы: 2");
    }

    #[test]
    fn calls_are_the_clipboard_tabs_own() {
        assert_eq!(list_call().cmd, "clip_list");
        assert_eq!(copy_call(7).args, json!({ "id": 7, "copyOnly": true, "plain": false }));
    }
}
