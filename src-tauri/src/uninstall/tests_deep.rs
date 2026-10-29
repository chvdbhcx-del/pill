// Deep-clean tests: evidence tiers, the wider guard, registry / service / task / firewall evidence from fixtures, the elevated-request validation,
// backup-before-delete ordering. Fixtures are plain structs, temp directories and one throw-away HKCU key; nothing real is touched.
use super::tests::{fake, prog, sv, Tmp};
use super::*;
use std::cell::RefCell;
use std::io::BufReader;
use std::net::{TcpListener, TcpStream};
use base64::Engine;

// ---------------------------------------------------------------- fixtures

/// A registry in memory. Key names keep their case, value names are lower-cased like the real reader returns them.
#[derive(Default)]
struct FakeReg {
    keys: Vec<(Hive, String, Vals)>,
}
impl FakeReg {
    fn key(&mut self, h: Hive, sub: &str, vals: &[(&str, Val)]) -> &mut Self {
        let mut p = String::new();
        for part in sub.split('\\') {
            if !p.is_empty() {
                p.push('\\');
            }
            p.push_str(part);
            if !self.keys.iter().any(|(hh, s, _)| *hh == h && s.eq_ignore_ascii_case(&p)) {
                self.keys.push((h, p.clone(), Vals::new()));
            }
        }
        let k = self.keys.iter_mut().find(|(hh, s, _)| *hh == h && s.eq_ignore_ascii_case(sub)).unwrap();
        for (n, v) in vals {
            k.2.insert(n.to_lowercase(), v.clone());
        }
        self
    }
}
impl Reg for FakeReg {
    fn subkeys(&self, h: Hive, sub: &str) -> Vec<String> {
        let pre = format!("{}\\", sub.to_lowercase());
        let mut out: Vec<String> = Vec::new();
        for (hh, s, _) in &self.keys {
            if *hh == h && s.to_lowercase().starts_with(&pre) && !s[pre.len()..].contains('\\') {
                out.push(s[pre.len()..].to_string());
            }
        }
        out
    }
    fn values(&self, h: Hive, sub: &str) -> Vals {
        self.keys.iter().find(|(hh, s, _)| *hh == h && s.eq_ignore_ascii_case(sub)).map(|k| k.2.clone()).unwrap_or_default()
    }
    fn exists(&self, h: Hive, sub: &str) -> bool {
        self.keys.iter().any(|(hh, s, _)| *hh == h && s.eq_ignore_ascii_case(sub))
    }
}

const G: &str = "{0A1B2C3D-4E5F-6789-ABCD-EF0123456789}";
const OTHER_G: &str = "{11111111-2222-3333-4444-555555555555}";

/// "Foo Editor" by "Acme Inc", installed in `D:\Apps\Foo Editor`, product code G.
fn foo() -> Prog {
    let mut p = prog("Foo Editor", "Acme Inc", r"D:\Apps\Foo Editor");
    p.sub = format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{G}");
    p.msi = Some(G.to_string());
    p
}

fn denv(t: &Tmp) -> Env {
    let mut e = t.env();
    e.drives = vec![t.dir("d")];
    e.others = vec![t.dir("ou")];
    for d in [r"ou\AppData\Roaming", r"ou\AppData\Local", r"ou\Documents", r"ou\Downloads", r"u\Documents", r"u\Downloads", r"u\Saved Games", r"u\AppData\LocalLow", r"u\AppData\Local\Temp", r"pf\Common Files", r"win\Temp"] {
        t.dir(d);
    }
    e
}

fn ok(w: &What, p: &Prog, env: &Env, reg: &dyn Reg) -> Option<(Tier, &'static str)> {
    guard_cand(w, p, env, reg).ok()
}

// ---------------------------------------------------------------- names and tiers

#[test]
fn name_tiers_with_look_alikes() {
    let n = names_of(&prog("Foo Editor", "Acme Inc", ""));
    let t = |s: &str| name_tier(s, &n).map(|x| x.0);
    for s in ["Foo Editor", "FooEditor", "foo-editor", "FOO_EDITOR", "Foo  Editor"] {
        assert_eq!(t(s), Some(Tier::Certain), "{s}");
    }
    for s in ["Foo Editor Data", "FooEditorData", "Microsoft Foo Editor", "foo-editor-cache", "My Foo Editor 2"] {
        assert_eq!(t(s), Some(Tier::Probable), "{s}");
    }
    assert_eq!(t("Acme"), Some(Tier::Probable)); // the publisher's own name
    assert_eq!(t("FooEditorPro"), Some(Tier::Probable)); // Foo / Editor / Pro: the full name as words, but maybe another product
    for s in ["Foo", "FooBar", "Foo-Bar", "Editor", "Fooeditors", "Foo Edit", "Acmeish", "Data", "Microsoft", ""] {
        assert_eq!(t(s), None, "{s}");
    }
    // a 3-letter name has no Probable tier at all: every "Foo-Something" would match
    let short = names_of(&prog("Foo", "", ""));
    assert_eq!(name_tier("Foo", &short).map(|x| x.0), Some(Tier::Certain));
    assert_eq!(name_tier("Foo-Bar", &short), None);
    assert_eq!(name_tier("Microsoft Foo", &short), None);
    // generic names never match in either tier
    let tools = names_of(&prog("Tools", "", ""));
    assert_eq!(name_tier("Tools", &tools), None);
    assert_eq!(name_tier("Data Tools", &tools), None);
    assert_eq!(tokens("FooEditor 2-Beta_x64"), vec!["foo", "editor", "2", "beta", "x64"]);
}

#[test]
fn installer_file_names() {
    let n = names_of(&prog("Foo Editor", "", ""));
    for f in ["FooEditor-Setup-1.2.3.exe", "foo_editor_installer_x64.msi", "Foo Editor Setup.exe", "FooEditor_Install.exe", "fooeditor-setup-win64-v2.exe"] {
        assert!(installer_name(f, &n), "{f}");
    }
    for f in ["FooEditor.exe", "FooEditorPro-Setup.exe", "Foo Editor Setup.zip", "notes-setup.exe", "Setup.exe", "FooEditor-Setup-beta-leak.exe", "FooEditor-Setup", "bar-FooEditor-Setup.exe"] {
        assert!(!installer_name(f, &n), "{f}");
    }
}

