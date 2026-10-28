// `pill.exe --open <target>`: a second launch (for example the widgets "Agents" widget click) is forwarded to the running instance by
// tauri-plugin-single-instance and lands in `handle` there; the second process then exits. Contract: the ONLY accepted form is
// `--open agents` or `--open settings` (also `--open=agents`), case-insensitive; every other argument is ignored, nothing is executed.
use tauri::AppHandle;

#[derive(Debug, PartialEq, Eq)]
enum Target {
    Agents,
    Settings,
}

/// `args` is the full argv (program name first). The first `--open` decides; an unknown or missing value yields None.
fn parse_open(args: &[String]) -> Option<Target> {
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        let value = if a.eq_ignore_ascii_case("--open") { it.next().map(String::as_str) } else { a.get(..7).filter(|p| p.eq_ignore_ascii_case("--open=")).map(|_| &a[7..]) };
        if let Some(v) = value {
            return match v.to_ascii_lowercase().as_str() {
                "agents" => Some(Target::Agents),
                "settings" => Some(Target::Settings),
                _ => None,
            };
        }
    }
    None
}

/// Runs in the already running instance when another one was started.
pub fn handle(app: &AppHandle, args: Vec<String>) {
    match parse_open(&args) {
        Some(Target::Agents) => crate::pill::expand(app, Some("agents".into()), "open"),
        Some(Target::Settings) => crate::settingswin::open(app, None),
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(a: &[&str]) -> Option<Target> {
        parse_open(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn accepts_only_the_two_fixed_words() {
        assert_eq!(p(&["pill.exe", "--open", "agents"]), Some(Target::Agents));
        assert_eq!(p(&["pill.exe", "--open=Settings"]), Some(Target::Settings));
        assert_eq!(p(&["pill.exe", "--other", "--OPEN", "AGENTS"]), Some(Target::Agents));
    }

    #[test]
    fn everything_else_is_ignored() {
        for a in [&["pill.exe"][..], &["pill.exe", "--open"], &["pill.exe", "--open", "calc.exe"], &["pill.exe", "--open=", "agents"], &["pill.exe", "--open", ""], &["pill.exe", "agents"], &["--open", "agents"], &["pill.exe", "--open", "agents "], &["pill.exe", "--öpen=agents"]] {
            assert_eq!(p(a), None, "{a:?}");
        }
    }
}
