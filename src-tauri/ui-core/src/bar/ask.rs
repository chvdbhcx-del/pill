//! The AI side of the bar: `? question` / `ии question` / `ai question` goes to `cmd_ask` (the chat model from Settings > AI) and the answer is
//! drawn above the list. After an answer the bar stays in a dialogue: the next line is a follow-up, sent with the last 6 turns (12 messages) as
//! `history` (memory only; ended by Esc / close / «Новый разговор»). This module is the model: the pending request, the dialogue memory, the
//! answer text (accumulated chunk by chunk when the backend streams, replaced by the final text), the error wording and `plain_text`
//! (markdown markers stripped, for «Скопировать ответ»; a compact port of md-lite.js `plain`). Network use stays opt-in: nothing is sent unless
//! `Config::ask_ready` (the `net.ai` switch plus address plus model).

use super::lang::{Lang, Tr};
use super::protocol::{AskResult, Config, Turn};
use super::strings as s;

/// Memory limit: 6 turns = 12 messages.
pub const MAX_MESSAGES: usize = 12;

/// What is drawn in the answer block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    None,
    /// «Думаю…» with a spinner. `partial` is the streamed text so far (empty until the first chunk arrives)
    Thinking { partial: String },
    /// the finished answer (markdown source; the renderer formats it)
    Shown(String),
}

/// Why an ask ended without an answer, as the bar words it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AskError {
    pub text: String,
    /// the message is a link to Settings > AI (`msg.dataset.page = 'ai'`)
    pub settings_page: Option<&'static str>,
    /// `cancelled`: no message at all
    pub silent: bool,
}

/// The wording of `cmd_ask` errors (`net_off`, `no_endpoint`, `no_model`, `cancelled`, anything else shown verbatim, cut to 300 chars).
pub fn error_text(code: Option<&str>, lang: Lang) -> AskError {
    match code {
        Some("cancelled") => AskError { text: String::new(), settings_page: None, silent: true },
        Some("net_off") => AskError { text: s::ASK_OFF.tr(lang).to_string(), settings_page: Some("ai"), silent: false },
        Some("no_endpoint") => AskError { text: s::ASK_NO_URL.tr(lang).to_string(), settings_page: Some("ai"), silent: false },
        Some("no_model") => AskError { text: s::ASK_NO_MODEL.tr(lang).to_string(), settings_page: Some("ai"), silent: false },
        Some(e) if !e.is_empty() => AskError { text: e.chars().take(300).collect(), settings_page: None, silent: false },
        _ => AskError { text: s::ASK_FAIL.tr(lang).to_string(), settings_page: None, silent: false },
    }
}

/// An ask that is in flight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InFlight {
    pub id: u64,
    /// the query generation it was started in: a changed query or a closed bar drops the answer
    pub seq: u64,
    pub question: String,
    /// the dialogue memory sent with it
    pub history: Vec<Turn>,
    pub stream: String,
}

/// How an ask ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Finished {
    /// the answer was accepted: the dialogue now holds it
    Answered(String),
    /// no answer; `history_len` > 0 means a dialogue is open and its last answer comes back
    Failed(AskError, bool),
    /// the reply belongs to an older ask / query: ignored
    Stale,
}

#[derive(Clone, Debug, Default)]
pub struct Dialog {
    conv: Vec<Turn>,
    last_ans: String,
    next_id: u64,
    inflight: Option<InFlight>,
}

impl Dialog {
    pub fn new() -> Dialog {
        Dialog::default()
    }

    /// The dialogue is open (an answer was received and not ended yet): typing is a follow-up.
    pub fn is_open(&self) -> bool {
        !self.conv.is_empty()
    }
    pub fn last_answer(&self) -> &str {
        &self.last_ans
    }
    pub fn history(&self) -> &[Turn] {
        &self.conv
    }
    pub fn asking(&self) -> bool {
        self.inflight.is_some()
    }
    pub fn inflight(&self) -> Option<&InFlight> {
        self.inflight.as_ref()
    }

    /// `endDialog()`: forget the memory. True when there was something to forget.
    pub fn end(&mut self) -> bool {
        if self.conv.is_empty() && self.last_ans.is_empty() {
            return false;
        }
        self.conv.clear();
        self.last_ans.clear();
        true
    }

    /// Starts an ask. `None` when one is already running (`if (asking) return`).
    pub fn begin(&mut self, question: &str, seq: u64) -> Option<&InFlight> {
        if self.inflight.is_some() {
            return None;
        }
        self.next_id += 1;
        self.inflight = Some(InFlight { id: self.next_id, seq, question: question.to_string(), history: self.conv.clone(), stream: String::new() });
        self.inflight.as_ref()
    }

