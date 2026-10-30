//! Note body helpers: the whitelist HTML sanitizer of tab-note.js (`sanitize`), plain-text extraction (`innerText` of the editor,
//! used for the title and for "copy text") and the empty check (`setEmptyClass`).
//!
//! Whitelist: b i s mark ul ol li div br a input[type=checkbox]. STRONG/EM/STRIKE/DEL/P/H1-H6/BLOCKQUOTE/PRE/TR are renamed,
//! script/style/iframe/form/img/... are dropped with their content, any other tag is unwrapped (its text stays), no styles,
//! classes or handlers survive, `href` only for http(s)/mailto. The output is what `innerHTML` of the JS result looked like,
//! so a note saved by the web UI and one saved here are byte-identical.
//! ponytail: a small tolerant tokenizer, not the HTML5 tree builder (no error recovery for mis-nested tags beyond closing
//! to the nearest matching open element; `li` closes an open `li`). Notes the editor itself produced are always well formed.

#[derive(Clone, Debug, PartialEq)]
enum Node {
    Text(String),
    El { tag: String, attrs: Vec<(String, String)>, kids: Vec<Node> },
}

const ALLOWED: [&str; 10] = ["b", "i", "s", "mark", "ul", "ol", "li", "div", "br", "a"];
const DROP: [&str; 24] = [
    "script", "style", "iframe", "object", "embed", "link", "meta", "template", "noscript", "svg", "math", "form", "textarea", "select", "button", "img",
    "video", "audio", "canvas", "head", "title", "xmp", "frame", "frameset",
];
const RAW: [&str; 6] = ["script", "style", "textarea", "title", "xmp", "iframe"];
const VOID: [&str; 14] = ["br", "input", "img", "hr", "meta", "link", "area", "base", "col", "embed", "source", "track", "wbr", "param"];

fn rename(tag: &str) -> &str {
    match tag {
        "strong" => "b",
        "em" => "i",
        "strike" | "del" => "s",
        "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "blockquote" | "pre" | "tr" => "div",
        t => t,
    }
}

fn decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.char_indices().take(12).find(|(_, c)| *c == ';').map(|(i, _)| i);
        let rep = end.and_then(|e| {
            let name = &rest[1..e];
            let ch = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some('\u{a0}'),
                n if n.starts_with("#x") || n.starts_with("#X") => u32::from_str_radix(&n[2..], 16).ok().and_then(char::from_u32),
                n if n.starts_with('#') => n[1..].parse::<u32>().ok().and_then(char::from_u32),
                _ => None,
            };
            ch.map(|c| (c, e))
        });
        match rep {
            Some((c, e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn parse_tag(s: &str) -> Option<(String, Vec<(String, String)>, bool, usize)> {
    // s starts after '<'; returns (name, attrs, self_closing, bytes consumed incl '>')
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' && b[i] != b'/' {
        i += 1;
    }
    if i == 0 {
        return None;
    }
    let name = s[..i].to_ascii_lowercase();
    let mut attrs = Vec::new();
    let mut selfc = false;
    loop {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            if b[i] == b'/' {
                selfc = true;
            }
            i += 1;
        }
        if i >= b.len() {
            return Some((name, attrs, selfc, b.len()));
        }
        if b[i] == b'>' {
            return Some((name, attrs, selfc, i + 1));
        }
        selfc = false;
        let st = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' && b[i] != b'>' && b[i] != b'/' {
            i += 1;
        }
        let an = s[st..i].to_ascii_lowercase();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut val = String::new();
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                let vs = i + 1;
                let ve = b[vs..].iter().position(|c| *c == q).map_or(b.len(), |p| vs + p);
                val = decode(&s[vs..ve]);
                i = (ve + 1).min(b.len());
            } else {
                let vs = i;
                while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' {
                    i += 1;
                }
                val = decode(&s[vs..i]);
            }
        }
        if !an.is_empty() {
            attrs.push((an, val));
        }
    }
}