#[test]
fn packed_guids_and_paths() {
    assert_eq!(pack_guid("{AC76BA86-7AD7-1033-7B44-AC0F074E4100}").as_deref(), Some("68AB67CA7DA73301B744CAF070E41400"));
    assert_eq!(pack_guid("nonsense"), None);
    let i = Path::new(r"D:\Apps\Foo Editor");
    assert!(points_into(r#""D:\Apps\Foo Editor\foo.exe" --tray"#, Some(i)));
    assert!(points_into(r"d:\apps\foo editor\sub\x.dll", Some(i)));
    assert!(points_into(r"\??\D:\Apps\Foo Editor\drv.sys", Some(i)));
    assert!(!points_into(r"D:\Apps\Foo Editor Pro\foo.exe", Some(i)));
    assert!(!points_into(r"D:\Apps\Foo Editor\..\x.exe", Some(i)));
    assert!(!points_into(r"C:\Windows\explorer.exe", Some(i)));
    assert!(!points_into(r"D:\Apps\Foo Editor\foo.exe", None));
    assert!(!points_into("", Some(i)));
}

// ---------------------------------------------------------------- the wider file guard

#[test]
fn deep_guard_accepts_named_folders_in_the_wider_places() {
    let t = Tmp::new();
    let e = denv(&t);
    let p = foo();
    for rel in [
        r"d\Foo Editor", r"d\Games\Foo Editor", r"d\Games\Foo Editor Saves", r"ou\AppData\Roaming\Foo Editor", r"ou\AppData\Local\FooEditor", r"ou\Documents\Foo Editor",
        r"u\Documents\Foo Editor", r"u\Saved Games\Foo Editor", r"u\AppData\LocalLow\Foo Editor", r"u\AppData\Local\Temp\Foo Editor", r"pf\Common Files\Acme\Foo Editor",
        r"pf\Common Files\Foo Editor", r"win\Temp\Foo Editor", r"pd\Acme\Foo Editor",
    ] {
        let d = t.dir(rel);
        assert_eq!(guard_deep(&d, &e, &p), Ok(()), "{rel}");
    }
    let f = t.file(r"u\AppData\Local\Temp\FooEditor-Setup-1.0.exe", 3);
    assert_eq!(guard_deep(&f, &e, &p), Ok(()));
    let dl = t.file(r"u\Downloads\Foo Editor Setup.exe", 3);
    assert_eq!(guard_deep(&dl, &e, &p), Ok(()));
    // the emptied parents of the install folder
    let mut q = foo();
    let inst = t.0.join(r"d\Big\Acme\Foo Editor");
    q.location = inst.to_string_lossy().into_owned();
    let parent = t.dir(r"d\Big\Acme");
    assert_eq!(guard_deep(&parent, &e, &q), Ok(()), "empty parent");
    let grand = t.0.join(r"d\Big");
    assert_eq!(guard_deep(&grand, &e, &q), Err("not inside an allowed folder"), "not empty: it holds Acme");
    std::fs::remove_dir(&parent).unwrap();
    assert_eq!(guard_deep(&grand, &e, &q), Ok(()), "empty grandparent");
}

#[test]
fn deep_guard_refuses_everything_else() {
    let t = Tmp::new();
    let e = denv(&t);
    let p = foo();
    let mut bad: Vec<PathBuf> = vec![];
    for rel in [
        r"d", r"d\Windows", r"d\Windows\Foo Editor", r"d\Program Files\Foo Editor", r"d\ProgramData\Foo Editor", r"d\Users\Foo Editor", r"d\$Recycle.Bin\Foo Editor",
        r"d\System Volume Information\Foo Editor", r"d\Games", r"d\Games\Other", r"d\Games\Foo Editor\saves\slot1", r"d\Other\Foo Editor\x", r"d\Games\Foo Editor Saves\x",
        r"ou", r"ou\AppData", r"ou\AppData\Roaming", r"ou\AppData\Roaming\Microsoft", r"ou\AppData\Local\Temp", r"ou\Documents", r"ou\Documents\Other", r"ou\Downloads", r"ou\Downloads\Foo Editor",
        r"u", r"u\Documents", r"u\Documents\Other", r"u\Documents\Foo Editor\a\b", r"u\Downloads", r"u\Downloads\Foo Editor", r"u\Downloads\notes.txt", r"u\Downloads\FooEditor.exe",
        r"u\Downloads\other-setup.exe", r"u\Saved Games", r"u\Saved Games\Other", r"u\AppData\Local\Temp", r"u\AppData\Local\Temp\Other", r"u\AppData\Local\Temp\Foo Editor\a\b",
        r"pf", r"pf\Common Files", r"pf\Common Files\Other", r"pf\Common Files\Acme\Other", r"pf\Common Files\Foo Editor\x\y", r"pf86",
        r"win", r"win\Temp", r"win\Temp\Other", r"win\System32", r"win\System32\Foo Editor", r"win\WinSxS", r"win\WinSxS\Foo Editor", r"win\System32\Tasks\Foo Editor", r"pd", r"pd\Microsoft",
    ] {
        bad.push(t.dir(rel));
    }
    bad.push(t.file(r"win\Foo Editor.dll", 1));
    for b in &bad {
        assert!(guard_deep(b, &e, &p).is_err(), "{} must be refused", b.display());
    }
    // lexical nastiness
    for s in [r"d\Games\Foo Editor\..\Other", r"d\Games\Foo Editor\.."] {
        let path = t.0.join(s);
        assert!(guard_deep(&path, &e, &p).is_err(), "{s}");
    }
    assert!(guard_deep(Path::new(""), &e, &p).is_err());
    assert!(guard_deep(Path::new(r"\\server\share\Foo Editor"), &e, &p).is_err());
    // not existing
    assert_eq!(guard_deep(&t.0.join(r"d\Nowhere\Foo Editor"), &e, &p), Err("does not exist"));
    // 8.3 / alternative names are not the canonical form
    let long = t.dir(r"d\Games\Foo Editor Long Name");
    let short = std::process::Command::new("cmd").args(["/c", "for", "%I", "in"]).arg(format!("(\"{}\")", long.display())).args(["do", "@echo", "%~sI"]).creation_flags(NO_WINDOW).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    if let Some(s) = short.filter(|s| s.contains('~')) {
        assert!(guard_deep(Path::new(&s), &e, &p).is_err(), "8.3 form {s}");
    }
}

#[test]
fn deep_guard_refuses_links_in_every_new_place() {
    let t = Tmp::new();
    let e = denv(&t);
    let p = foo();
    let target = t.dir("outside\\Precious");
    t.file("outside\\Precious\\keep.txt", 3);
    let mut links = Vec::new();
    for rel in [r"d\Games\Foo Editor", r"d\Foo Editor", r"ou\AppData\Roaming\Foo Editor", r"u\Documents\Foo Editor", r"pf\Common Files\Acme\Foo Editor", r"win\Temp\Foo Editor"] {
        let l = t.0.join(rel);
        std::fs::create_dir_all(l.parent().unwrap()).unwrap();
        let st = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&l).arg(&target).creation_flags(NO_WINDOW).output().unwrap();
        assert!(st.status.success());
        links.push(l);
    }
    for l in &links {
        assert!(guard_deep(l, &e, &p).is_err(), "{}", l.display());
        assert!(guard_deep(&l.join("keep.txt"), &e, &p).is_err(), "through {}", l.display());
    }
    // the scan never lists them
    let found = scan_fs(&p, &e, false, &|_| None, &mut Budget::new(10_000, 10));
    assert!(found.iter().all(|c| !matches!(&c.what, What::Fs(x) if links.contains(x))), "{found:?}");
    for l in &links {
        let _ = std::process::Command::new("cmd").args(["/c", "rmdir"]).arg(l).creation_flags(NO_WINDOW).output();
    }
    assert!(target.join("keep.txt").exists());
}

// ---------------------------------------------------------------- file scan: tiers, admin, look-alikes

#[test]
fn deep_file_scan_tiers_and_admin_flags() {
    let t = Tmp::new();
    let e = denv(&t);
    let mut p = foo();
    let install = t.dir(r"pf\Foo Editor");
    t.file(r"pf\Foo Editor\foo.exe", 50);
    p.location = install.to_string_lossy().into_owned();
    // certain
    t.dir(r"u\AppData\Roaming\Foo Editor");
    t.dir(r"ou\AppData\Local\FooEditor");
    t.dir(r"d\Games\Foo Editor");
    t.dir(r"d\Foo-Editor");
    t.dir(r"pd\Acme\Foo Editor"); // the publisher folder holds only this app: it is Certain and covers the child
    t.dir(r"u\AppData\LocalLow\Foo Editor");
    // probable
    t.dir(r"u\Documents\Foo Editor Notes");
    t.dir(r"u\Saved Games\Foo Editor"); // exactly the app's name, but a place for the user's own files: Probable only
    t.dir(r"u\AppData\Local\Temp\Foo Editor Cache");
    t.file(r"u\Downloads\FooEditor-Setup-2.0.exe", 5);
    t.dir(r"u\AppData\Roaming\Acme\Foo Editor");
    t.dir(r"u\AppData\Roaming\Acme\Bar");
    t.dir(r"d\Microsoft Foo Editor");
    t.dir(r"u\AppData\Local\FooEditorPro"); // the app's name as words inside a longer name
    // look-alikes and protected places: nothing
    for r in [r"u\AppData\Local\FooBar", r"u\AppData\Local\Foo", r"u\AppData\Local\Foo Edit", r"d\Games\Other", r"d\Windows\Foo Editor", r"d\Program Files\Foo Editor", r"d\Users\Foo Editor",
        r"u\Documents\Foo", r"u\Downloads\FooEditor-Setup-beta-leak.exe", r"win\System32\Foo Editor", r"u\AppData\Local\Temp\Foo"] {
        if r.contains('.') {
            t.file(r, 1);
        } else {
            t.dir(r);
        }
    }
    t.file(r"u\Desktop\Foo Editor.lnk", 1);
    t.file(r"u\Desktop\Foo Editor Help.lnk", 1);
    t.file(r"u\Desktop\Pinned thing.lnk", 1);
    t.file(r"u\Desktop\Other.lnk", 1);
    t.file(r"u\AppData\Roaming\Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar\Foo Editor.lnk", 1);
    t.file(r"u\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Acme\Launch.lnk", 1);
    t.dir(r"u\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Foo Editor");
    let ins = install.clone();
    let lnk = move |l: &Path| (l.file_name().unwrap() == "Pinned thing.lnk" || l.file_name().unwrap() == "Launch.lnk").then(|| ins.join("foo.exe"));
    let found = scan_fs(&p, &e, false, &lnk, &mut Budget::new(100_000, 20));
    let rel = |c: &Cand| match &c.what {
        What::Fs(x) => x.strip_prefix(&t.0).unwrap().to_string_lossy().to_lowercase(),
        _ => String::new(),
    };
    let mut got: Vec<(String, Tier, bool)> = found.iter().map(|c| (rel(c), c.tier, c.admin)).collect();
    got.sort_by(|a, b| a.0.cmp(&b.0));
    let (c, pr) = (Tier::Certain, Tier::Probable);
    let mut want = vec![
        (r"pf\foo editor", c, true),
        (r"u\appdata\roaming\foo editor", c, false),
        (r"ou\appdata\local\fooeditor", c, true),
        (r"d\games\foo editor", c, false),
        (r"d\foo-editor", c, false),
        (r"pd\acme", c, true),
        (r"u\appdata\localLow\foo editor".to_lowercase().as_str().to_string().leak(), c, false),
        (r"u\documents\foo editor notes", pr, false),
        (r"u\saved games\foo editor", pr, false),
        (r"u\appdata\local\temp\foo editor cache", pr, false),
        (r"u\downloads\fooeditor-setup-2.0.exe", pr, false),
        (r"u\appdata\roaming\acme", pr, false),
        (r"u\appdata\local\fooeditorpro", pr, false),
        (r"u\appdata\roaming\acme\foo editor", c, false),
        (r"d\microsoft foo editor", pr, false),
        (r"u\desktop\foo editor.lnk", c, false),
        (r"u\desktop\foo editor help.lnk", pr, false),
        (r"u\desktop\pinned thing.lnk", c, false),
        (r"u\appdata\roaming\microsoft\internet explorer\quick launch\user pinned\taskbar\foo editor.lnk", c, false),
        (r"u\appdata\roaming\microsoft\windows\start menu\programs\acme\launch.lnk", c, false),
        (r"u\appdata\roaming\microsoft\windows\start menu\programs\foo editor", c, false),
    ]
    .into_iter()
    .map(|(a, b, d)| (a.to_string(), b, d))
    .collect::<Vec<_>>();
    want.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(got, want);
    // every Certain one is pre-ticked, every Probable one is not
    assert!(found.iter().all(|c| c.ticked == (c.tier == Tier::Certain)));
    // each one passes the guard it will be deleted through
    for cd in &found {
        assert!(guard_cand(&cd.what, &p, &e, &FakeReg::default()).is_ok(), "{}", cd.shown);
    }
    // a shared install folder is only Probable
    let shared = scan_fs(&p, &e, true, &|_| None, &mut Budget::new(100_000, 20));
    assert!(shared.iter().any(|c| matches!(&c.what, What::Fs(x) if *x == install) && c.tier == Tier::Probable && !c.ticked));
}

#[test]
fn emptied_parents_of_the_install_folder_are_probable() {
    let t = Tmp::new();
    let e = denv(&t);
    let mut p = foo();
    p.location = t.0.join(r"d\Big\Acme\Foo Editor").to_string_lossy().into_owned();
    t.dir(r"d\Big\Acme");
    let found = scan_fs(&p, &e, false, &|_| None, &mut Budget::new(10_000, 10));
    let shown: Vec<String> = found.iter().map(|c| c.shown.to_lowercase()).collect();
    assert!(shown.len() == 1 && shown[0].ends_with(r"d\big\acme"), "{shown:?}"); // Big still holds Acme, so only Acme is empty
    assert!(found.iter().all(|c| c.tier == Tier::Probable && !c.ticked));
    // not empty -> not listed
    t.file(r"d\Big\Acme\other.txt", 1);
    let found = scan_fs(&p, &e, false, &|_| None, &mut Budget::new(10_000, 10));
    assert!(found.is_empty(), "{found:?}");
}

// ---------------------------------------------------------------- registry evidence

fn reg_fixture() -> FakeReg {
    let mut r = FakeReg::default();
    let (l, u) = (Hive::Hklm, Hive::Hkcu);
    // Software keys
    r.key(u, r"Software\Foo Editor", &[("x", sv("1"))]);
    r.key(u, r"Software\Acme\Foo Editor", &[]);
    r.key(l, r"SOFTWARE\WOW6432Node\Acme\Foo Editor\Sub", &[("a", sv("b"))]);
    r.key(l, r"SOFTWARE\Foo Editor Settings", &[]);
    r.key(l, r"SOFTWARE\Acme", &[]); // publisher key: holds only this app below
    r.key(u, r"Software\Beta\Foo Editor", &[]); // publisher does not match
    r.key(u, r"Software\FooEditorPro", &[]);
    // a publisher key shared with another app
    r.key(u, r"Software\Acme", &[]);
    r.key(u, r"Software\Acme\Bar", &[]);
    // COM / classes
    r.key(l, &format!(r"SOFTWARE\Classes\CLSID\{G}"), &[("", sv("Foo"))]);
    r.key(l, &format!(r"SOFTWARE\Classes\CLSID\{OTHER_G}"), &[("", sv("Other"))]);
    r.key(l, &format!(r"SOFTWARE\Classes\CLSID\{OTHER_G}\InprocServer32"), &[("", sv(r"C:\Windows\System32\other.dll"))]);
    let srv = "{22222222-3333-4444-5555-666666666666}";
    r.key(l, &format!(r"SOFTWARE\Classes\CLSID\{srv}"), &[]);
    r.key(l, &format!(r"SOFTWARE\Classes\CLSID\{srv}\InprocServer32"), &[("", sv(r"D:\Apps\Foo Editor\shell.dll"))]);
    r.key(l, &format!(r"SOFTWARE\Classes\AppID\{G}"), &[]);
    r.key(l, &format!(r"SOFTWARE\Classes\Interface\{OTHER_G}"), &[]);
    r.key(l, r"SOFTWARE\Classes\FooEditor.Document\shell\open\command", &[("", sv(r#""D:\Apps\Foo Editor\foo.exe" "%1""#))]);
    r.key(l, r"SOFTWARE\Classes\Other.Document\shell\open\command", &[("", sv(r#""C:\Other\o.exe" "%1""#))]);
    r.key(l, r"SOFTWARE\Classes\.foo", &[("", sv("FooEditor.Document"))]);
    r.key(l, r"SOFTWARE\Classes\.foo\OpenWithProgids", &[("fooeditor.document", Val::S(String::new())), ("other.document", Val::S(String::new()))]);
    r.key(l, r"SOFTWARE\Classes\*\shellex\ContextMenuHandlers\FooShell", &[("", sv(srv))]);
    r.key(l, r"SOFTWARE\Classes\*\shellex\ContextMenuHandlers\Other", &[("", sv(OTHER_G))]);
    r.key(l, r"SOFTWARE\Classes\Directory\shell\FooHere\command", &[("", sv(r#"D:\Apps\Foo Editor\foo.exe "%V""#))]);
    r.key(l, r"SOFTWARE\Classes\Directory\shell\OtherHere\command", &[("", sv(r"C:\x\y.exe"))]);
    // uninstall remnants and App Paths
    let un = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
    r.key(l, &format!(r"{un}\{G}"), &[("displayname", sv("Foo Editor")), ("uninstallstring", sv(r#""D:\Apps\Foo Editor\unins000.exe""#))]);
    r.key(l, &format!(r"{un}\Other"), &[("displayname", sv("Other")), ("uninstallstring", sv(r"C:\gone\u.exe"))]);
    r.key(l, &format!(r"{un}\FooByName"), &[("displayname", sv("Foo Editor")), ("uninstallstring", sv(r"C:\gone\u.exe"))]);
    r.key(l, r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\foo.exe", &[("", sv(r"D:\Apps\Foo Editor\foo.exe"))]);
    r.key(l, r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\other.exe", &[("", sv(r"C:\x\other.exe"))]);
    // values
    let run = r"Software\Microsoft\Windows\CurrentVersion\Run";
    r.key(u, run, &[("FooEditor", sv(r#""D:\Apps\Foo Editor\foo.exe" --tray"#)), ("Other", sv(r"C:\Windows\explorer.exe")), ("Sib", sv(r"D:\Apps\Foo Editor Pro\x.exe"))]);
    r.key(u, r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\MuiCache", &[(r"D:\Apps\Foo Editor\foo.exe.FriendlyAppName", sv("Foo")), (r"C:\other\o.exe.FriendlyAppName", sv("O"))]);
    r.key(u, r"Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers", &[(r"D:\Apps\Foo Editor\foo.exe", sv("~ RUNASADMIN"))]);
    let pk = pack_guid(G).unwrap();
    r.key(l, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Installer\UserData\S-1-5-18\Products", &[]);
    r.key(l, &format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Installer\UserData\S-1-5-18\Products\{pk}"), &[("displayname", sv("Foo Editor"))]);
    r.key(l, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Installer\UserData\S-1-5-18\Components\C0FFEE", &[(&pk, sv(r"D:\Apps\Foo Editor\foo.exe")), ("ZZZZ", sv("other"))]);
    r.key(l, &format!(r"SOFTWARE\Classes\Installer\Products\{pk}"), &[]);
    // firewall
    r.key(
        l,
        r"SYSTEM\CurrentControlSet\Services\SharedAccess\Parameters\FirewallPolicy\FirewallRules",
        &[("{rule-1}", sv(r"v2.30|Action=Allow|Active=TRUE|Dir=In|App=D:\Apps\Foo Editor\foo.exe|Name=Foo Editor|")), ("{rule-2}", sv(r"v2.30|Action=Allow|Active=TRUE|Dir=In|App=C:\Windows\System32\svchost.exe|Name=Other|"))],
    );
    // services
    let sv_ = r"SYSTEM\CurrentControlSet\Services";
    r.key(l, &format!(r"{sv_}\FooUpdater"), &[("type", Val::D(0x10)), ("imagepath", Val::E(r#""D:\Apps\Foo Editor\updater.exe" /svc"#.into()))]);
    r.key(l, &format!(r"{sv_}\FooDriver"), &[("type", Val::D(1)), ("imagepath", Val::E(r"\??\D:\Apps\Foo Editor\foo.sys".into()))]);
    r.key(l, &format!(r"{sv_}\OtherSvc"), &[("type", Val::D(0x10)), ("imagepath", Val::E(r"C:\Windows\System32\svchost.exe -k x".into()))]);
    r
}

#[test]
fn registry_keys_need_evidence() {
    let t = Tmp::new();
    let e = t.env();
    let p = foo();
    let r = reg_fixture();
    let (l, u) = (Hive::Hklm, Hive::Hkcu);
    let key = |h: Hive, s: &str| ok(&What::Key(h, s.to_string()), &p, &e, &r);
    // software keys
    assert_eq!(key(u, r"Software\Foo Editor").map(|x| x.0), Some(Tier::Certain));
    assert_eq!(key(u, r"Software\Acme\Foo Editor").map(|x| x.1), Some("key under the publisher"));
    assert_eq!(key(l, r"SOFTWARE\WOW6432Node\Acme\Foo Editor").map(|x| x.0), Some(Tier::Certain));
    assert_eq!(key(l, r"SOFTWARE\Foo Editor Settings").map(|x| x.0), Some(Tier::Probable));
    assert_eq!(key(l, r"SOFTWARE\Acme").map(|x| x.1), Some("publisher key holding only this app"));
    assert_eq!(key(u, r"Software\FooEditorPro").map(|x| x.0), Some(Tier::Probable));
    for bad in [r"Software\Acme", r"Software\Beta\Foo Editor", r"Software\Microsoft\Foo Editor", r"Software\Foo Editor\Sub\Deeper", r"Software", r"Software\Classes", r"Software\Microsoft",
        r"Software\Wow6432Node", r"SYSTEM\Foo Editor", r"Software\..\Foo Editor", r"Software\\Foo Editor", "", r"Foo Editor"] {
        assert!(key(u, bad).is_none(), "{bad}");
    }
    // Software\Acme in HKCU is shared with another app: refused (HKLM's holds only this app: accepted above)
    // roots and keys with other people's subkeys are never offered
    for bad in [r"SOFTWARE\Classes", r"SOFTWARE\Classes\CLSID", r"SOFTWARE\Classes\Installer", r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run", r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths", r"SYSTEM\CurrentControlSet\Services", r"SYSTEM\CurrentControlSet\Services\FooUpdater", r"SOFTWARE\Microsoft", r"SOFTWARE\Microsoft\Windows"] {
        assert!(key(l, bad).is_none(), "{bad}");
    }
    // classes: GUID evidence, COM server evidence, ProgIDs, shell handlers
    assert_eq!(key(l, &format!(r"SOFTWARE\Classes\CLSID\{G}")).map(|x| x.1), Some("carries the product GUID"));
    assert_eq!(key(l, &format!(r"SOFTWARE\Classes\AppID\{G}")).map(|x| x.0), Some(Tier::Certain));
    assert_eq!(key(l, r"SOFTWARE\Classes\CLSID\{22222222-3333-4444-5555-666666666666}").map(|x| x.1), Some("COM class or handler in the install folder"));
    assert!(key(l, &format!(r"SOFTWARE\Classes\CLSID\{OTHER_G}")).is_none());
    assert!(key(l, &format!(r"SOFTWARE\Classes\Interface\{OTHER_G}")).is_none());
    assert!(key(l, "SOFTWARE\\Classes\\CLSID\\notaguid").is_none());
    assert!(key(l, r"SOFTWARE\Classes\FooEditor.Document").is_some());
    assert!(key(l, r"SOFTWARE\Classes\Other.Document").is_none());
    assert!(key(l, r"SOFTWARE\Classes\.foo").is_none(), "extension keys are never deleted");
    assert!(key(l, r"SOFTWARE\Classes\*").is_none() && key(l, r"SOFTWARE\Classes\Directory").is_none());
    assert!(key(l, r"SOFTWARE\Classes\*\shellex\ContextMenuHandlers\FooShell").is_some());
    assert!(key(l, r"SOFTWARE\Classes\*\shellex\ContextMenuHandlers\Other").is_none());
    assert!(key(l, r"SOFTWARE\Classes\*\shellex\ContextMenuHandlers").is_none());
    assert!(key(l, r"SOFTWARE\Classes\Directory\shell\FooHere").is_some());
    assert!(key(l, r"SOFTWARE\Classes\Directory\shell\OtherHere").is_none());
    assert!(key(l, r"SOFTWARE\Classes\Directory\shell").is_none());
    // Windows Installer records need the packed ProductCode
    let pk = pack_guid(G).unwrap();
    let ud = format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Installer\UserData\S-1-5-18\Products\{pk}");
    assert_eq!(key(l, &ud).map(|x| x.1), Some("Windows Installer record of this product"));
    assert!(key(l, &ud.replace("S-1-5-18", "Everyone")).is_none());
    assert!(key(l, &ud.replace(&pk, "0123456789ABCDEF0123456789ABCDEF")).is_none());
    assert!(key(u, &ud).is_none(), "only HKLM holds these");
    assert!(key(l, &format!(r"SOFTWARE\Classes\Installer\Products\{pk}")).is_some());
    // dead uninstall entries
    let un = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
    assert_eq!(key(l, &format!(r"{un}\{G}")).map(|x| x.1), Some("uninstall entry of this product, its uninstaller is gone"));
    assert!(key(l, &format!(r"{un}\FooByName")).is_some());
    assert!(key(l, &format!(r"{un}\Other")).is_none());
    // App Paths
    assert!(key(l, r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\foo.exe").is_some());
    assert!(key(l, r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\other.exe").is_none());
}

#[test]
fn a_live_uninstall_entry_is_never_a_leftover() {
    let t = Tmp::new();
    let e = t.env();
    let alive = t.file(r"i\unins000.exe", 3);
    let mut p = foo();
    p.location = t.0.join("i").to_string_lossy().into_owned();
    let mut r = FakeReg::default();
    let un = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
    r.key(Hive::Hklm, &format!(r"{un}\{G}"), &[("displayname", sv("Foo Editor")), ("uninstallstring", sv(&format!("\"{}\"", alive.display())))]);
    assert!(ok(&What::Key(Hive::Hklm, format!(r"{un}\{G}")), &p, &e, &r).is_none(), "its uninstaller still exists: the app is installed");
    std::fs::remove_file(&alive).unwrap();
    assert!(ok(&What::Key(Hive::Hklm, format!(r"{un}\{G}")), &p, &e, &r).is_some());
}

#[test]
fn registry_values_need_evidence() {
    let t = Tmp::new();
    let e = t.env();
    let p = foo();
    let r = reg_fixture();
    let (l, u) = (Hive::Hklm, Hive::Hkcu);
    let val = |h: Hive, s: &str, n: &str| ok(&What::Value(h, s.to_string(), n.to_string()), &p, &e, &r);
    let run = r"Software\Microsoft\Windows\CurrentVersion\Run";
    assert_eq!(val(u, run, "FooEditor").map(|x| x.1), Some("starts the app at sign-in"));
    assert!(val(u, run, "Other").is_none());
    assert!(val(u, run, "Sib").is_none(), "a sibling folder with the same prefix is not inside");
    assert!(val(u, run, "Missing").is_none());
    assert!(val(u, run, "").is_none());
    assert!(val(u, r"Software\Microsoft\Windows\CurrentVersion\Policies\Explorer\Run", "FooEditor").is_none());
    assert_eq!(val(u, r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\MuiCache", r"D:\Apps\Foo Editor\foo.exe.FriendlyAppName").map(|x| x.1), Some("install folder path in a history list"));
    assert!(val(u, r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\MuiCache", r"C:\other\o.exe.FriendlyAppName").is_none());
    assert!(val(u, r"Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers", r"D:\Apps\Foo Editor\foo.exe").is_some());
    // handler associations need the ProgID to be this app's
    let ow = r"SOFTWARE\Classes\.foo\OpenWithProgids";
    assert_eq!(val(l, ow, "fooeditor.document").map(|x| x.1), Some("association with a removed handler"));
    assert!(val(l, ow, "other.document").is_none());
    // Windows Installer component records are matched by the packed ProductCode in the value NAME
    let pk = pack_guid(G).unwrap();
    let comp = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Installer\UserData\S-1-5-18\Components\C0FFEE";
    assert!(val(l, comp, &pk.to_lowercase()).is_some());
    assert!(val(l, comp, "zzzz").is_none());
    // firewall rules: the App= path decides
    let fw = r"SYSTEM\CurrentControlSet\Services\SharedAccess\Parameters\FirewallPolicy\FirewallRules";
    assert_eq!(val(l, fw, "{rule-1}").map(|x| x.1), Some("firewall rule for a program in the install folder"));
    assert!(val(l, fw, "{rule-2}").is_none());
    assert!(val(u, fw, "{rule-1}").is_none());
    // not a list we know
    assert!(val(l, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System", "x").is_none());
}

#[test]
fn services_tasks_and_drivers() {
    let t = Tmp::new();
    let e = t.env();
    let p = foo();
    let r = reg_fixture();
    assert_eq!(ok(&What::Service("FooUpdater".into()), &p, &e, &r).map(|x| x.1), Some("service runs a file in the install folder"));
    assert!(ok(&What::Service("FooDriver".into()), &p, &e, &r).is_none(), "drivers are only listed");
    assert!(ok(&What::Service("OtherSvc".into()), &p, &e, &r).is_none());
    assert!(ok(&What::Service("Nope".into()), &p, &e, &r).is_none());
    assert!(ok(&What::Service(r"..\x".into()), &p, &e, &r).is_none());
    assert!(ok(&What::Service("a\"b".into()), &p, &e, &r).is_none());
    assert_eq!(list_drivers(&p, &e, &r), vec!["Driver service: FooDriver".to_string()]);
    // tasks: UTF-16 and UTF-8 XML as the Task Scheduler writes it
    let xml = |cmd: &str| format!("<?xml version=\"1.0\" encoding=\"UTF-16\"?>\n<Task><Actions Context=\"Author\"><Exec><Command>{cmd}</Command><Arguments>--check</Arguments></Exec></Actions></Task>");
    let u16bytes = |s: &str| -> Vec<u8> { std::iter::once(0xFEFFu16).chain(s.encode_utf16()).flat_map(u16::to_le_bytes).collect() };
    let tasks = e.windows.join(r"System32\Tasks");
    std::fs::create_dir_all(tasks.join("Acme")).unwrap();
    std::fs::write(tasks.join(r"Acme\FooUpdate"), u16bytes(&xml(r#""D:\Apps\Foo Editor\updater.exe""#))).unwrap();
    std::fs::write(tasks.join(r"Acme\Other"), xml(r"C:\Windows\System32\cmd.exe")).unwrap();
    std::fs::write(tasks.join("Foo Editor"), xml(r"C:\somewhere\else.exe")).unwrap();
    std::fs::write(tasks.join("FooUtf8"), xml("D:\\Apps\\Foo Editor\\x.exe")).unwrap();
    assert_eq!(task_commands(&task_text(&u16bytes(&xml("a&amp;b")))), vec!["a&b"]);
    assert_eq!(ok(&What::Task(r"\Acme\FooUpdate".into()), &p, &e, &r).map(|x| x.1), Some("scheduled task starts a file in the install folder"));
    assert!(ok(&What::Task(r"\FooUtf8".into()), &p, &e, &r).is_some());
    assert!(ok(&What::Task(r"\Acme\Other".into()), &p, &e, &r).is_none());
    assert_eq!(ok(&What::Task(r"\Foo Editor".into()), &p, &e, &r), Some((Tier::Probable, "task named like the app")));
    for bad in [r"\..\x", r"Acme\FooUpdate", r"\Acme\\FooUpdate", r"\Missing", "", r"\a:b"] {
        assert!(ok(&What::Task(bad.into()), &p, &e, &r).is_none(), "{bad}");
    }
    let mut found: Vec<String> = scan_tasks(&p, &e, &mut Budget::new(1000, 5)).iter().map(|c| c.shown.clone()).collect();
    found.sort();
    assert_eq!(found, vec![r"Scheduled task: \Acme\FooUpdate", r"Scheduled task: \Foo Editor", r"Scheduled task: \FooUtf8"]);
    assert!(ok(&What::Remote("x".into()), &p, &e, &r).is_none());
}

#[test]
fn full_registry_scan_from_fixtures() {
    let t = Tmp::new();
    let e = t.env();
    let p = foo();
    let r = reg_fixture();
    let stages = RefCell::new(Vec::<String>::new());
    let found = scan_registry(&p, &e, &r, &mut Budget::new(100_000, 20), &|s, _| stages.borrow_mut().push(s.to_string()));
    let mut keys: Vec<String> = found.iter().map(|c| cand_key(&c.what)).collect();
    keys.sort();
    for want in [
        r"key:HKCU\software\foo editor",
        r"key:HKCU\software\acme\foo editor",
        r"key:HKLM\software\wow6432node\acme\foo editor",
        r"key:HKLM\software\foo editor settings",
        r"key:HKLM\software\classes\fooeditor.document",
        r"key:HKLM\software\classes\*\shellex\contextmenuhandlers\fooshell",
        r"key:HKLM\software\classes\directory\shell\foohere",
        r"key:HKLM\software\microsoft\windows\currentversion\app paths\foo.exe",
        r"val:HKCU\software\microsoft\windows\currentversion\run|fooeditor",
        r"val:HKLM\software\classes\.foo\openwithprogids|fooeditor.document",
        r"val:HKLM\system\currentcontrolset\services\sharedaccess\parameters\firewallpolicy\firewallrules|{rule-1}",
        "svc:fooupdater",
    ] {
        assert!(keys.contains(&want.to_string()), "missing {want}\n{keys:#?}");
    }
    for unwanted in ["other", "beta", "otherhere", "foodriver", "sib", "rule-2", "othersvc"] {
        assert!(!keys.iter().any(|k| k.contains(unwanted)), "{unwanted} must not be found: {keys:#?}");
    }
    // the Software\Acme publisher key in HKCU is shared with "Bar": not offered; in HKLM it holds only this app
    assert!(!keys.contains(&r"key:HKCU\software\acme".to_string()));
    assert!(keys.contains(&r"key:HKLM\software\acme".to_string()));
    // HKLM / services / firewall are administrator items, HKCU ones are not
    assert!(found.iter().filter(|c| c.shown.starts_with("HKCU")).all(|c| !c.admin));
    assert!(found.iter().filter(|c| c.shown.starts_with("HKLM") || c.kind == "service").all(|c| c.admin));
    assert!(found.iter().all(|c| c.ticked == (c.tier == Tier::Certain)));
    assert!(stages.borrow().iter().any(|s| s == "services") && stages.borrow().iter().any(|s| s.starts_with("registry")));
    // the cap is reported
    let mut b = Budget::new(3, 20);
    let _ = scan_registry(&p, &e, &r, &mut b, &|_, _| {});
    assert!(b.cut);
}

#[test]
fn cand_keys_are_stable_and_case_blind() {
    assert_eq!(cand_key(&What::Key(Hive::Hklm, r"SOFTWARE\Acme".into())), cand_key(&What::Key(Hive::Hklm, r"software\ACME".into())));
    assert_ne!(cand_key(&What::Key(Hive::Hklm, "a".into())), cand_key(&What::Key(Hive::Hkcu, "a".into())));
    assert_ne!(cand_key(&What::Key(Hive::Hklm, "a\\b".into())), cand_key(&What::Value(Hive::Hklm, "a".into(), "b".into())));
    let c = Cand::new(What::Service("S".into()), Tier::Probable, "x", 0);
    assert!(c.admin && !c.ticked && c.kind == "service");
}

// ---------------------------------------------------------------- the elevated request

fn req(name: &str, location: &str) -> Req {
    Req { mode: Mode::Delete, name: name.into(), publisher: "Acme Inc".into(), location: location.into(), sub: String::new(), hklm: true, guids: vec![G.into()], keys: vec!["key:HKLM\\software\\acme".into()] }
}

#[test]
fn elevated_request_validation_rejects_forged_identities() {
    let t = Tmp::new();
    let e = t.env();
    let r = FakeReg::default();
    let good = t.0.join(r"pf\Foo Editor").to_string_lossy().into_owned();
    // a believable identity passes
    let p = validate_req(&req("Foo Editor", &good), &e, &r).unwrap();
    assert_eq!(p.name, "Foo Editor");
    assert!(p.guids().contains(&G.to_string()));
    assert!(validate_req(&req("Foo Editor", ""), &e, &r).is_ok(), "no recorded folder: name evidence only");
    // generic / short / system-ish names
    for n in ["", "ab", "Foo", "Tools", "App", "Microsoft", "Windows", "System", "Data", "Update", "  ", "Pro", "Setup"] {
        assert!(validate_req(&req(n, &good), &e, &r).is_err(), "name {n:?}");
    }
    // install folders that are system roots, contain one, or are not app folders
    let bad_dirs = [
        e.windows.clone(), e.windows.join("System32"), e.windows.join("Foo Editor"), e.pf.clone(), e.pf86.clone(), e.programdata.clone(), e.profile.clone(), e.profile.join("Documents"),
        e.profile.join(r"Documents\Foo Editor"), e.profile.join("Desktop"), e.appdata.clone(), e.local.clone(), t.0.clone(), t.0.join("u").join(".."), e.pf.join("Common Files"),
        e.pf.join("WindowsApps"), e.pf.join("Windows Defender"), e.local.join("Pill"), PathBuf::from(r"C:\"), PathBuf::from(r"C:\Users"), PathBuf::from(r"\\server\share\x"), PathBuf::from("relative\\dir"),
    ];
    for d in &bad_dirs {
        assert!(validate_req(&req("Foo Editor", &d.to_string_lossy()), &e, &r).is_err(), "install {}", d.display());
    }
    // malformed values
    let mut q = req("Foo Editor", &good);
    q.guids = vec!["{not-a-guid}".into()];
    assert!(validate_req(&q, &e, &r).is_err());
    q = req("Foo\u{0}Editor", &good);
    assert!(validate_req(&q, &e, &r).is_err());
    q = req("Foo Editor", &good);
    q.keys = vec!["x".repeat(701)];
    assert!(validate_req(&q, &e, &r).is_err());
    q.keys = (0..5001).map(|i| format!("k{i}")).collect();
    assert!(validate_req(&q, &e, &r).is_err());
    q = req("Foo Editor", &good);
    q.guids = (0..9).map(|_| G.to_string()).collect();
    assert!(validate_req(&q, &e, &r).is_err());
}

#[test]
fn elevated_request_refuses_apps_that_are_still_installed() {
    let t = Tmp::new();
    let e = t.env();
    let inst = t.dir(r"pf\Foo Editor");
    let un = t.file(r"pf\Foo Editor\unins000.exe", 3);
    let loc = inst.to_string_lossy().into_owned();
    let mut r = FakeReg::default();
    let k = format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{G}");
    r.key(Hive::Hklm, &k, &[("displayname", sv("Foo Editor")), ("installlocation", sv(&loc)), ("uninstallstring", sv(&format!("\"{}\"", un.display())))]);
    assert_eq!(validate_req(&req("Foo Editor", &loc), &e, &r).err(), Some("the app is still installed"));
    // another program registered inside the folder
    let mut r2 = FakeReg::default();
    r2.key(Hive::Hklm, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Neighbour", &[("displayname", sv("Neighbour")), ("uninstallstring", sv(&format!("\"{}\"", un.display()))), ("installlocation", sv(&loc))]);
    assert!(validate_req(&req("Foo Editor", &loc), &e, &r2).is_err());
    // the same name registered with a living uninstaller elsewhere
    let other = t.file(r"pf\Other\u.exe", 3);
    let mut r3 = FakeReg::default();
    r3.key(Hive::Hkcu, r"Software\Microsoft\Windows\CurrentVersion\Uninstall\x", &[("displayname", sv("Foo Editor")), ("uninstallstring", sv(&other.to_string_lossy()))]);
    assert!(validate_req(&req("Foo Editor", &loc), &e, &r3).is_err());
    // a dead entry (uninstaller gone) is a leftover, not an installed app
    std::fs::remove_file(&un).unwrap();
    assert!(validate_req(&req("Foo Editor", &loc), &e, &r).is_ok());
}

#[test]
fn request_wire_format() {
    let q = req("Foo Editor", r"D:\Apps\Foo Editor");
    let back = Req::from_json(&q.to_json()).unwrap();
    assert_eq!((back.name, back.mode, back.keys, back.guids), (q.name, Mode::Delete, q.keys, q.guids));
    assert!(Req::from_json(&json!({ "v": 2, "mode": "scan" })).is_err());
    assert!(Req::from_json(&json!({ "v": 1, "mode": "format c:" })).is_err());
    assert!(Req::from_json(&json!("x")).is_err());
    assert_eq!(valid_port_token("5000:0123456789abcdef0123456789abcdef"), Some((5000, "0123456789abcdef0123456789abcdef".into())));
    for bad in ["80:0123456789abcdef0123456789abcdef", "5000:XYZ", "5000", "5000:0123456789ABCDEF0123456789ABCDEF", "99999:0123456789abcdef0123456789abcdef", ":0123456789abcdef0123456789abcdef", "5000:0123456789abcdef0123456789abcdef0"] {
        assert!(valid_port_token(bad).is_none(), "{bad}");
    }
    for good in ["reg", "log", "task0.xml", "task12.xml"] {
        assert!(store_name_ok(good), "{good}");
    }
    for bad in ["", "..\\x", "reg.exe", "task.xml", "taskx.xml", "task1.xml.exe", "../reg", "C:\\x", "task1\\.xml"] {
        assert!(!store_name_ok(bad), "{bad}");
    }
}

// ---------------------------------------------------------------- deleting: ordering, backups, refusals

struct HkcuKey(String);
impl Drop for HkcuKey {
    fn drop(&mut self) {
        let _ = std::process::Command::new("reg").args(["delete", &format!(r"HKCU\{}", self.0), "/f"]).creation_flags(NO_WINDOW).output();
    }
}
fn hkcu_exists(sub: &str) -> bool {
    std::process::Command::new("reg").args(["query", &format!(r"HKCU\{sub}")]).creation_flags(NO_WINDOW).output().is_ok_and(|o| o.status.success())
}
fn reg_add(sub: &str, args: &[&str]) {
    let mut a = vec!["add".to_string(), format!(r"HKCU\{sub}")];
    a.extend(args.iter().map(|s| s.to_string()));
    a.push("/f".into());
    assert!(std::process::Command::new("reg").args(&a).creation_flags(NO_WINDOW).output().unwrap().status.success());
}
fn utf16(bytes: &[u8]) -> String {
    String::from_utf16_lossy(&bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>())
}

#[test]
fn the_backup_is_written_before_the_registry_key_is_deleted_and_restores_it() {
    let t = Tmp::new();
    let e = t.env();
    let name = format!("PillDeepTestApp{}", std::process::id());
    let sub = format!(r"Software\{name}");
    let _g = HkcuKey(sub.clone());
    reg_add(&format!(r"{sub}\Settings"), &["/v", "Marker", "/t", "REG_SZ", "/d", "hello-backup"]);
    reg_add(&sub, &["/v", "Num", "/t", "REG_DWORD", "/d", "42"]);
    reg_add(&sub, &["/v", "Blob", "/t", "REG_BINARY", "/d", "0102ff"]);
    reg_add(&sub, &["/v", "Multi", "/t", "REG_MULTI_SZ", "/d", "a\\0b"]);
    reg_add(&sub, &["/v", "Exp", "/t", "REG_EXPAND_SZ", "/d", "%TEMP%\\x"]);
    reg_add(&sub, &["/v", "Quote", "/t", "REG_SZ", "/d", "say \"hi\" \\ there"]);
    let p = prog(&name, "", "");
    let events = RefCell::new(Vec::<String>::new());
    let bk = t.dir("bk");
    let store = |n: &str, bytes: &[u8]| -> Result<String, String> {
        events.borrow_mut().push(format!("store {n}; key still there: {}", hkcu_exists(&sub)));
        file_store(&bk, "app-1", n, bytes)
    };
    let c = Cand::new(What::Key(Hive::Hkcu, sub.clone()), Tier::Certain, "x", 0);
    let r = delete_cands(&p, &[&c], &e, &RealReg, "1", false, &store, &|_, _, _, _, _| {});
    assert_eq!((r.done.len(), r.failed.len()), (1, 0), "{:?}", r.failed);
    assert!(!hkcu_exists(&sub), "key should be gone");
    let ev = events.borrow();
    assert!(ev[0].starts_with("store reg; key still there: true"), "{ev:?}"); // the backup was stored while the key still existed
    assert!(ev.last().unwrap().starts_with("store log"));
    let reg = r.backup.expect("backup path");
    let bytes = std::fs::read(&reg).unwrap();
    assert!(bytes[..2] == [0xFF, 0xFE]);
    let text = utf16(&bytes);
    assert!(text.starts_with("\u{feff}Windows Registry Editor Version 5.00") && text.contains(&name) && text.contains("hello-backup"), "{text}");
    // restore it with the real regedit importer: everything comes back
    let st = std::process::Command::new("reg").arg("import").arg(&reg).creation_flags(NO_WINDOW).output().unwrap();
    assert!(st.status.success(), "{}", String::from_utf8_lossy(&st.stderr));
    assert!(hkcu_exists(&sub));
    let q = std::process::Command::new("reg").args(["query", &format!(r"HKCU\{sub}")]).creation_flags(NO_WINDOW).output().unwrap();
    let out = String::from_utf8_lossy(&q.stdout).to_string();
    for want in ["Num", "0x2a", "Blob", "0102FF", "Multi", "Exp", "%TEMP%", "hi"] {
        assert!(out.contains(want), "{want} in {out}");
    }
    assert!(hkcu_exists(&format!(r"{sub}\Settings")));
    let q2 = std::process::Command::new("reg").args(["query", &format!(r"HKCU\{sub}\Settings")]).creation_flags(NO_WINDOW).output().unwrap();
    assert!(String::from_utf8_lossy(&q2.stdout).contains("hello-backup"));
}

#[test]
fn no_backup_means_nothing_in_the_registry_is_deleted() {
    let t = Tmp::new();
    let e = t.env();
    let name = format!("PillDeepKeepApp{}", std::process::id());
    let sub = format!(r"Software\{name}");
    let _g = HkcuKey(sub.clone());
    reg_add(&sub, &["/v", "V", "/d", "1"]);
    let p = prog(&name, "", "");
    let c = Cand::new(What::Key(Hive::Hkcu, sub.clone()), Tier::Certain, "x", 0);
    let fail = |_: &str, _: &[u8]| -> Result<String, String> { Err("disk full".into()) };
    let r = delete_cands(&p, &[&c], &e, &RealReg, "2", false, &fail, &|_, _, _, _, _| {});
    assert_eq!((r.done.len(), r.failed.len()), (0, 1));
    assert!(r.failed[0].1.contains("no backup"));
    assert!(hkcu_exists(&sub), "the key must survive");
}

#[test]
fn reg_dump_is_regedit_compatible_for_values_too() {
    let name = format!("PillDeepDumpApp{}", std::process::id());
    let sub = format!(r"Software\{name}");
    let _g = HkcuKey(sub.clone());
    reg_add(&sub, &["/v", "One", "/d", "first"]);
    reg_add(&sub, &["/v", "Two", "/d", "second"]);
    let txt = reg_dump(&[(Hive::Hkcu, sub.clone(), Some("one".into()))]).unwrap();
    assert!(txt.contains("\"One\"=\"first\"") && !txt.contains("second"), "{txt}");
    assert!(reg_dump(&[(Hive::Hkcu, format!(r"Software\{name}\nope"), None)]).is_err());
    assert_eq!(reg_line("", 1, &[b'a', 0, 0, 0]), "@=\"a\"\r\n");
    assert_eq!(reg_line("n", 3, &[1, 2]), "\"n\"=hex:01,02\r\n");
    assert_eq!(reg_line("n", 7, &[0, 0]), "\"n\"=hex(7):00,00\r\n");
}

#[test]
fn files_go_to_the_recycle_bin_and_the_log_is_stored() {
    let t = Tmp::new();
    let e = denv(&t);
    let p = prog("PillDeepTestFolder", "", "");
    let dir = t.dir(r"u\AppData\Local\PillDeepTestFolder");
    t.file(r"u\AppData\Local\PillDeepTestFolder\a.txt", 10);
    let keep = t.dir(r"u\AppData\Local\PillDeepKeep");
    let found = scan_fs(&p, &e, false, &|_| None, &mut Budget::new(1000, 5));
    assert_eq!(found.len(), 1);
    let refs: Vec<&Cand> = found.iter().collect();
    let bk = t.dir("bk");
    let seen = RefCell::new(Vec::<(String, bool, usize, usize)>::new());
    let r = delete_cands(&p, &refs, &e, &FakeReg::default(), "3", false, &|n, b| file_store(&bk, "x", n, b), &|s, ok, _, n, total| seen.borrow_mut().push((s.to_string(), ok, n, total)));
    assert_eq!(seen.borrow().len(), 1, "one progress report per item");
    assert!(seen.borrow()[0].0.ends_with("PillDeepTestFolder") && seen.borrow()[0].1 && seen.borrow()[0].2 == 1 && seen.borrow()[0].3 == 1);
    assert_eq!((r.done.len(), r.failed.len()), (1, 0), "{:?}", r.failed);
    assert!(!dir.exists() && keep.exists());
    assert!(r.backup.is_none());
    let log = std::fs::read_to_string(r.log.unwrap()).unwrap();
    assert!(log.contains("deleted folder") && log.contains("PillDeepTestFolder"), "{log}");
}

#[test]
fn administrator_items_are_left_alone_without_elevation() {
    let t = Tmp::new();
    let e = denv(&t);
    let p = foo();
    let f = t.dir(r"pf\Foo Editor");
    let mut c = Cand::new(What::Fs(f.clone()), Tier::Certain, "install folder", 0);
    c.admin = fs_admin(&f, &e);
    assert!(c.admin);
    let svc = Cand::new(What::Service("FooUpdater".into()), Tier::Certain, "x", 0);
    let key = Cand::new(What::Key(Hive::Hklm, r"SOFTWARE\Foo Editor".into()), Tier::Certain, "x", 0);
    let r = delete_cands(&p, &[&c, &svc, &key], &e, &reg_fixture(), "4", false, &|_, _| Err("must not be called".into()), &|_, _, _, _, _| {});
    assert_eq!((r.need_admin, r.done.len(), r.failed.len()), (3, 0, 0));
    assert!(f.exists());
}

#[test]
fn forged_candidates_are_refused_by_the_guard() {
    let t = Tmp::new();
    let e = denv(&t);
    let p = foo();
    let victim = t.file(r"win\Foo Editor.dll", 1);
    let outside = t.dir(r"outside\Foo Editor");
    let mut forged = vec![
        Cand::new(What::Fs(victim.clone()), Tier::Certain, "x", 0),
        Cand::new(What::Fs(outside.clone()), Tier::Certain, "x", 0),
        Cand::new(What::Fs(e.profile.clone()), Tier::Certain, "x", 0),
        Cand::new(What::Fs(e.windows.join("System32")), Tier::Certain, "x", 0),
        Cand::new(What::Fs(PathBuf::from(r"C:\")), Tier::Certain, "x", 0),
        Cand::new(What::Key(Hive::Hkcu, r"Software\Microsoft".into()), Tier::Certain, "x", 0),
        Cand::new(What::Key(Hive::Hkcu, r"Software".into()), Tier::Certain, "x", 0),
        Cand::new(What::Value(Hive::Hkcu, r"Software\Microsoft\Windows\CurrentVersion\Run".into(), "Anything".into()), Tier::Certain, "x", 0),
        Cand::new(What::Service("wuauserv".into()), Tier::Certain, "x", 0),
        Cand::new(What::Task(r"\Microsoft\Windows\Defrag\ScheduledDefrag".into()), Tier::Certain, "x", 0),
        Cand::new(What::Remote("x".into()), Tier::Certain, "x", 0),
    ];
    for c in &mut forged {
        c.admin = false; // pretend the elevated process is the one asking
    }
    let refs: Vec<&Cand> = forged.iter().collect();
    let r = delete_cands(&p, &refs, &e, &RealReg, "5", true, &|_, _| Err("no".into()), &|_, _, _, _, _| {});
    assert_eq!((r.done.len(), r.failed.len()), (0, forged.len()), "{:?}", r.done);
    assert!(victim.exists() && outside.exists() && e.profile.exists());
}

#[test]
fn full_scan_orders_user_items_first_and_certain_before_probable() {
    let t = Tmp::new();
    let e = denv(&t);
    let mut p = foo();
    p.location = t.dir(r"pf\Foo Editor").to_string_lossy().into_owned();
    t.dir(r"u\AppData\Roaming\Foo Editor");
    t.dir(r"u\Documents\Foo Editor Notes");
    let r = reg_fixture();
    let (c, manual, cut) = scan_all_with(&p, &e, &r, false, &|_, _| {});
    assert!(!cut);
    assert!(manual.is_empty());
    let keys: Vec<String> = c.iter().map(|x| cand_key(&x.what)).collect();
    let mut dedup = keys.clone();
    dedup.sort();
    dedup.dedup();
    assert_eq!(keys.len(), dedup.len(), "no duplicates");
    let first_admin = c.iter().position(|x| x.admin).unwrap();
    assert!(c[..first_admin].iter().all(|x| !x.admin) && c[first_admin..].iter().all(|x| x.admin));
    let user: Vec<&Cand> = c.iter().filter(|x| !x.admin).collect();
    let first_probable = user.iter().position(|x| x.tier == Tier::Probable).unwrap();
    assert!(user[..first_probable].iter().all(|x| x.tier == Tier::Certain) && user[first_probable..].iter().all(|x| x.tier == Tier::Probable));
}

// keep the fake env helper used (it documents the Windows-looking layout the lexical rules are written for)
#[test]
fn deep_guard_lexical_rules_on_a_windows_looking_layout() {
    let e = fake();
    let p = foo();
    for b in [r"C:\", r"D:\", r"C:\Windows\WinSxS", r"C:\Windows\WinSxS\Foo Editor", r"C:\Windows\System32\Foo Editor", r"C:\Program Files\Common Files", r"C:\Program Files (x86)\Common Files",
        r"C:\ProgramData", r"C:\Users", r"C:\Users\other", r"C:\Users\other\AppData\Roaming", r"C:\Users\other\Documents", r"C:\Users\usr", r"C:\Users\usr\Documents", r"C:\Windows\Temp",
        r"C:\Program Files\Foo Editor\..\Other", r"C:\Users\usr\AppData\Local\Pill\Foo Editor", r"C:\Users\Public\Foo Editor"] {
        assert!(guard_deep(Path::new(b), &e, &p).is_err(), "{b}");
    }
}

// ---------------------------------------------------------------- the conversation with the elevated helper (no elevation needed)

fn listener() -> (TcpListener, u16) {
    let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = l.local_addr().unwrap().port();
    (l, port)
}
const TOKEN: &str = "0123456789abcdef0123456789abcdef";

#[test]
fn helper_conversation_scan_and_refusals() {
    let t = Tmp::new();
    let e = denv(&t);
    let inst = t.dir(r"pf\Foo Editor");
    let mut p = foo();
    p.location = inst.to_string_lossy().into_owned();
    t.dir(r"ou\AppData\Local\FooEditor"); // an administrator item (another profile)
    t.dir(r"u\AppData\Roaming\Foo Editor"); // a user item: the helper does not report it
    let mut r = FakeReg::default();
    r.key(Hive::Hklm, r"SYSTEM\CurrentControlSet\Services\FooUpdater", &[("type", Val::D(0x10)), ("imagepath", Val::E(format!(r#""{}\updater.exe" /svc"#, inst.display())))]);
    let (l, port) = listener();
    // helper in a thread, using the same code as the elevated process but our fixtures
    let (e2, r2) = (e.clone(), r);
    let h = std::thread::spawn(move || worker_with(port, TOKEN, &e2, &r2));
    let res = serve(&l, TOKEN, &p, Mode::Scan, vec![], &|_, _| Err("no".into()), 10).unwrap();
    h.join().unwrap().unwrap();
    let items = res["items"].as_array().unwrap();
    assert!(!items.is_empty());
    assert!(items.iter().any(|i| i["key"].as_str().unwrap().starts_with("svc:fooupdater")), "{items:?}");
    assert!(items.iter().any(|i| i["shown"].as_str().unwrap().to_lowercase().contains(r"ou\appdata\local\fooeditor")));
    assert!(!items.iter().any(|i| i["shown"].as_str().unwrap().to_lowercase().contains(r"u\appdata\roaming")), "user items stay with the user process");
    // a forged identity gets an error and nothing else
    let mut bad = foo();
    bad.name = "Tools".into();
    let (l2, port2) = listener();
    let (e3, r3) = (e.clone(), FakeReg::default());
    let h = std::thread::spawn(move || worker_with(port2, TOKEN, &e3, &r3));
    let res = serve(&l2, TOKEN, &bad, Mode::Delete, vec![r"key:HKLM\software\microsoft".into()], &|_, _| Err("no".into()), 10).unwrap();
    h.join().unwrap().unwrap();
    assert!(res["error"].as_str().is_some() && res.get("done").is_none(), "{res}");
    // the app is still installed: refused too
    let mut live = FakeReg::default();
    let exe = t.file(r"pf\Foo Editor\unins000.exe", 2);
    live.key(Hive::Hklm, &format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{G}"), &[("displayname", sv("Foo Editor")), ("uninstallstring", sv(&format!("\"{}\"", exe.display())))]);
    let (l3, port3) = listener();
    let e4 = e.clone();
    let h = std::thread::spawn(move || worker_with(port3, TOKEN, &e4, &live));
    let res = serve(&l3, TOKEN, &p, Mode::Delete, vec![], &|_, _| Err("no".into()), 10).unwrap();
    h.join().unwrap().unwrap();
    assert_eq!(res["error"], "the app is still installed");
}

#[test]
fn helper_with_a_wrong_token_is_turned_away() {
    let (l, port) = listener();
    let h = std::thread::spawn(move || {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        send(&mut s, &json!({ "hello": "ffffffffffffffffffffffffffffffff" })).unwrap();
    });
    let r = serve(&l, TOKEN, &foo(), Mode::Scan, vec![], &|_, _| Ok(String::new()), 10);
    h.join().unwrap();
    assert!(r.is_err());
    // nobody connects: gives up
    let (l2, _) = listener();
    assert!(serve(&l2, TOKEN, &foo(), Mode::Scan, vec![], &|_, _| Ok(String::new()), 0).is_err());
}

#[test]
fn only_known_backup_names_are_stored_for_the_helper() {
    let (l, port) = listener();
    let stored = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let st = stored.clone();
    let h = std::thread::spawn(move || {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let mut rd = BufReader::new(s.try_clone().unwrap());
        send(&mut s, &json!({ "hello": TOKEN })).unwrap();
        let _req = recv(&mut rd).unwrap();
        let b64 = base64::engine::general_purpose::STANDARD.encode(b"data");
        let mut replies = Vec::new();
        for name in [r"..\..\evil", "reg", r"C:\x", "task3.xml"] {
            send(&mut s, &json!({ "store": { "name": name, "b64": b64 } })).unwrap();
            replies.push(recv(&mut rd).unwrap()["ok"].as_bool().unwrap());
        }
        send(&mut s, &json!({ "result": { "done": [] } })).unwrap();
        *st.lock().unwrap() = replies.iter().map(|b| b.to_string()).collect();
    });
    let names = RefCell::new(Vec::<String>::new());
    let res = serve(&l, TOKEN, &foo(), Mode::Delete, vec![], &|n, b| { names.borrow_mut().push(format!("{n}:{}", b.len())); Ok("path".into()) }, 10).unwrap();
    h.join().unwrap();
    assert!(res.get("done").is_some());
    assert_eq!(*names.borrow(), vec!["reg:4", "task3.xml:4"]);
    assert_eq!(*stored.lock().unwrap(), vec!["false", "true", "false", "true"]);
}