    /// A streamed piece of the answer. False when it is not for the ask in flight.
    pub fn chunk(&mut self, id: u64, delta: &str) -> bool {
        match self.inflight.as_mut() {
            Some(f) if f.id == id => {
                f.stream.push_str(delta);
                true
            }
            _ => false,
        }
    }

    /// The streamed text so far (empty when nothing streams or nothing is in flight).
    pub fn partial(&self) -> &str {
        self.inflight.as_ref().map(|f| f.stream.as_str()).unwrap_or("")
    }

    /// The end of an ask (the `cmd_ask` result). `current_seq` is the bar's present query generation.
    pub fn finish(&mut self, id: u64, result: &AskResult, current_seq: u64, lang: Lang) -> Finished {
        let Some(f) = self.inflight.as_ref() else { return Finished::Stale };
        if f.id != id || f.seq != current_seq {
            return Finished::Stale;
        }
        let f = self.inflight.take().unwrap();
        match result {
            AskResult::Answer(text) => {
                // the final text wins; a backend that only streamed leaves the answer in the chunks
                let text = if text.is_empty() { f.stream.clone() } else { text.clone() };
                let mut conv = f.history;
                conv.push(Turn { role: "user".into(), content: f.question });
                conv.push(Turn { role: "assistant".into(), content: text.clone() });
                let drop = conv.len().saturating_sub(MAX_MESSAGES);
                self.conv = conv.split_off(drop);
                self.last_ans = text.clone();
                Finished::Answered(text)
            }
            AskResult::Failed(code) => Finished::Failed(error_text(code.as_deref(), lang), !f.history.is_empty()),
        }
    }

    /// Drops an in-flight ask (new keystroke, Esc, bar closed). True when one was running: the caller sends `ai_transform_cancel`.
    pub fn abandon(&mut self) -> bool {
        self.inflight.take().is_some()
    }
}

// ---- plain text of an answer ----

/// Inline markers removed: `**b**`, `*i*`, `` `c` `` and `[text](url)` (kept as `text (url)`).
fn inline_plain(s: &str) -> String {
    const WINDOW: usize = 500;
    let c: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch == '`' {
            if let Some(j) = (i + 1..c.len().min(i + WINDOW + 1)).find(|&j| c[j] == '`') {
                if j > i + 1 {
                    out.extend(&c[i + 1..j]);
                    i = j + 1;
                    continue;
                }
            }
        } else if ch == '*' && c.get(i + 1) == Some(&'*') {
            let end = c.len().min(i + 2 + WINDOW);
            if let Some(j) = (i + 2..end.saturating_sub(1)).find(|&j| c[j] == '*' && c[j + 1] == '*') {
                let inner: String = c[i + 2..j].iter().collect();
                if j > i + 2 && !inner.starts_with(char::is_whitespace) && !inner.ends_with(char::is_whitespace) {
                    out.push_str(&inline_plain(&inner));
                    i = j + 2;
                    continue;
                }
            }
        } else if ch == '*' && c.get(i + 1).is_some_and(|n| !n.is_whitespace() && *n != '*') {
            let end = c.len().min(i + 1 + WINDOW);
            if let Some(j) = (i + 2..end).find(|&k| c[k] == '*' && c[k - 1] != '*' && c.get(k + 1) != Some(&'*') && !c[k - 1].is_whitespace()) {
                let inner: String = c[i + 1..j].iter().collect();
                out.push_str(&inline_plain(&inner));
                i = j + 1;
                continue;
            }
        } else if ch == '[' {
            let rest: String = c[i..c.len().min(i + 720)].iter().collect();
            if let Some(close) = rest.find("](") {
                let label = &rest[1..close];
                let after = &rest[close + 2..];
                if let Some(end) = after.find(')') {
                    let url = &after[..end];
                    if (1..=200).contains(&label.chars().count()) && !label.contains(['\n', ']']) && !url.contains(char::is_whitespace) && url.chars().count() <= 500 {
                        out.push_str(label);
                        if !url.is_empty() {
                            out.push_str(&format!(" ({url})"));
                        }
                        i += rest[..close + 2 + end + 1].chars().count();
                        continue;
                    }
                }
            }
        }
        out.push(ch);
        i += 1;
    }
    out
}

fn list_item(line: &str) -> Option<(bool, &str)> {
    let t = line.trim_start();
    let mut it = t.chars();
    let first = it.next()?;
    if "-*+".contains(first) {
        let rest = it.as_str();
        return rest.starts_with(char::is_whitespace).then(|| (true, rest.trim_start()));
    }
    let digits: String = t.chars().take_while(char::is_ascii_digit).collect();
    if (1..=3).contains(&digits.len()) {
        let rest = &t[digits.len()..];
        let mut r = rest.chars();
        if matches!(r.next(), Some('.') | Some(')')) {
            let tail = r.as_str();
            return tail.starts_with(char::is_whitespace).then(|| (false, tail.trim_start()));
        }
    }
    None
}