fn parse(html: &str) -> Vec<Node> {
    // stack of open elements; index 0 is the root
    let mut stack: Vec<(String, Vec<(String, String)>, Vec<Node>)> = vec![(String::new(), vec![], vec![])];
    let mut i = 0;
    let push_text = |stack: &mut Vec<(String, Vec<(String, String)>, Vec<Node>)>, t: &str| {
        if !t.is_empty() {
            stack.last_mut().unwrap().2.push(Node::Text(decode(t)));
        }
    };
    let close_to = |stack: &mut Vec<(String, Vec<(String, String)>, Vec<Node>)>, name: &str| {
        if let Some(pos) = stack.iter().rposition(|(t, _, _)| t == name) {
            if pos > 0 {
                while stack.len() > pos {
                    let (tag, attrs, kids) = stack.pop().unwrap();
                    stack.last_mut().unwrap().2.push(Node::El { tag, attrs, kids });
                }
            }
        }
    };
    while i < html.len() {
        let Some(lt) = html[i..].find('<').map(|p| i + p) else {
            push_text(&mut stack, &html[i..]);
            break;
        };
        push_text(&mut stack, &html[i..lt]);
        let rest = &html[lt + 1..];
        if rest.starts_with("!--") {
            i = rest.find("-->").map_or(html.len(), |p| lt + 1 + p + 3);
            continue;
        }
        if rest.starts_with('!') || rest.starts_with('?') {
            i = rest.find('>').map_or(html.len(), |p| lt + 1 + p + 1);
            continue;
        }
        if let Some(r) = rest.strip_prefix('/') {
            let end = r.find('>').map_or(r.len(), |p| p);
            let name = r[..end].trim().to_ascii_lowercase();
            close_to(&mut stack, &name);
            i = lt + 2 + (end + 1).min(r.len());
            continue;
        }
        match parse_tag(rest) {
            Some((name, attrs, selfc, used)) => {
                i = lt + 1 + used;
                if RAW.contains(&name.as_str()) && !selfc {
                    let close = format!("</{name}");
                    let low = html[i..].to_ascii_lowercase();
                    let skip = low.find(&close).map_or(html.len() - i, |p| p);
                    let after = html[i + skip..].find('>').map_or(html.len(), |p| i + skip + p + 1);
                    stack.last_mut().unwrap().2.push(Node::El { tag: name, attrs, kids: vec![] });
                    i = after;
                    continue;
                }
                if name == "li" {
                    // an open <li> (inside the nearest list) ends here
                    let list = stack.iter().rposition(|(t, _, _)| t == "ul" || t == "ol").unwrap_or(0);
                    if stack.iter().skip(list + 1).any(|(t, _, _)| t == "li") {
                        close_to(&mut stack, "li");
                    }
                }
                if VOID.contains(&name.as_str()) || selfc && !ALLOWED.contains(&name.as_str()) {
                    stack.last_mut().unwrap().2.push(Node::El { tag: name, attrs, kids: vec![] });
                } else {
                    stack.push((name, attrs, vec![]));
                }
            }
            None => {
                push_text(&mut stack, "<");
                i = lt + 1;
            }
        }
    }
    while stack.len() > 1 {
        let (tag, attrs, kids) = stack.pop().unwrap();
        stack.last_mut().unwrap().2.push(Node::El { tag, attrs, kids });
    }
    stack.pop().unwrap().2
}

fn esc_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('\u{a0}', "&nbsp;")
}
fn esc_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('\u{a0}', "&nbsp;")
}

