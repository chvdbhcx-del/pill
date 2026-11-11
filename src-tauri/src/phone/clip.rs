// Clipboard sync between the PC and the phones (docs/phone-protocol.md, "Clipboard (v1.2)"): the pure decisions and the loop-prevention state.
// The Win32 side (listener window, reading and writing the clipboard) is `clipwin.rs`; everything here is plain data so it can be tested.
// PRIVACY: a clipboard holds passwords. Opt-in (`phone.clipboard`, default off); only a hash of the last texts is kept, text is never logged.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Mutex;

/// Largest text (UTF-8 bytes) that travels in either direction.
pub const MAX_BYTES: usize = 256 * 1024;

/// The settings that gate the feature, read fresh each time (a change applies at once).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClipCfg {
    /// `phone.clipboard`: the master switch, default OFF
    pub on: bool,
    /// `phone.clipToPhone`
    pub to_phone: bool,
    /// `phone.clipFromPhone`
    pub from_phone: bool,
}

/// The system clipboard as the server sees it. The real one is `clipwin::WinClip`; tests use a fake.
pub trait ClipIo: Send + Sync {
    /// Puts plain text on the clipboard. The change it causes must not come back through the watcher.
    fn set_text(&self, text: &str) -> bool;
    /// Registers (`true`) or drops (`false`) the change listener. Idempotent; while dropped nothing runs and nothing wakes up.
    fn watch(&self, on: bool);
}

/// Which of the "do not process this" formats a clipboard carries (password managers set them).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Formats {
    /// `ExcludeClipboardContentFromMonitorProcessing` (or the older `Clipboard Viewer Ignore`) is present
    pub exclude: bool,
    /// the DWORD of `CanIncludeInClipboardHistory`, when the format is present
    pub history: Option<u32>,
    /// the DWORD of `CanUploadToCloudClipboard`, when the format is present
    pub cloud: Option<u32>,
}

impl Formats {
    /// Registers one registered clipboard format by its name; `dword` reads its value (only called for the two that carry one).
    pub fn note(&mut self, name: &str, dword: impl FnOnce() -> Option<u32>) {
        match name.to_ascii_lowercase().as_str() {
            "excludeclipboardcontentfrommonitorprocessing" | "clipboard viewer ignore" => self.exclude = true,
            "canincludeinclipboardhistory" => self.history = dword(),
            "canuploadtocloudclipboard" => self.cloud = dword(),
            _ => {}
        }
    }
    /// False for a clipboard that must not leave the PC.
    pub fn allows(&self) -> bool {
        !self.exclude && self.history != Some(0) && self.cloud != Some(0)
    }
}

/// Why a local clipboard text is not sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
    Empty,
    TooBig,
    /// the text a phone just sent (or that this PC just sent): never echoed
    Echo,
}

fn digest(t: &str) -> [u8; 32] {
    Sha256::digest(t.as_bytes()).into()
}

/// What `clip` carries: the text of a well-formed message, else `None` (empty, over 256 KiB, not a string: dropped without an answer).
pub fn parse(m: &Value) -> Option<String> {
    let t = m["text"].as_str()?;
    (!t.is_empty() && t.len() <= MAX_BYTES).then(|| t.to_string())
}

/// The wire message for a text.
pub fn message(id: &str, text: &str, ts: i64) -> Value {
    serde_json::json!({ "t": "clip", "id": id, "text": text, "ts": ts })
}

/// Hashes of the last text the PC applied from a phone and the last it sent; no text is kept.
#[derive(Default)]
pub struct State {
    last: Mutex<(Option<[u8; 32]>, Option<[u8; 32]>)>, // (applied, sent)
}

impl State {
    fn g(&self) -> std::sync::MutexGuard<'_, (Option<[u8; 32]>, Option<[u8; 32]>)> {
        self.last.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A text from a phone. True = apply it (and it is now remembered as applied); false = it is what the PC already holds or just sent.
    pub fn take_incoming(&self, text: &str) -> bool {
        let h = digest(text);
        let mut g = self.g();
        if g.0 == Some(h) || g.1 == Some(h) {
            return false;
        }
        *g = (Some(h), None);
        true
    }

    /// The clipboard write failed: forget that it was applied.
    pub fn undo_incoming(&self, text: &str) {
        let mut g = self.g();
        if g.0 == Some(digest(text)) {
            g.0 = None;
        }
    }

