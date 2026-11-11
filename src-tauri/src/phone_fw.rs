// Windows Firewall for the Phone link. The phone reaches Pill on the PC's LAN address; on a network Windows calls "Public" (a wired network is
// often classified so) an inbound connection is blocked unless a rule allows the program. When "Phone link" is switched on and no rule for THIS
// pill.exe exists, Pill asks once (a UAC prompt, the user's consent: Pill never elevates itself) to add the rule "Pill Phone link":
// inbound, TCP, this program only, remote addresses LocalSubnet only (the local network, never the internet), all profiles.
// Switching the link off and on again asks again (after a dismissed prompt). `netsh` is the only tool used; nothing is changed without consent.
use std::os::windows::process::CommandExt;
use std::sync::atomic::{AtomicBool, Ordering};

const RULE: &str = "Pill Phone link";
const NO_WINDOW: u32 = 0x0800_0000;
static ASKED: AtomicBool = AtomicBool::new(false);

/// The elevated command line: replace any older rule of that name (a moved install), then add the right one.
pub fn add_rule_cmd(exe: &str) -> String {
    format!(
        "/c netsh advfirewall firewall delete rule name=\"{RULE}\" & netsh advfirewall firewall add rule name=\"{RULE}\" dir=in action=allow program=\"{exe}\" protocol=TCP remoteip=LocalSubnet profile=any enable=yes"
    )
}

/// Does the `netsh ... show rule ... verbose` output describe an enabled allow-rule for exactly this program? (Language independent: it looks for the path.)
pub fn rule_covers(output: &str, exe: &str) -> bool {
    let o = output.to_lowercase();
    o.contains(&exe.to_lowercase()) && o.contains("localsubnet") && o.contains("allow")
}

fn exe_path() -> Option<String> {
    std::env::current_exe().ok().map(|p| p.to_string_lossy().trim_start_matches(r"\\?\").to_string())
}

/// True when a rule for this very pill.exe already allows the LAN.
pub fn rule_ok() -> bool {
    let Some(exe) = exe_path() else { return true };
    let out = std::process::Command::new("netsh").args(["advfirewall", "firewall", "show", "rule", &format!("name={RULE}"), "verbose"]).creation_flags(NO_WINDOW).output();
    match out {
        Ok(o) if o.status.success() => rule_covers(&String::from_utf8_lossy(&o.stdout), &exe),
        Ok(_) => false, // "No rules match": exit code 1
        Err(_) => true, // no netsh: nothing to do
    }
}

fn shell_runas(file: &str, params: &str) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
    let w = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (v, f, p) = (w("runas"), w(file), w(params));
    let r = unsafe { ShellExecuteW(HWND::default(), PCWSTR(v.as_ptr()), PCWSTR(f.as_ptr()), PCWSTR(p.as_ptr()), PCWSTR::null(), SW_HIDE) };
    r.0 as isize > 32
}

/// "Phone link" was switched on: if the LAN is not allowed yet, ask once (in the background; the UAC prompt is the user's decision).
pub fn ensure_async() {
    if ASKED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let Some(exe) = exe_path() else { return };
        if rule_ok() {
            return;
        }
        eprintln!("phone: no firewall rule for the LAN yet, asking Windows to add one (UAC)");
        if !shell_runas("cmd.exe", &add_rule_cmd(&exe)) {
            eprintln!("phone: the firewall rule was not added (prompt declined)");
        }
    });
}

/// "Phone link" was switched off: the next switch-on may ask again.
pub fn reset() {
    ASKED.store(false, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rule_is_inbound_tcp_for_this_program_and_the_local_subnet_only() {
        let c = add_rule_cmd(r"C:\Users\x\AppData\Local\Pill\pill.exe");
        assert!(c.starts_with("/c netsh advfirewall firewall delete rule name=\"Pill Phone link\" & netsh advfirewall firewall add rule"));
        for must in ["dir=in", "action=allow", "protocol=TCP", "remoteip=LocalSubnet", "program=\"C:\\Users\\x\\AppData\\Local\\Pill\\pill.exe\""] {
            assert!(c.contains(must), "{must} in {c}");
        }
        assert!(!c.contains("remoteip=any") && !c.to_lowercase().contains("0.0.0.0"), "never open to the internet");
    }

    #[test]
    fn an_existing_rule_counts_only_for_this_exact_program() {
        let out = "Rule Name:   Pill Phone link\nEnabled: Yes\nDirection: In\nProgram: C:\\Users\\x\\AppData\\Local\\Pill\\pill.exe\nRemoteIP: LocalSubnet\nAction: Allow\n";
        assert!(rule_covers(out, r"C:\Users\x\AppData\Local\Pill\pill.exe"));
        assert!(rule_covers(out, r"c:\users\X\appdata\local\pill\PILL.EXE"), "case does not matter");
        assert!(!rule_covers(out, r"D:\Other\pill.exe"), "a moved install needs a new rule");
        assert!(!rule_covers("Program: C:\\p\\pill.exe\nRemoteIP: Any\nAction: Allow", r"C:\p\pill.exe"), "an open-to-everyone rule is not ours");
    }
}
