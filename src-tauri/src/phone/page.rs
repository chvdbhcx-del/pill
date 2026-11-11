// `GET /p`: the page a phone camera opens after scanning the QR code. One self-contained HTML document (no scripts, no external requests): the
// app's look from docs/phone-protocol.md (dark, Pill violet, pill-shaped buttons), a droplet mark, two big buttons.
use super::http::pct_encode;

pub fn esc(s: &str) -> String {
    s.chars().fold(String::new(), |mut o, c| {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c => o.push(c),
        }
        o
    })
}

/// The deep link the "Open in Pill Phone" button uses (Android intent URL; `pillphone://` is the fallback).
pub fn intent_url(ip: &str, port: u16, code: &str, pc: &str) -> String {
    format!("intent://pair?h={}&p={port}&c={}&n={}#Intent;scheme=pillphone;package=app.pill.phone;end", pct_encode(ip), pct_encode(code), pct_encode(pc))
}
pub fn scheme_url(ip: &str, port: u16, code: &str, pc: &str) -> String {
    format!("pillphone://pair?h={}&p={port}&c={}&n={}", pct_encode(ip), pct_encode(code), pct_encode(pc))
}

struct Text {
    title: &'static str,
    lead: &'static str,
    get: &'static str,
    open: &'static str,
    step1: &'static str,
    step2: &'static str,
    retry: &'static str,
    note: &'static str,
    web: &'static str,
    web_hint: &'static str,
    web_small: &'static str,
}

const RU: Text = Text {
    title: "Pill Phone",
    lead: "Уведомления телефона на острове Pill и файлы между телефоном и ПК.",
    get: "Скачать приложение",
    open: "Открыть в Pill Phone",
    step1: "Установите приложение (один раз)",
    step2: "Откройте его этой кнопкой, чтобы подключиться к",
    retry: "Не открылось? Попробуйте эту ссылку",
    note: "Связь идёт только по вашей домашней сети. Код действует 5 минут.",
    web: "Открыть в браузере",
    web_hint: "На iPhone приложения нет: Pill работает прямо в Safari (файлы между телефоном и ПК, без уведомлений). Лучше сначала добавить страницу на экран «Домой».",
    web_small: "iPhone или iPad? Открыть в браузере",
};
const EN: Text = Text {
    title: "Pill Phone",
    lead: "Your phone's notifications on the Pill island, and files between phone and PC.",
    get: "Download the app",
    open: "Open in Pill Phone",
    step1: "Install the app (once)",
    step2: "Open it with this button to connect to",
    retry: "Did not open? Try this link",
    note: "Everything stays on your home network. The code is valid for 5 minutes.",
    web: "Open in browser",
    web_hint: "There is no iPhone app: Pill runs right in Safari (files between phone and PC, no notifications). Best: add the page to your Home Screen first.",
    web_small: "On iPhone or iPad? Open in browser",
};

/// iPhone / iPad Safari (no Android app there): the page offers the browser client instead.
pub fn is_ios(user_agent: &str) -> bool {
    user_agent.contains("iPhone") || user_agent.contains("iPad")
}

/// `/w` with the pairing code and the address, for the "Open in browser" button (the web client, docs/phone-protocol.md).
pub fn web_url(ip: &str, port: u16, code: &str) -> String {
    if code.is_empty() {
        "/w".to_string()
    } else {
        format!("/w?c={}&h={}&p={port}", pct_encode(code), pct_encode(ip))
    }
}

