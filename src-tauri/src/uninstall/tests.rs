// Tests use synthetic registry values and folders built under a temp directory of their own; nothing real is uninstalled.
// The only real side effects: a tiny temp folder lands in the Recycle Bin (recycle test), and one throw-away HKCU test key is created and removed.
use super::*;
use std::sync::atomic::AtomicU32;

static N: AtomicU32 = AtomicU32::new(0);

/// A fresh directory under the system temp dir, removed on drop.
struct Tmp(PathBuf);
impl Tmp {
    fn new() -> Tmp {
        let n = N.fetch_add(1, Ordering::Relaxed);
        let base = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        let d = plain(base).join(format!("pill-uninst-test-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        Tmp(d)
    }
    fn dir(&self, rel: &str) -> PathBuf {
        let p = self.0.join(rel);
        std::fs::create_dir_all(&p).unwrap();
        p
    }
    fn file(&self, rel: &str, bytes: usize) -> PathBuf {
        let p = self.0.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, vec![b'x'; bytes]).unwrap();
        p
    }
    /// An Env whose every root lives inside this temp dir.
    fn env(&self) -> Env {
        let r = &self.0;
        let (appdata, programdata) = (r.join(r"u\AppData\Roaming"), r.join("pd"));
        for d in [&appdata, &r.join(r"u\AppData\Local"), &programdata, &r.join("pf"), &r.join("pf86"), &r.join("win"), &r.join(r"u\Desktop"), &r.join(r"pub\Desktop")] {
            std::fs::create_dir_all(d).unwrap();
        }
        let start = |b: &Path| b.join(r"Microsoft\Windows\Start Menu\Programs");
        std::fs::create_dir_all(start(&appdata)).unwrap();
        std::fs::create_dir_all(start(&programdata)).unwrap();
        Env {
            profile: r.join("u"),
            windows: r.join("win"),
            local: r.join(r"u\AppData\Local"),
            pf: r.join("pf"),
            pf86: r.join("pf86"),
            desktop: r.join(r"u\Desktop"),
            public_desktop: r.join(r"pub\Desktop"),
            start_user: start(&appdata),
            start_common: start(&programdata),
            pill: vec![r.join(r"u\AppData\Local\Pill"), r.join(r"u\AppData\Roaming\com.pill.dev")],
            appdata,
            programdata,
        }
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Paths that exist nowhere: the lexical rules are checked on a Windows-looking layout.
fn fake() -> Env {
    Env {
        profile: r"C:\Users\usr".into(),
        windows: r"C:\Windows".into(),
        appdata: r"C:\Users\usr\AppData\Roaming".into(),
        local: r"C:\Users\usr\AppData\Local".into(),
        programdata: r"C:\ProgramData".into(),
        pf: r"C:\Program Files".into(),
        pf86: r"C:\Program Files (x86)".into(),
        desktop: r"C:\Users\usr\Desktop".into(),
        public_desktop: r"C:\Users\Public\Desktop".into(),
        start_user: r"C:\Users\usr\AppData\Roaming\Microsoft\Windows\Start Menu\Programs".into(),
        start_common: r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs".into(),
        pill: vec![r"C:\Users\usr\AppData\Local\Pill".into(), r"C:\Users\usr\AppData\Roaming\com.pill.dev".into()],
    }
}

fn prog(name: &str, publisher: &str, location: &str) -> Prog {
    Prog { name: name.into(), publisher: publisher.into(), location: location.into(), uninst: r#""C:\x\unins000.exe""#.into(), ..Default::default() }
}

fn vals(kv: &[(&str, Val)]) -> Vals {
    kv.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}
fn sv(s: &str) -> Val {
    Val::S(s.into())
}

// ---------------------------------------------------------------- names

#[test]
fn normalisation_and_generic_words() {
    assert_eq!(norm(" Foo-Bar 2 "), "foobar2");
    assert_eq!(norm("ПрИвЕт, мир"), "приветмир");
    assert!(usable("foo") && usable("7zip"));
    assert!(!usable("ab")); // too short
    for g in ["app", "apps", "data", "tools", "common", "microsoft", "windows", "system", "program", "programs", "update", "updates", "install", "setup", "cache", "temp", "logs"] {
        assert!(!usable(g), "{g}");
    }
    assert_eq!(short_name("7-Zip 23.01 (x64)"), "7-Zip");
    assert_eq!(short_name("Mozilla Firefox (x64 en-US)"), "Mozilla Firefox");
    assert_eq!(short_name("Python 3.12.1 (64-bit)"), "Python");
    assert_eq!(short_name("Foo - Bar edition"), "Foo");
    assert_eq!(short_name("2024"), "2024"); // never below one token
    assert_eq!(short_name("(x64)"), "(x64)");
}

#[test]
fn names_drop_generic_and_publisher_suffixes() {
    let n = names_of(&prog("Foo Editor 1.2.3", "Acme Software Inc.", ""));
    assert_eq!(n.product, vec!["fooeditor123", "fooeditor"]);
    assert_eq!(n.publisher, vec!["acme"]);
    let g = names_of(&prog("Tools", "Microsoft Corporation", ""));
    assert!(g.product.is_empty() && g.publisher.is_empty(), "{g:?}");
    assert!(names_of(&prog("Go", "", "")).product.is_empty()); // 2 chars
}

// ---------------------------------------------------------------- uninstall strings and registry fixtures

#[test]
fn split_command_lines() {
    assert_eq!(split_cmd(r#""C:\Program Files\Foo Bar\unins000.exe" /SILENT /x"#), Some((r"C:\Program Files\Foo Bar\unins000.exe".into(), "/SILENT /x".into())));
    assert_eq!(split_cmd(r"C:\Program Files\Foo Bar\unins000.exe /SILENT"), Some((r"C:\Program Files\Foo Bar\unins000.exe".into(), "/SILENT".into())));
    assert_eq!(split_cmd(r"C:\Foo\x.exe"), Some((r"C:\Foo\x.exe".into(), String::new())));
    assert_eq!(split_cmd(r#"  "C:\a b\c.exe"  "#), Some((r"C:\a b\c.exe".into(), String::new())));
    assert_eq!(split_cmd(r#""C:\unterminated"#), None);
    assert_eq!(split_cmd("   "), None);
    assert_eq!(split_cmd("MsiExec.exe /X{AAAAAAAA-0000-0000-0000-000000000001}"), Some(("MsiExec.exe".into(), "/X{AAAAAAAA-0000-0000-0000-000000000001}".into())));
    // ".exe" inside a folder name is not the end of the file name when the longer path is a real file
    let t = Tmp::new();
    let f = t.file(r"a.exe folder\u.exe", 1);
    let line = format!("{} -q", f.display());
    assert_eq!(split_cmd(&line), Some((f.to_string_lossy().into_owned(), "-q".into())));
}

#[test]
fn guid_and_msi_command() {
    let g = "{0A1B2C3D-4E5F-6789-ABCD-EF0123456789}";
    assert_eq!(find_guid(&format!("MsiExec.exe /I{g}")).as_deref(), Some(g));
    assert_eq!(find_guid("{0A1B2C3D-4E5F-6789-ABCD-EF012345678}"), None); // short
    assert_eq!(find_guid("{ZA1B2C3D-4E5F-6789-ABCD-EF0123456789}"), None);
    let p = parse_entry(true, &format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{g}"), &vals(&[("displayname", sv("Foo")), ("uninstallstring", sv(&format!("MsiExec.exe /I{g}")))])).unwrap();
    assert_eq!(build_cmd(&p, false).unwrap(), ("msiexec.exe".into(), format!("/x {g}")));
    assert_eq!(build_cmd(&p, true).unwrap(), ("msiexec.exe".into(), format!("/x {g} /passive /norestart")));
    assert!(can_silent(&p));
    // WindowsInstaller=1 with only the GUID key name
    let q = parse_entry(true, &format!(r"X\{g}"), &vals(&[("displayname", sv("Foo")), ("windowsinstaller", Val::D(1))])).unwrap();
    assert_eq!(q.msi.as_deref(), Some(g));
    // plain exe: loud = UninstallString, silent only with QuietUninstallString
    let e = parse_entry(false, "X\\Foo", &vals(&[("displayname", sv("Foo")), ("uninstallstring", sv(r#""C:\F\u.exe""#))])).unwrap();
    assert_eq!(build_cmd(&e, false).unwrap().0, r"C:\F\u.exe");
    assert!(!can_silent(&e) && build_cmd(&e, true).is_err());
    let e2 = parse_entry(false, "X\\Foo", &vals(&[("displayname", sv("Foo")), ("uninstallstring", sv("u.exe")), ("quietuninstallstring", sv(r#""C:\F\u.exe" /S"#))])).unwrap();
    assert_eq!(build_cmd(&e2, true).unwrap(), (r"C:\F\u.exe".into(), "/S".into()));
}

#[test]
fn registry_entry_filtering() {
    let base = [("displayname", sv("Foo")), ("uninstallstring", sv(r#""C:\F\u.exe""#))];
    let with = |extra: (&str, Val)| {
        let mut v = base.to_vec();
        v.push(extra);
        parse_entry(true, "K\\Foo", &vals(&v)).unwrap()
    };
    assert_eq!(classify(&parse_entry(true, "K\\Foo", &vals(&base)).unwrap(), None), Some(false));
    assert_eq!(classify(&with(("systemcomponent", Val::D(1))), None), None);
    assert_eq!(classify(&with(("parentkeyname", sv("Windows"))), None), None);
    assert_eq!(classify(&with(("parentdisplayname", sv("Windows"))), None), None);
    assert_eq!(classify(&with(("releasetype", sv("Security Update"))), None), None);
    assert_eq!(classify(&with(("releasetype", sv("Hotfix"))), None), None);
    assert_eq!(classify(&with(("noremove", Val::D(1))), None), None);
    assert!(parse_entry(true, "K\\x", &vals(&[("uninstallstring", sv("u.exe"))])).is_none()); // no DisplayName
    // no uninstall string at all
    assert_eq!(classify(&parse_entry(true, "K\\Foo", &vals(&[("displayname", sv("Foo"))])).unwrap(), None), None);
    // updates by name
    for n in ["Update for Windows (KB5000)", "Security Update for X", "Something (KB123456)", "Hotfix for Y"] {
        let p = parse_entry(true, "K\\u", &vals(&[("displayname", sv(n)), ("uninstallstring", sv("u.exe"))])).unwrap();
        assert_eq!(classify(&p, None), None, "{n}");
    }
    // Pill itself, by name and by its uninstaller's folder
    assert_eq!(classify(&prog("Pill", "", ""), None), None);
    let mut me = prog("Whatever", "", "");
    me.uninst = r#""C:\Users\usr\AppData\Local\Pill\uninstall.exe" /S"#.into();
    assert_eq!(classify(&me, Some(Path::new(r"C:\Users\usr\AppData\Local\Pill"))), None);
    assert_eq!(classify(&me, Some(Path::new(r"C:\Other"))), Some(false));
    // runtimes and drivers are listed as risky (the list hides them unless asked)
    for n in ["Microsoft Visual C++ 2015-2022 Redistributable (x64)", "Microsoft .NET Runtime - 8.0", "NVIDIA Graphics Driver 551", "Microsoft Edge WebView2 Runtime"] {
        assert_eq!(classify(&prog(n, "", ""), None), Some(true), "{n}");
    }
    // values keyed in any case, DWORD given as a string
    let v = vals(&[("displayname", sv(" Foo ")), ("uninstallstring", sv("u.exe")), ("estimatedsize", sv("2048")), ("systemcomponent", sv("1"))]);
    let p = parse_entry(true, "K\\Foo", &v).unwrap();
    assert_eq!((p.name.as_str(), p.size_kb, p.sys), ("Foo", 2048, true));
}

#[test]
fn bar_trigger_words() {
    assert_eq!(trigger("u foo"), Some("foo"));
    assert_eq!(trigger("del  foo bar "), Some("foo bar"));
    assert_eq!(trigger("Uninstall x"), Some("x"));
    assert_eq!(trigger("удалить фото"), Some("фото"));
    assert_eq!(trigger("u "), Some(""));
    assert_eq!(trigger("u"), None);
    assert_eq!(trigger("delta x"), None);
    assert_eq!(trigger("c foo"), None);
}

#[test]
fn store_apps_json() {
    let one = r#"{"Name":"Contoso.App","PackageFullName":"Contoso.App_1.0.0.0_x64__8wekyb3d8bbwe","Publisher":"CN=Contoso, O=Contoso, C=US","Version":"1.0.0.0"}"#;
    let p = parse_appx(one);
    assert_eq!((p.len(), p[0].appx, p[0].publisher.as_str()), (1, true, "Contoso"));
    assert_eq!(parse_appx(&format!("[{one},{one}]")).len(), 2);
    assert!(parse_appx("garbage").is_empty());
    assert!(parse_appx(r#"{"Name":"X","PackageFullName":"a'; rm -rf","Publisher":"","Version":""}"#).is_empty()); // never reaches a command line
    assert!(valid_full_name("A.B_1.0_x64__abc") && !valid_full_name("a b") && !valid_full_name("a'b") && !valid_full_name(""));
}

// ---------------------------------------------------------------- the guard

#[test]
fn guard_refuses_dangerous_paths() {
    let e = fake();
    let bad = [
        r"C:\",
        r"C:",
        r"C:\Windows",
        r"C:\Windows\System32",
        r"C:\Windows\System32\drivers\etc",
        r"C:\Windows\Temp\Foo",
        r"C:\Users",
        r"C:\Users\usr",
        r"C:\Users\usr\Documents",
        r"C:\Users\usr\Desktop",
        r"C:\Users\usr\AppData",
        r"C:\Users\usr\AppData\Local",
        r"C:\Users\usr\AppData\Roaming",
        r"C:\Users\usr\AppData\Local\Programs",
        r"C:\Program Files",
        r"C:\Program Files (x86)",
        r"C:\ProgramData",
        r"C:\Users\usr\AppData\Local\Pill",
        r"C:\Users\usr\AppData\Local\Pill\uninstall-backups",
        r"C:\Users\usr\AppData\Roaming\com.pill.dev\store.db",
        r"C:\Program Files\Foo\..\Bar",
        r"C:\Program Files\Foo\..",
        r"C:\Program Files\.\Foo",
        r"Foo",
        r"..\Foo",
        r"\\server\share\Foo",
        r"\\?\C:\Program Files\Foo",
        "",
        r"D:\",
        r"D:\Games\Foo", // nothing records it as the install folder
        r"C:\Users\usr\AppData\Roaming\Microsoft", // generic name right below an allowed root
        r"C:\Users\usr\AppData\Local\Temp",
        r"C:\Users\usr\AppData\Local\Microsoft\Windows\Foo",
        r"C:\Users\usr\AppData\Local\Temp\Foo",
        r"C:\Program Files\Common Files",
        r"C:\Program Files\WindowsApps",
        r"C:\Users\usr\AppData\Local\Packages",
        r"C:\Users\usr\AppData\Roaming\Microsoft\Windows\Start Menu\Programs", // the Start Menu root itself
        r"C:\Users\usr\Documents\Foo",
    ];
    for b in bad {
        assert!(guard(Path::new(b), &e, None).is_err(), "{b} must be refused");
    }
    // an install folder never makes the dangerous ones deletable
    for b in [r"C:\", r"C:\Windows", r"C:\Windows\System32", r"C:\Users\usr", r"C:\Program Files", r"C:\Users\usr\Documents"] {
        assert!(guard(Path::new(b), &e, Some(Path::new(b))).is_err(), "{b} as its own install folder");
        assert!(guard(Path::new(r"C:\Users\usr\Documents\x"), &e, Some(Path::new(b))).is_err(), "inside {b}");
    }
    // a recorded install folder inside a protected tree is not believed
    assert!(guard(Path::new(r"C:\Windows\Foo\bar"), &e, Some(Path::new(r"C:\Windows\Foo"))).is_err());
    assert!(guard(Path::new(r"C:\Users\usr\Documents\Foo"), &e, Some(Path::new(r"C:\Users\usr\Documents\Foo"))).is_err());
    assert!(guard(Path::new(r"C:\Users\usr\AppData\Local\Pill\a"), &e, Some(Path::new(r"C:\Users\usr\AppData\Local\Pill"))).is_err());
    assert!(guard(Path::new(r"D:\Tools\x"), &e, Some(Path::new(r"D:\Tools"))).is_err()); // generic folder name
}

#[test]
fn guard_accepts_real_app_folders_only_when_inside_allowed_places() {
    let t = Tmp::new();
    let e = t.env();
    let ok_dirs = [e.local.join("Foo"), e.appdata.join("Foo"), e.programdata.join("Foo"), e.pf.join("Foo"), e.local.join(r"Programs\Foo"), e.start_user.join("Foo")];
    // %LOCALAPPDATA%\Programs is protected as a root but its children are fine
    for d in &ok_dirs {
        std::fs::create_dir_all(d).unwrap();
        assert_eq!(guard(d, &e, None), Ok(()), "{}", d.display());
    }
    let lnk = e.desktop.join("Foo.lnk");
    std::fs::write(&lnk, b"x").unwrap();
    assert_eq!(guard(&lnk, &e, None), Ok(()));
    // does not exist
    assert_eq!(guard(&e.local.join("Missing"), &e, None), Err("does not exist"));
    // case-insensitive root match still works
    let up = PathBuf::from(e.local.join("Foo").to_string_lossy().to_uppercase());
    assert!(guard(&up, &e, None).is_ok());
    // outside every root but recorded as the install folder
    let game = t.dir(r"games\Foo");
    assert!(guard(&game, &e, None).is_err());
    assert_eq!(guard(&game, &e, Some(&game)), Ok(()));
    let save = t.dir(r"games\Foo\saves");
    assert_eq!(guard(&save, &e, Some(&game)), Ok(()));
    // a sibling that merely shares the prefix is not inside it
    let sib = t.dir(r"games\FooBar");
    assert!(guard(&sib, &e, Some(&game)).is_err());
    // the install folder must not be a drive-root level or an ancestor of a protected folder
    let root_like = PathBuf::from(t.0.to_string_lossy().to_string()); // contains every root of this Env
    assert!(guard(&root_like.join("games"), &e, Some(&root_like)).is_err());
}

#[test]
fn guard_refuses_junctions_and_links() {
    let t = Tmp::new();
    let e = t.env();
    let target = t.dir("outside\\Precious");
    t.file("outside\\Precious\\keep.txt", 5);
    let link = e.local.join("Foo");
    let st = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&link).arg(&target).creation_flags(NO_WINDOW).output().unwrap();
    assert!(st.status.success(), "mklink failed: {}", String::from_utf8_lossy(&st.stdout));
    assert!(guard(&link, &e, None).is_err(), "junction itself");
    assert!(guard(&link.join("keep.txt"), &e, None).is_err(), "file reached through a junction");
    // a real folder below a junction is refused too
    t.dir("outside\\Precious\\sub");
    assert!(guard(&link.join("sub"), &e, None).is_err());
    // the scan neither lists nor follows it
    let p = prog("Foo", "", "");
    let mut b = Budget::new(1000, 5);
    let found = scan_fs(&p, &e, false, &|_| None, &mut b);
    assert!(found.is_empty(), "{found:?}");
    // removing the junction first lets the cleanup of the temp dir succeed without touching the target
    let _ = std::process::Command::new("cmd").args(["/c", "rmdir"]).arg(&link).creation_flags(NO_WINDOW).output();
    assert!(target.join("keep.txt").exists());
}

#[test]
fn registry_guards() {
    let n = names_of(&prog("Foo Editor", "Acme Inc", ""));
    assert!(key_guard(r"Software\Foo Editor", &n).is_ok());
    assert!(key_guard(r"software\FOOEDITOR", &n).is_ok());
    assert!(key_guard(r"Software\Acme\Foo Editor", &n).is_ok());
    for bad in [
        r"Software",
        r"Software\Acme",
        r"Software\Microsoft\Windows",
        r"Software\Microsoft\Windows\CurrentVersion\Run",
        r"Software\Classes\Foo Editor",
        r"Software\Other\Foo Editor",
        r"Software\Acme\Other",
        r"Software\FooEditor\Sub\Deeper",
        r"Software\FooEditor\..\Microsoft",
        r"Software\\FooEditor",
        r"SYSTEM\Foo Editor",
        r"Foo Editor",
        "",
    ] {
        assert!(key_guard(bad, &n).is_err(), "{bad}");
    }
    // generic or short names never produce a deletable key
    let g = names_of(&prog("Microsoft", "Microsoft", ""));
    assert!(key_guard(r"Software\Microsoft", &g).is_err());
    let e = fake();
    let inst = Path::new(r"D:\Apps\FooEditor");
    assert!(run_guard(r#""D:\Apps\FooEditor\foo.exe" --tray"#, Some(inst), &e).is_ok());
    assert!(run_guard(r"D:\apps\fooeditor\sub\foo.exe", Some(inst), &e).is_ok());
    assert!(run_guard(r"D:\Apps\FooEditorPro\foo.exe", Some(inst), &e).is_err()); // sibling prefix
    assert!(run_guard(r"D:\Apps\FooEditor\..\x.exe", Some(inst), &e).is_err());
    assert!(run_guard(r"C:\Windows\explorer.exe", Some(inst), &e).is_err());
    assert!(run_guard(r"D:\Apps\FooEditor\foo.exe", None, &e).is_err());
    assert!(run_guard(r"C:\Windows\x.exe", Some(Path::new(r"C:\Windows")), &e).is_err());
    assert!(run_guard("", Some(inst), &e).is_err());
}

// ---------------------------------------------------------------- scan

#[test]
fn scan_finds_exact_names_and_skips_look_alikes() {
    let t = Tmp::new();
    let e = t.env();
    let install = t.dir("pf\\Foo Editor");
    t.file("pf\\Foo Editor\\foo.exe", 100);
    t.dir("u\\AppData\\Local\\FooEditor"); // normalised equal
    t.dir("u\\AppData\\Roaming\\foo editor");
    t.dir("u\\AppData\\Roaming\\Acme\\FooEditor");
    t.dir("u\\AppData\\Roaming\\Acme\\Other");
    t.dir("pd\\Foo Editor");
    // must NOT match
    t.dir("u\\AppData\\Local\\FooEditorPro");
    t.dir("u\\AppData\\Local\\Foo");
    t.dir("u\\AppData\\Local\\Editor");
    t.dir("u\\AppData\\Local\\FooBar");
    t.dir("u\\AppData\\Local\\Data");
    t.file("u\\AppData\\Local\\FooEditor.txt", 3); // a file with a folder-like name
    // shortcuts
    t.file("u\\Desktop\\Foo Editor.lnk", 1);
    t.file("u\\Desktop\\Other.lnk", 1);
    t.file("u\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\Acme\\Foo Editor.lnk", 1);
    t.file("u\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\Stranger.lnk", 1); // target inside the install folder
    t.file("u\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\Unrelated.lnk", 1);
    let p = prog("Foo Editor 1.2", "Acme Inc.", &install.to_string_lossy());
    let ins = install.clone();
    let lnk = move |l: &Path| (l.file_name().unwrap() == "Stranger.lnk").then(|| ins.join("foo.exe"));
    let mut b = Budget::new(10_000, 10);
    let found = scan_fs(&p, &e, false, &lnk, &mut b);
    let rel = |c: &Cand| match &c.what {
        What::Fs(p) => p.strip_prefix(&t.0).unwrap().to_string_lossy().to_lowercase(),
        _ => String::new(),
    };
    let mut got: Vec<(String, bool)> = found.iter().map(|c| (rel(c), c.ticked)).collect();
    got.sort();
    let want = [
        (r"pf\foo editor", true),
        (r"pd\foo editor", true),
        (r"u\appdata\local\fooeditor", true),
        (r"u\appdata\roaming\foo editor", true),
        (r"u\appdata\roaming\acme", false), // publisher folder: listed, never pre-ticked
        (r"u\appdata\roaming\acme\fooeditor", true),
        (r"u\desktop\foo editor.lnk", true),
        (r"u\appdata\roaming\microsoft\windows\start menu\programs\acme\foo editor.lnk", true),
        (r"u\appdata\roaming\microsoft\windows\start menu\programs\stranger.lnk", true),
    ];
    let mut want: Vec<(String, bool)> = want.iter().map(|(a, b)| (a.to_string(), *b)).collect();
    want.sort();
    assert_eq!(got, want);
    // the install folder reports its size
    assert!(found.iter().any(|c| c.size == 100 && c.note == "install folder"));
    // every ticked candidate passes the guard
    for c in &found {
        if let What::Fs(path) = &c.what {
            assert_eq!(guard(path, &e, Some(&install)), Ok(()), "{}", path.display());
        }
    }
    // shared install folder: not pre-ticked
    let mut b2 = Budget::new(10_000, 10);
    let shared = scan_fs(&p, &e, true, &|_| None, &mut b2);
    assert!(shared.iter().any(|c| matches!(&c.what, What::Fs(x) if *x == install) && !c.ticked));
}

#[test]
fn scan_never_matches_generic_or_short_names() {
    let t = Tmp::new();
    let e = t.env();
    for d in ["App", "Data", "Tools", "Microsoft", "Common", "Windows", "Fo", "Temp", "Update"] {
        t.dir(&format!("u\\AppData\\Local\\{d}"));
        t.dir(&format!("u\\AppData\\Roaming\\{d}"));
    }
    for name in ["App", "Data", "Tools", "Microsoft", "Common", "Windows", "Fo", "Temp", "Update", "Logs"] {
        let found = scan_fs(&prog(name, "Microsoft Corporation", ""), &e, false, &|_| None, &mut Budget::new(10_000, 10));
        assert!(found.is_empty(), "{name}: {found:?}");
        assert!(scan_keys(&prog(name, "Microsoft", ""), &|_| vec![name.into(), "Microsoft".into()]).is_empty(), "{name}");
    }
    // an install folder that is a generic name is not offered either
    let tools = t.dir("pf\\Tools");
    let found = scan_fs(&prog("Tools Pro", "", &tools.to_string_lossy()), &e, false, &|_| None, &mut Budget::new(1000, 5));
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn scan_is_bounded() {
    let t = Tmp::new();
    let e = t.env();
    for i in 0..50 {
        t.dir(&format!("u\\AppData\\Local\\dir{i}"));
    }
    t.dir("u\\AppData\\Local\\Foo");
    let mut b = Budget::new(5, 10);
    let _ = scan_fs(&prog("Foo", "", ""), &e, false, &|_| None, &mut b);
    assert!(b.cut, "entry cap must trip");
    let mut b = Budget::new(1_000_000, 0);
    std::thread::sleep(Duration::from_millis(5));
    assert_eq!(dir_size(&e.local, &mut b), 0);
    assert!(b.cut, "time cap must trip");
}

#[test]
fn registry_scan_with_fixtures() {
    let tree: HashMap<&str, Vec<&str>> = HashMap::from([
        ("Software", vec!["Acme", "FooEditor", "Microsoft", "Classes", "Foo", "Other"]),
        ("Software\\Acme", vec!["Foo Editor", "Unrelated"]),
        ("Software\\Microsoft", vec!["Foo Editor"]),
    ]);
    let children = |s: &str| tree.get(s).map(|v| v.iter().map(|x| x.to_string()).collect()).unwrap_or_default();
    let p = prog("Foo Editor", "Acme Inc", "");
    let mut shown: Vec<String> = scan_keys(&p, &children).iter().map(|c| c.shown.clone()).collect();
    shown.sort();
    assert_eq!(shown, vec![r"HKCU\Software\Acme\Foo Editor", r"HKCU\Software\FooEditor"]);
    assert!(scan_keys(&p, &children).iter().all(|c| c.ticked));
    // every candidate passes its own guard
    for c in scan_keys(&p, &children) {
        if let What::Key(k) = &c.what {
            assert!(key_guard(k, &names_of(&p)).is_ok());
        }
    }
    let e = fake();
    let p2 = prog("Foo Editor", "", r"D:\Apps\FooEditor");
    let vals = vec![
        ("FooEditor".to_string(), r#""D:\Apps\FooEditor\foo.exe" --tray"#.to_string()),
        ("Other".to_string(), r"C:\Windows\explorer.exe".to_string()),
        ("Sibling".to_string(), r"D:\Apps\FooEditorPro\x.exe".to_string()),
    ];
    let runs = scan_run(&p2, &vals, &e);
    assert_eq!(runs.len(), 1);
    assert!(matches!(&runs[0].what, What::Run(_, n, _) if n == "FooEditor"));
    assert!(scan_run(&prog("Foo Editor", "", ""), &vals, &e).is_empty(), "no recorded install folder, no autorun match");
}

// ---------------------------------------------------------------- deleting

#[test]
fn delete_sends_to_recycle_bin_and_logs() {
    let t = Tmp::new();
    let e = t.env();
    let dir = t.dir("u\\AppData\\Local\\PillUninstTestFolder");
    t.file("u\\AppData\\Local\\PillUninstTestFolder\\a.txt", 10);
    let keep = t.dir("u\\AppData\\Local\\PillUninstKeep"); // not selected
    let p = prog("PillUninstTestFolder", "", "");
    let mut b = Budget::new(1000, 5);
    let cands = scan_fs(&p, &e, false, &|_| None, &mut b);
    assert_eq!(cands.len(), 1);
    let refs: Vec<&Cand> = cands.iter().collect();
    let out = t.dir("backups");
    let r = delete_cands(&p, &refs, &e, &out, "t1");
    assert_eq!((r.done.len(), r.failed.len()), (1, 0), "{:?}", r.failed);
    assert!(!dir.exists() && keep.exists());
    assert!(r.backup.is_none());
    let log = std::fs::read_to_string(&r.log).unwrap();
    assert!(log.contains("deleted folder") && log.contains("PillUninstTestFolder"), "{log}");
}

#[test]
fn delete_rechecks_every_item_with_the_guard() {
    let t = Tmp::new();
    let e = t.env();
    let win = e.windows.clone();
    std::fs::write(win.join("keep.dll"), b"x").unwrap();
    let outside = t.dir("outside\\Foo");
    let p = prog("Foo", "", "");
    let mk = |path: &Path| Cand { kind: "folder", shown: path.to_string_lossy().into_owned(), size: 0, ticked: true, note: "", what: What::Fs(path.to_path_buf()) };
    let (c1, c2, c3) = (mk(&win), mk(&outside), mk(&e.profile));
    let r = delete_cands(&p, &[&c1, &c2, &c3], &e, &t.dir("bk"), "t2");
    assert_eq!((r.done.len(), r.failed.len()), (0, 3));
    assert!(win.join("keep.dll").exists() && outside.exists() && e.profile.exists());
    // registry candidates the guards do not accept are refused as well
    let bad_key = Cand { kind: "key", shown: String::new(), size: 0, ticked: true, note: "", what: What::Key(r"Software\Microsoft".into()) };
    let bad_run = Cand { kind: "autorun", shown: String::new(), size: 0, ticked: true, note: "", what: What::Run(RUN_KEY.into(), "x".into(), r"C:\Windows\explorer.exe".into()) };
    let r = delete_cands(&p, &[&bad_key, &bad_run], &e, &t.dir("bk2"), "t3");
    assert_eq!((r.done.len(), r.failed.len()), (0, 2));
    assert!(r.backup.is_none());
}

/// Removes the throw-away key even when an assertion fails.
struct HkcuTestKey(String);
impl Drop for HkcuTestKey {
    fn drop(&mut self) {
        let _ = std::process::Command::new("reg").args(["delete", &format!(r"HKCU\{}", self.0), "/f"]).creation_flags(NO_WINDOW).output();
    }
}
fn hkcu_exists(sub: &str) -> bool {
    std::process::Command::new("reg").args(["query", &format!(r"HKCU\{sub}")]).creation_flags(NO_WINDOW).output().is_ok_and(|o| o.status.success())
}

#[test]
fn registry_keys_are_backed_up_before_they_are_deleted() {
    let t = Tmp::new();
    let e = t.env();
    let name = format!("PillUninstTestApp{}", std::process::id());
    let sub = format!(r"Software\{name}");
    let _guard = HkcuTestKey(sub.clone());
    let add = std::process::Command::new("reg").args(["add", &format!(r"HKCU\{sub}\Settings"), "/v", "Marker", "/t", "REG_SZ", "/d", "hello-backup", "/f"]).creation_flags(NO_WINDOW).output().unwrap();
    assert!(add.status.success());
    assert!(hkcu_exists(&sub));
    let p = prog(&name, "", "");
    let children = |s: &str| Key::open(HKEY_CURRENT_USER, s, KEY_READ).map(|k| k.subkeys(3000)).unwrap_or_default();
    let cands = scan_keys(&p, &children);
    assert_eq!(cands.len(), 1, "{cands:?}");
    let refs: Vec<&Cand> = cands.iter().collect();
    let out = t.dir("bk");
    let r = delete_cands(&p, &refs, &e, &out, "20260101");
    assert_eq!((r.done.len(), r.failed.len()), (1, 0), "{:?}", r.failed);
    assert!(!hkcu_exists(&sub), "key should be gone");
    let reg = r.backup.expect("backup path");
    let bytes = std::fs::read(&reg).unwrap();
    assert!(bytes.len() > 20 && bytes[..2] == [0xFF, 0xFE], "UTF-16LE with BOM");
    let text = String::from_utf16_lossy(&bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>());
    assert!(text.contains("Windows Registry Editor") && text.contains(&name) && text.contains("hello-backup"), "{text}");
    assert!(reg.file_name().unwrap().to_string_lossy().ends_with("-20260101.reg"));
    assert!(std::fs::read_to_string(&r.log).unwrap().contains("backup:"));
}

#[test]
fn no_backup_means_no_registry_delete() {
    let t = Tmp::new();
    let e = t.env();
    let name = format!("PillUninstGhost{}", std::process::id());
    let p = prog(&name, "", "");
    // the candidate names a key that cannot be exported (it does not exist): nothing may be deleted, nothing panics
    let c = Cand { kind: "key", shown: format!(r"HKCU\Software\{name}"), size: 0, ticked: true, note: "", what: What::Key(format!(r"Software\{name}")) };
    let r = delete_cands(&p, &[&c], &e, &t.dir("bk"), "t4");
    assert_eq!((r.done.len(), r.failed.len()), (0, 1));
    assert!(r.backup.is_none());
    assert!(std::fs::read_to_string(&r.log).unwrap().contains("backup failed"));
}

#[test]
fn export_merges_parts_with_one_header() {
    let t = Tmp::new();
    let pid = std::process::id();
    let (a, b) = (format!(r"Software\PillUninstMergeA{pid}"), format!(r"Software\PillUninstMergeB{pid}"));
    let _ga = HkcuTestKey(a.clone());
    let _gb = HkcuTestKey(b.clone());
    for (k, v) in [(&a, "one"), (&b, "two")] {
        assert!(std::process::Command::new("reg").args(["add", &format!(r"HKCU\{k}"), "/v", "V", "/d", v, "/f"]).creation_flags(NO_WINDOW).output().unwrap().status.success());
    }
    let out = t.0.join("m.reg");
    export_keys(&[a.clone(), b.clone()], &out).unwrap();
    let bytes = std::fs::read(&out).unwrap();
    let text = String::from_utf16_lossy(&bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>());
    assert_eq!(text.matches("Windows Registry Editor").count(), 1, "{text}");
    assert!(text.contains(&format!("MergeA{pid}")) && text.contains(&format!("MergeB{pid}")));
    assert!(export_keys(&[format!(r"Software\PillUninstNope{pid}")], &t.0.join("n.reg")).is_err());
}

#[test]
fn byte_formatting() {
    assert_eq!(fmt_size(0), "");
    assert_eq!(fmt_size(512), "512 KB");
    assert_eq!(fmt_size(2048), "2 MB");
    assert_eq!(fmt_size(3 << 20), "3.0 GB");
}
