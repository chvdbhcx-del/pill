//! AI setup: the local chat card (servers found on this computer, the Ollama installer and model catalog).
use super::page::RowRt;
use super::rows::*;
use super::ui::{key, Act};
use super::SettingsView;
use glasskit::controls::ButtonKind;
use glasskit::geom::Rect;
use glasskit::text::Style;
use pill_ui_core::settings::aisetup::{self as ais, OllamaStep};
use pill_ui_core::settings::flows;
use pill_ui_core::settings::tr::pick;
use serde_json::Value;

fn arr<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}
fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

impl SettingsView {
    pub(super) fn w_chat(&mut self, d: &Dc, r: &RowRt, full: Rect, paint: bool) -> f32 {
        let (cv, p) = (d.cv, d.p);
        let (x0, w) = (full.x + PAD_X, full.w - 2.0 * PAD_X);
        let m = self.row_metrics_plain(d, r, w);
        let st = self.data.status.clone();
        if st.is_null() {
            if paint {
                self.paint_label(d, r, &m, x0, full.y);
            }
            return m.nh;
        }
        // the title row; its hint is replaced by what was found
        let (line, found) = ais::servers_line(d.lang, st.get("servers").unwrap_or(&Value::Null));
        if paint {
            cv.text(Rect::new(x0, full.y + 11.0, w, 18.0), r.row.label.get(d.lang), st_title(), p.text);
        }
        let mut y = full.y + 11.0 + 18.0 + 2.0;
        y += self.para_at(d, paint, x0, y, w, &line, st_hint(), if found { p.green } else { p.dim }) + 10.0;
        let ai = st.get("ai").cloned().unwrap_or(Value::Null);
        for sv in arr(&st, "servers").iter().filter(|x| x.get("up").and_then(Value::as_bool).unwrap_or(false)) {
            let models: Vec<&str> = arr(sv, "models").iter().filter_map(Value::as_str).collect();
            let cur = s(&ai, "model");
            let model = if models.contains(&cur) { cur } else { models.first().copied().unwrap_or("") };
            let url = s(sv, "url");
            let used = ais::used_here(&ai, url.trim_end_matches('/')) && model == cur;
            if paint {
                cv.fill_rect(Rect::new(full.x, y, full.w, 1.0), p.row_line);
                cv.text(Rect::new(x0, y + 8.0, w - 120.0, 18.0), s(sv, "name"), st_title(), p.text);
                cv.text(Rect::new(x0, y + 26.0, w - 120.0, 16.0), &ais::server_sub(d.lang, models.len(), url), st_hint(), p.dim);
            }
            let label = ais::use_button_label(d.lang, used);
            self.btn_at(d, paint, &format!("srv:{}", s(sv, "id")), label, if used { ButtonKind::Normal } else { ButtonKind::Primary }, 0.0, Some(full.right() - PAD_X), y + 9.0, Act::ServerUse(s(sv, "id").to_string(), model.to_string()), !used);
            y += 50.0;
        }
        // Ollama
        let ol = st.get("ollama").cloned().unwrap_or(Value::Null);
        if paint {
            cv.fill_rect(Rect::new(full.x, y, full.w, 1.0), p.row_line);
        }
        y += 12.0;
        if paint {
            cv.text(Rect::new(x0, y, w, 18.0), "Ollama", st_title(), p.text);
        }
        y += 20.0;
        y += self.para_at(d, paint, x0, y, w, &ais::ollama_hint(d.lang, &ol), st_hint(), p.dim) + 8.0;
        let busy = self.data.job.is_some();
        match ais::ollama_step(d.lang, &ol) {
            OllamaStep::StartIt(t) => y += self.para_at(d, paint, x0, y, w, t.get(d.lang), st_hint(), p.amber) + 4.0,
            OllamaStep::RunInstaller { label, .. } => {
                self.btn_at(d, paint, "ol:primary", label, ButtonKind::Primary, x0, None, y, Act::OllamaPrimary, !busy);
                y += 32.0 + 4.0;
            }
            OllamaStep::DownloadInstaller { label, .. } => {
                self.btn_at(d, paint, "ol:primary", &label, ButtonKind::Primary, x0, None, y, Act::OllamaPrimary, !busy);
                y += 32.0 + 4.0;
            }
            OllamaStep::Catalog => {
                let servers = st.get("servers").cloned().unwrap_or(Value::Null);
                for c in arr(&ol, "catalog") {
                    let name = s(c, "name");
                    let have = ais::have_model(&servers, name);
                    if paint {
                        cv.text(Rect::new(x0, y, w - 120.0, 36.0), &format!("{name}  ·  {}", flows::size(d.lang, c.get("size").and_then(Value::as_f64).unwrap_or(0.0))), Style::new(13.0, 400), p.item);
                    }
                    let label = if have { pick(d.lang, "Скачано", "Pulled") } else { pick(d.lang, "Скачать", "Pull") };
                    self.btn_at(d, paint, &format!("pull:{name}"), label, ButtonKind::Normal, 0.0, Some(full.right() - PAD_X), y + 2.0, Act::ModelPull(name.to_string()), !have && !busy);
                    y += 38.0;
                }
            }
        }
        let jid = self.data.job.clone().unwrap_or_default();
        if jid == "ollama-setup" || jid == "ollama-pull" {
            if paint {
                self.job_box(d, x0, y + 4.0, w);
            }
            y += 12.0 + 44.0;
        } else if let Some((id, msg)) = self.data.job_result.clone() {
            if id.starts_with("ollama") {
                y += self.para_at(d, paint, x0, y, w, &msg.text, st_hint(), msg_color(&p, msg.kind)) + 4.0;
            }
        }
        y + 12.0 - full.y
    }
}

#[allow(dead_code)]
fn _k() -> u64 {
    key("x")
}