    /// A change on the PC. `Ok` = send it (call `sent` once a phone got it).
    pub fn check_outgoing(&self, text: &str) -> Result<(), Skip> {
        if text.is_empty() {
            return Err(Skip::Empty);
        }
        if text.len() > MAX_BYTES {
            return Err(Skip::TooBig);
        }
        let h = digest(text);
        let g = self.g();
        if g.0 == Some(h) || g.1 == Some(h) {
            return Err(Skip::Echo);
        }
        Ok(())
    }

    /// A phone got this text: remember it as sent (copying it again is not news) and forget what was applied (copying that again is).
    pub fn sent(&self, text: &str) {
        *self.g() = (None, Some(digest(text)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wire_parsing_and_limits() {
        assert_eq!(parse(&json!({ "t": "clip", "id": "a", "text": "hi", "ts": 1 })).as_deref(), Some("hi"));
        assert_eq!(parse(&json!({ "t": "clip", "text": "" })), None, "empty text is dropped");
        assert_eq!(parse(&json!({ "t": "clip" })), None);
        assert_eq!(parse(&json!({ "t": "clip", "text": 5 })), None);
        assert!(parse(&json!({ "text": "a".repeat(MAX_BYTES) })).is_some(), "exactly 256 KiB passes");
        assert_eq!(parse(&json!({ "text": "a".repeat(MAX_BYTES + 1) })), None);
        // the limit counts UTF-8 bytes, not characters
        assert_eq!(parse(&json!({ "text": "я".repeat(MAX_BYTES / 2 + 1) })), None);
        assert!(parse(&json!({ "text": "я".repeat(MAX_BYTES / 2) })).is_some());
        let m = message("id1", "привет", 42);
        assert_eq!(m, json!({ "t": "clip", "id": "id1", "text": "привет", "ts": 42 }));
    }

    #[test]
    fn exclusion_formats_decide() {
        let mut f = Formats::default();
        assert!(f.allows(), "a plain clipboard is fine");
        f.note("CanIncludeInClipboardHistory", || Some(1));
        f.note("CanUploadToCloudClipboard", || Some(1));
        f.note("SomethingElse", || panic!("not read"));
        assert!(f.allows(), "1 means allowed");
        let mut h = f;
        h.note("canincludeinclipboardhistory", || Some(0));
        assert!(!h.allows());
        let mut c = Formats::default();
        c.note("CanUploadToCloudClipboard", || Some(0));
        assert!(!c.allows());
        let mut x = Formats::default();
        x.note("ExcludeClipboardContentFromMonitorProcessing", || panic!("no value to read"));
        assert!(!x.allows(), "the format's presence is enough");
        let mut v = Formats::default();
        v.note("Clipboard Viewer Ignore", || None);
        assert!(!v.allows());
        let mut unread = Formats::default();
        unread.note("CanIncludeInClipboardHistory", || None);
        assert!(unread.allows(), "a value that cannot be read does not block");
    }

    #[test]
    fn outgoing_limits() {
        let s = State::default();
        assert_eq!(s.check_outgoing(""), Err(Skip::Empty));
        assert_eq!(s.check_outgoing(&"x".repeat(MAX_BYTES + 1)), Err(Skip::TooBig));
        assert_eq!(s.check_outgoing(&"x".repeat(MAX_BYTES)), Ok(()));
    }

    #[test]
    fn loop_prevention() {
        let s = State::default();
        // a phone sends "a": applied once; the change it causes on the PC is not sent back
        assert!(s.take_incoming("a"));
        assert_eq!(s.check_outgoing("a"), Err(Skip::Echo), "the applied text is never echoed");
        assert!(!s.take_incoming("a"), "the same text again is not applied twice");
        // the user copies "b" on the PC: sent, and a phone echoing it back is ignored
        assert_eq!(s.check_outgoing("b"), Ok(()));
        s.sent("b");
        assert_eq!(s.check_outgoing("b"), Err(Skip::Echo), "copying the same text again is not news");
        assert!(!s.take_incoming("b"), "the phone's echo of what the PC sent is ignored");
        // "a" is new again once something else happened
        assert_eq!(s.check_outgoing("a"), Ok(()));
        // a phone text after the PC sent: applied, and now the PC's own "b" is news again
        assert!(s.take_incoming("c"));
        assert_eq!(s.check_outgoing("b"), Ok(()));
        // a failed clipboard write is forgotten
        assert!(s.take_incoming("d"));
        s.undo_incoming("d");
        assert!(s.take_incoming("d"));
    }
}