/// The answer as plain text, markers stripped (for «Скопировать ответ»): paragraphs joined by a blank line, code blocks as they are,
/// lists as `- item` / `1. item`, headings as their text.
pub fn plain_text(text: &str) -> String {
    let norm = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = norm.split('\n').collect();
    let mut blocks: Vec<String> = Vec::new();
    let mut para: Vec<String> = Vec::new();
    // current list: (ordered?, items)
    let mut list: Option<(bool, Vec<String>)> = None;
    fn end_para(para: &mut Vec<String>, blocks: &mut Vec<String>) {
        if !para.is_empty() {
            blocks.push(inline_plain(&para.join("\n")));
            para.clear();
        }
    }
    fn end_list(list: &mut Option<(bool, Vec<String>)>, blocks: &mut Vec<String>) {
        if let Some((ul, items)) = list.take() {
            blocks.push(items.iter().enumerate().map(|(i, it)| if ul { format!("- {it}") } else { format!("{}. {it}", i + 1) }).collect::<Vec<_>>().join("\n"));
        }
    }
    let mut n = 0;
    while n < lines.len() {
        let line = lines[n];
        if line.trim_start().starts_with("```") {
            end_para(&mut para, &mut blocks);
            end_list(&mut list, &mut blocks);
            let mut code = Vec::new();
            n += 1;
            while n < lines.len() && lines[n].trim() != "```" {
                code.push(lines[n]);
                n += 1;
            }
            blocks.push(code.join("\n"));
            n += 1;
            continue;
        }
        if line.trim().is_empty() {
            end_para(&mut para, &mut blocks);
            end_list(&mut list, &mut blocks);
            n += 1;
            continue;
        }
        if let Some((ul, rest)) = list_item(line) {
            end_para(&mut para, &mut blocks);
            if list.as_ref().is_none_or(|l| l.0 != ul) {
                end_list(&mut list, &mut blocks);
                list = Some((ul, Vec::new()));
            }
            if let Some(l) = list.as_mut() {
                l.1.push(inline_plain(rest));
            }
            n += 1;
            continue;
        }
        if let Some(l) = list.as_mut() {
            if line.starts_with(char::is_whitespace) {
                if let Some(last) = l.1.last_mut() {
                    last.push('\n');
                    last.push_str(&inline_plain(line.trim()));
                }
                n += 1;
                continue;
            }
        }
        end_list(&mut list, &mut blocks);
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && line[hashes..].starts_with(char::is_whitespace) {
            end_para(&mut para, &mut blocks);
            blocks.push(inline_plain(line[hashes..].trim_start()));
        } else {
            para.push(line.to_string());
        }
        n += 1;
    }
    end_para(&mut para, &mut blocks);
    end_list(&mut list, &mut blocks);
    blocks.join("\n\n").trim().to_string()
}