pub fn render(lang: &str, pc: &str, ip: &str, port: u16, code: &str, apk: &str, ios: bool) -> String {
    let t = if lang == "ru" { &RU } else { &EN };
    // the code is shown to nobody and only goes into the links; a malformed one is dropped instead of echoed
    let code = if super::reg::valid_code_shape(code) { code } else { "" };
    let open = esc(&intent_url(ip, port, code, pc));
    let scheme = esc(&scheme_url(ip, port, code, pc));
    let web = esc(&web_url(ip, port, code));
    let open_btn = if ios || code.is_empty() {
        String::new()
    } else {
        format!(r#"<a class="btn ghost" href="{open}"><span class="n">2</span>{}</a><p class="hint">{} <b>{}</b></p><a class="small" href="{scheme}">{}</a>"#, esc(t.open), esc(t.step2), esc(pc), esc(t.retry))
    };
    format!(
        r##"<!doctype html>
<html lang="{lang}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<meta name="color-scheme" content="dark light"><meta name="robots" content="noindex"><title>Pill Phone</title>
<style>
:root{{--bg:#0B0B0F;--surface:#15151C;--line:#FFFFFF14;--text:#F2F2F7;--dim:#9A9AA8;--accent:#8E7BFF;--soft:#B9AEFF;--on:#0B0B0F}}
@media (prefers-color-scheme:light){{:root{{--bg:#F4F3FA;--surface:#FFFFFF;--line:#0000001A;--text:#16151F;--dim:#6C6B7B;--accent:#6F5BF0;--soft:#5A47D6;--on:#FFFFFF}}}}
*{{box-sizing:border-box;margin:0}}
body{{min-height:100vh;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:24px;padding:max(24px,env(safe-area-inset-top)) 20px max(24px,env(safe-area-inset-bottom));background:radial-gradient(120% 60% at 50% 0,#8E7BFF26,transparent 60%),var(--bg);color:var(--text);font:500 16px/1.45 system-ui,-apple-system,"Segoe UI",Roboto,sans-serif}}
.card{{width:100%;max-width:420px;background:var(--surface);border:1px solid var(--line);border-radius:28px;padding:32px 24px 24px;text-align:center;box-shadow:0 24px 60px #0006}}
.logo{{width:84px;height:84px;margin:0 auto 18px;display:block;filter:drop-shadow(0 8px 24px #8E7BFF66)}}
h1{{font-size:28px;font-weight:700;letter-spacing:-.02em}}
.lead{{color:var(--dim);margin:8px 0 26px}}
.btn{{display:flex;align-items:center;justify-content:center;gap:12px;width:100%;min-height:60px;border-radius:999px;font-size:18px;font-weight:700;text-decoration:none;color:var(--on);background:linear-gradient(135deg,var(--soft),var(--accent));box-shadow:0 10px 28px #8E7BFF55;transition:transform .2s cubic-bezier(.34,1.56,.64,1)}}
.btn:active{{transform:scale(.97)}}
.btn.ghost{{color:var(--text);background:transparent;border:1.5px solid var(--accent);box-shadow:none;margin-top:14px}}
.n{{display:inline-grid;place-items:center;width:26px;height:26px;border-radius:50%;background:#FFFFFF2E;font-size:14px}}
.btn.ghost .n{{background:var(--accent);color:var(--on)}}
.hint{{color:var(--dim);font-size:14px;margin:12px 0 4px}}.hint b{{color:var(--text);font-weight:600}}
.small{{display:inline-block;color:var(--soft);font-size:14px;margin-top:6px}}
.note{{max-width:420px;color:var(--dim);font-size:13px;text-align:center}}
</style></head><body>
<main class="card">
<svg class="logo" viewBox="0 0 96 96" role="img" aria-label="Pill"><defs><linearGradient id="g" x1="20%" y1="0" x2="80%" y2="100%"><stop offset="0" stop-color="#B9AEFF"/><stop offset="1" stop-color="#8E7BFF"/></linearGradient></defs>
<path d="M48 8C48 8 20 38 20 60a28 28 0 0 0 56 0C76 38 48 8 48 8Z" fill="url(#g)"/><ellipse cx="37" cy="58" rx="6" ry="11" fill="#fff" opacity=".28" transform="rotate(14 37 58)"/></svg>
<h1>{title}</h1><p class="lead">{lead}</p>
{buttons}
{open_btn}
</main>
{web_link}<p class="note">{note}</p>
</body></html>"##,
        lang = if lang == "ru" { "ru" } else { "en" },
        buttons = if ios {
            format!(r#"<a class="btn" href="{web}">{}</a><p class="hint">{}</p>"#, esc(t.web), esc(t.web_hint))
        } else {
            format!(r#"<a class="btn" href="{}"><span class="n">1</span>{}</a><p class="hint">{}</p>"#, esc(apk), esc(t.get), esc(t.step1))
        },
        title = esc(t.title),
        lead = esc(t.lead),
        note = esc(t.note),
        web_link = if ios { String::new() } else { format!(r#"<a class="small" href="{web}">{}</a>"#, esc(t.web_small)) },
    )
}

// ------------------------------------------------------------------------------------------------ the web client (`GET /w`)

/// The browser client for iPhone (docs/phone-protocol.md, "Web client"): one self-contained document. It contains nothing of the PC's: the name,
/// the token and every file name arrive over the API and go in with `textContent`. The only substitution is the per-request script nonce.
const WEB_HTML: &str = include_str!("web/w.html");
/// 180x180 apple-touch-icon (the droplet mark, violet on dark); iOS ignores SVG icons.
pub const WEB_ICON: &[u8] = include_bytes!("web/icon.png");
pub const WEB_MANIFEST: &str = r##"{"name":"Pill Phone","short_name":"Pill","id":"/w","start_url":"/w","scope":"/w","display":"standalone","orientation":"portrait","background_color":"#0B0B0F","theme_color":"#0B0B0F","description":"Files between your phone and your PC","icons":[{"src":"/w/icon.png","sizes":"180x180","type":"image/png","purpose":"any"}]}"##;

pub fn web_page(nonce: &str) -> String {
    WEB_HTML.replace("__NONCE__", nonce)
}

/// Inline script only with this request's nonce; styles may be inline; the page talks to its own origin and its WebSocket only.
pub fn web_csp(nonce: &str) -> String {
    format!("default-src 'none'; script-src 'nonce-{nonce}'; style-src 'unsafe-inline'; img-src 'self' data:; connect-src 'self' ws:; manifest-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_has_both_buttons_the_logo_and_the_deep_link() {
        let h = render("en", "DESKTOP-X", "192.168.1.5", 41717, "AB3D9XKQ", "https://example.org/PillPhone.apk", false);
        assert!(h.contains("Download the app") && h.contains("Open in Pill Phone"));
        assert!(h.contains("<svg") && h.contains("href=\"https://example.org/PillPhone.apk\""));
        assert!(h.contains("intent://pair?h=192.168.1.5&amp;p=41717&amp;c=AB3D9XKQ&amp;n=DESKTOP-X#Intent;scheme=pillphone;package=app.pill.phone;end"));
        assert!(h.contains("pillphone://pair?h=192.168.1.5&amp;p=41717&amp;c=AB3D9XKQ&amp;n=DESKTOP-X"));
        assert!(!h.contains("<script"));
    }

    #[test]
    fn iphone_gets_the_browser_client_as_the_main_button_and_android_a_small_link() {
        let ios = render("en", "DESKTOP-X", "192.168.1.5", 41717, "AB3D9XKQ", "https://example.org/PillPhone.apk", true);
        assert!(ios.contains("Open in browser") && ios.contains("href=\"/w?c=AB3D9XKQ&amp;h=192.168.1.5&amp;p=41717\""));
        assert!(!ios.contains("Download the app") && !ios.contains("intent://") && !ios.contains("PillPhone.apk"));
        let and = render("ru", "DESKTOP-X", "192.168.1.5", 41717, "AB3D9XKQ", "https://example.org/PillPhone.apk", false);
        assert!(and.contains("Скачать приложение") && and.contains("iPhone или iPad? Открыть в браузере") && and.contains("href=\"/w?c=AB3D9XKQ"));
        assert!(render("en", "PC", "10.0.0.2", 41717, "", "https://e.org/a.apk", true).contains("href=\"/w\""));
        assert!(is_ios("Mozilla/5.0 (iPhone; CPU iPhone OS 17_2 like Mac OS X)") && is_ios("Mozilla/5.0 (iPad; CPU OS 16_0)") && !is_ios("Mozilla/5.0 (Linux; Android 15)"));
    }

    #[test]
    fn russian_by_language_and_everything_is_escaped() {
        let h = render("ru", "<img src=x onerror=alert(1)>", "10.0.0.2", 41717, "AB3D9XKQ", "https://e.org/a\"b", false);
        assert!(h.contains("Скачать приложение") && h.contains("Открыть в Pill Phone"));
        assert!(!h.contains("<img src=x"));
        assert!(!h.contains("a\"b"));
    }

    #[test]
    fn a_malformed_code_is_never_echoed() {
        let h = render("en", "PC", "10.0.0.2", 41717, "<b>bad</b>", "https://e.org/a.apk", false);
        assert!(!h.contains("bad") && !h.contains("intent://"), "no deep link without a valid code");
        assert!(h.contains("Download the app"));
    }
}