fn walk(nodes: &[Node], out: &mut String) {
    for n in nodes {
        match n {
            Node::Text(t) => out.push_str(&esc_text(t)),
            Node::El { tag, attrs, kids } => {
                if DROP.contains(&tag.as_str()) {
                    continue;
                }
                let tag = rename(tag);
                if tag == "input" {
                    let ty = attrs.iter().find(|(k, _)| k == "type").map(|(_, v)| v.to_ascii_lowercase()).unwrap_or_default();
                    if ty == "checkbox" {
                        out.push_str("<input type=\"checkbox\"");
                        if attrs.iter().any(|(k, _)| k == "checked") {
                            out.push_str(" checked=\"\"");
                        }
                        out.push('>');
                    }
                    continue;
                }
                if !ALLOWED.contains(&tag) {
                    walk(kids, out); // unknown tag: keep its content, drop the tag
                    continue;
                }
                out.push('<');
                out.push_str(tag);
                if tag == "a" {
                    let href = attrs.iter().find(|(k, _)| k == "href").map(|(_, v)| v.trim().to_string()).unwrap_or_default();
                    let l = href.to_ascii_lowercase();
                    if l.starts_with("http://") || l.starts_with("https://") || l.starts_with("mailto:") {
                        out.push_str(&format!(" href=\"{}\"", esc_attr(&href)));
                    }
                }
                out.push('>');
                if tag != "br" {
                    walk(kids, out);
                    out.push_str(&format!("</{tag}>"));
                }
            }
        }
    }
}

/// Whitelist sanitizer (see the module docs).
pub fn sanitize(html: &str) -> String {
    let mut out = String::new();
    walk(&parse(html), &mut out);
    out
}

fn text_content(nodes: &[Node], out: &mut String) {
    for n in nodes {
        match n {
            Node::Text(t) => out.push_str(t),
            Node::El { tag, kids, .. } if !RAW.contains(&tag.as_str()) => text_content(kids, out),
            _ => {}
        }
    }
}
fn has_tag(nodes: &[Node], names: &[&str]) -> bool {
    nodes.iter().any(|n| matches!(n, Node::El { tag, kids, .. } if names.contains(&tag.as_str()) || has_tag(kids, names)))
}
/// `!ed.textContent.trim() && !ed.querySelector('li,input')`
pub fn is_empty(html: &str) -> bool {
    let nodes = parse(html);
    let mut t = String::new();
    text_content(&nodes, &mut t);
    t.trim_matches(super::text::js_ws).is_empty() && !has_tag(&nodes, &["li", "input"])
}