/// Is the AI usable right now? Same test the ask row uses for its status line.
pub fn ready(cfg: &Config) -> bool {
    cfg.ask_ready()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(t: &str) -> AskResult {
        AskResult::Answer(t.into())
    }

    #[test]
    fn dialogue_memory_keeps_the_last_six_turns() {
        let mut d = Dialog::new();
        let mut seq = 1;
        for i in 0..8 {
            let id = d.begin(&format!("q{i}"), seq).unwrap().id;
            assert!(matches!(d.finish(id, &ok(&format!("a{i}")), seq, Lang::En), Finished::Answered(_)));
        }
        assert_eq!(d.history().len(), MAX_MESSAGES);
        assert_eq!(d.history()[0].content, "q2", "the oldest two turns dropped out");
        assert_eq!(d.history()[11].content, "a7");
        assert_eq!(d.last_answer(), "a7");
        assert!(d.is_open());
        seq += 1;
        let f = d.begin("next", seq).unwrap();
        assert_eq!(f.history.len(), 12, "the follow-up carries the memory");
        assert!(d.end());
        assert!(!d.is_open() && d.last_answer().is_empty());
        assert!(!d.end(), "nothing to end twice");
    }

    #[test]
    fn only_one_ask_at_a_time() {
        let mut d = Dialog::new();
        assert!(d.begin("a", 1).is_some());
        assert!(d.begin("b", 1).is_none());
        assert!(d.asking());
        assert!(d.abandon());
        assert!(!d.abandon());
        assert!(d.begin("b", 1).is_some());
    }

    #[test]
    fn streaming_chunks_accumulate_and_the_final_text_wins() {
        let mut d = Dialog::new();
        let id = d.begin("why", 3).unwrap().id;
        assert!(d.chunk(id, "Hel"));
        assert!(d.chunk(id, "lo, "));
        assert!(d.chunk(id, "wor"));
        assert!(!d.chunk(id + 1, "x"), "a chunk of another ask is dropped");
        assert_eq!(d.partial(), "Hello, wor");
        // the backend finishes with the complete text (may differ from the chunks: it wins)
        assert_eq!(d.finish(id, &ok("Hello, world!"), 3, Lang::En), Finished::Answered("Hello, world!".into()));
        assert_eq!(d.last_answer(), "Hello, world!");
        assert_eq!(d.partial(), "");
        // a backend that only streams: an empty final text falls back to the accumulated chunks
        let id2 = d.begin("again", 3).unwrap().id;
        d.chunk(id2, "only ");
        d.chunk(id2, "chunks");
        assert_eq!(d.finish(id2, &ok(""), 3, Lang::En), Finished::Answered("only chunks".into()));
        assert_eq!(d.history().last().unwrap().content, "only chunks");
    }

    #[test]
    fn a_moved_query_or_old_ask_makes_the_answer_stale() {
        let mut d = Dialog::new();
        let id = d.begin("q", 5).unwrap().id;
        assert_eq!(d.finish(id, &ok("a"), 6, Lang::En), Finished::Stale, "the query changed meanwhile");
        assert!(d.asking(), "a stale reply leaves the pending ask alone (the caller already abandoned or will)");
        assert_eq!(d.finish(id + 9, &ok("a"), 5, Lang::En), Finished::Stale);
        assert!(!d.is_open());
        assert_eq!(Dialog::new().finish(1, &ok("a"), 0, Lang::En), Finished::Stale);
    }

    #[test]
    fn errors_are_worded_and_link_to_settings() {
        assert!(error_text(Some("cancelled"), Lang::En).silent);
        let off = error_text(Some("net_off"), Lang::Ru);
        assert_eq!((off.text.as_str(), off.settings_page), (s::ASK_OFF.0, Some("ai")));
        assert_eq!(error_text(Some("no_endpoint"), Lang::En).text, s::ASK_NO_URL.1);
        assert_eq!(error_text(Some("no_model"), Lang::En).settings_page, Some("ai"));
        assert_eq!(error_text(None, Lang::En).text, s::ASK_FAIL.1);
        assert_eq!(error_text(Some(""), Lang::Ru).text, s::ASK_FAIL.0);
        let long = "я".repeat(400);
        assert_eq!(error_text(Some(&long), Lang::En).text.chars().count(), 300);
        assert_eq!(error_text(Some("rate limit"), Lang::En).settings_page, None);
    }

    #[test]
    fn failure_reports_whether_a_dialogue_is_open() {
        let mut d = Dialog::new();
        let id = d.begin("q", 1).unwrap().id;
        let f = d.finish(id, &AskResult::Failed(Some("net_off".into())), 1, Lang::En);
        assert!(matches!(f, Finished::Failed(_, false)));
        let id = d.begin("q", 1).unwrap().id;
        d.finish(id, &ok("a"), 1, Lang::En);
        let id = d.begin("follow", 1).unwrap().id;
        assert!(matches!(d.finish(id, &AskResult::Failed(None), 1, Lang::En), Finished::Failed(_, true)));
        assert!(d.is_open(), "a failed follow-up keeps the dialogue");
    }

    #[test]
    fn plain_text_strips_markdown_markers() {
        assert_eq!(plain_text("**Bold** and *italic* and `code`"), "Bold and italic and code");
        assert_eq!(plain_text("# Title\n\nSome text\nsecond line"), "Title\n\nSome text\nsecond line");
        assert_eq!(plain_text("- one\n- two\n\n1. a\n2. b"), "- one\n- two\n\n1. a\n2. b");
        assert_eq!(plain_text("* star item"), "- star item");
        assert_eq!(plain_text("```rust\nfn main() {}\n```\nafter"), "fn main() {}\n\nafter");
        assert_eq!(plain_text("see [docs](https://x.y/z) now"), "see docs (https://x.y/z) now");
        assert_eq!(plain_text("2 * 3 * 4"), "2 * 3 * 4", "spaced stars are not emphasis");
        assert_eq!(plain_text("**unclosed"), "**unclosed");
        assert_eq!(plain_text("  \n"), "");
        assert_eq!(plain_text("Привет **мир**\r\nвторая *строка*"), "Привет мир\nвторая строка");
    }

    #[test]
    fn ready_needs_the_opt_in_switch() {
        let mut c = Config::default();
        assert!(!ready(&c));
        c.net_ai = true;
        c.ai_base_url = "http://localhost:11434".into();
        c.ai_model = "qwen".into();
        assert!(ready(&c));
    }
}