fn inner_text(nodes: &[Node], out: &mut String) {
    let brk = |out: &mut String| {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
    };
    for n in nodes {
        match n {
            Node::Text(t) => out.push_str(&t.replace('\r', "")),
            Node::El { tag, kids, .. } => match tag.as_str() {
                "br" => out.push('\n'),
                "div" | "ul" | "ol" | "li" | "p" | "h1" | "h2" | "h3" | "blockquote" | "pre" | "tr" => {
                    brk(out);
                    inner_text(kids, out);
                    brk(out);
                }
                t if RAW.contains(&t) => {}
                _ => inner_text(kids, out),
            },
        }
    }
}
/// `ed.innerText` of the (sanitized) body: block elements and `<br>` become line breaks.
pub fn plain_text(html: &str) -> String {
    let mut out = String::new();
    inner_text(&parse(html), &mut out);
    out
}
/// First non-blank trimmed line of the note (its title source).
pub fn first_line(html: &str) -> String {
    plain_text(html).split('\n').map(|l| l.trim_matches(super::text::js_ws)).find(|l| !l.is_empty()).unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_whitelist() {
        assert_eq!(sanitize("<b>a</b><i>b</i><s>c</s><mark>d</mark>"), "<b>a</b><i>b</i><s>c</s><mark>d</mark>");
        assert_eq!(sanitize("<ul><li>x</li><li>y</li></ul><ol><li>z</li></ol>"), "<ul><li>x</li><li>y</li></ul><ol><li>z</li></ol>");
        assert_eq!(sanitize("a<br>b<div>c</div>"), "a<br>b<div>c</div>");
        assert_eq!(sanitize("<input type=\"checkbox\" checked>&nbsp;todo"), "<input type=\"checkbox\" checked=\"\">&nbsp;todo");
        assert_eq!(sanitize("<input type=\"checkbox\">x"), "<input type=\"checkbox\">x");
    }
    #[test]
    fn renames_and_unwraps() {
        assert_eq!(sanitize("<strong>a</strong><em>b</em><strike>c</strike><del>d</del>"), "<b>a</b><i>b</i><s>c</s><s>d</s>");
        assert_eq!(sanitize("<h1>T</h1><p>p</p><blockquote>q</blockquote><pre>c</pre>"), "<div>T</div><div>p</div><div>q</div><div>c</div>");
        assert_eq!(sanitize("<span style=\"color:red\">x</span><font>y</font><table><tr><td>z</td></tr></table>"), "xy<div>z</div>");
    }
    #[test]
    fn drops_dangerous_content_with_its_text() {
        assert_eq!(sanitize("a<script>alert(1)</script>b"), "ab");
        assert_eq!(sanitize("a<style>p{}</style><iframe src=x></iframe><img src=x onerror=y>b"), "ab");
        assert_eq!(sanitize("<form><button>go</button><textarea>t</textarea></form>ok"), "ok");
        assert_eq!(sanitize("<input type=\"text\" value=\"a\">x"), "x");
        assert_eq!(sanitize("<div onclick=\"x()\" class=\"c\" style=\"s\">t</div>"), "<div>t</div>");
        assert_eq!(sanitize("<svg><circle/></svg>a<!-- c -->b"), "ab");
    }
    #[test]
    fn links_only_http_https_mailto() {
        assert_eq!(sanitize("<a href=\" https://x.io/a?b=1&c=2 \">l</a>"), "<a href=\"https://x.io/a?b=1&amp;c=2\">l</a>");
        assert_eq!(sanitize("<a href=\"MAILTO:a@b.c\">m</a>"), "<a href=\"MAILTO:a@b.c\">m</a>");
        assert_eq!(sanitize("<a href=\"javascript:alert(1)\">j</a>"), "<a>j</a>");
        assert_eq!(sanitize("<a href=\"data:text/html,x\" target=_blank>d</a>"), "<a>d</a>");
    }
    #[test]
    fn entities_roundtrip() {
        assert_eq!(sanitize("a &lt; b &amp; c &gt; d &quot;q&quot; &#39;s&#39; &#x41;"), "a &lt; b &amp; c &gt; d \"q\" 's' A");
        assert_eq!(sanitize("x&nbsp;y"), "x&nbsp;y");
        assert_eq!(sanitize("1 < 2"), "1 &lt; 2");
        assert_eq!(sanitize("Привет 😀"), "Привет 😀");
    }
    #[test]
    fn misnested_and_unclosed() {
        assert_eq!(sanitize("<b>a<i>b</b>c</i>"), "<b>a<i>b</i></b>c");
        assert_eq!(sanitize("<ul><li>a<li>b</ul>"), "<ul><li>a</li><li>b</li></ul>");
        assert_eq!(sanitize("<div>open"), "<div>open</div>");
        assert_eq!(sanitize(""), "");
    }
    #[test]
    fn idempotent() {
        let once = sanitize("<p><strong>Hi</strong> <a href=\"https://a.b\">x</a><script>1</script><input type=checkbox checked> t</p>");
        assert_eq!(sanitize(&once), once);
    }
    #[test]
    fn plain_text_and_first_line() {
        assert_eq!(plain_text("<div>one</div><div>two</div>").trim(), "one\ntwo");
        assert_eq!(plain_text("a<br>b").trim(), "a\nb");
        assert_eq!(plain_text("<ul><li>x</li><li>y</li></ul>").trim(), "x\ny");
        assert_eq!(first_line("<div><br></div><div>  Заголовок  </div><div>body</div>"), "Заголовок");
        assert_eq!(first_line("<div>&nbsp;</div>"), "");
        assert_eq!(first_line(""), "");
    }
    #[test]
    fn emptiness() {
        assert!(is_empty(""));
        assert!(is_empty("<div><br></div>"));
        assert!(is_empty("<div>&nbsp; </div>"));
        assert!(!is_empty("<div>x</div>"));
        assert!(!is_empty("<ul><li></li></ul>"));
        assert!(!is_empty("<input type=\"checkbox\">"));
    }
}
