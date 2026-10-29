// Round-3 hardening tests: every finding of the security review has a test here that fails without its fix.
use super::tests::{prog, sv, Tmp};
use super::tests_deep::{denv, foo, listener, reg_fixture, FakeReg, G, OTHER_G, TOKEN};
use std::io::{BufReader, Write};
use std::net::TcpStream;
use std::os::windows::fs::OpenOptionsExt;
use std::time::{Duration, Instant};
use super::*;

fn dirs(t: &Tmp, rels: &[&str]) -> Vec<PathBuf> {
    rels.iter().map(|r| t.dir(r)).collect()
}

// ---------------------------------------------------------------- 1. generic first component / deny list

#[test]
fn generic_first_component_install_locations_are_never_trusted() {
    let t = Tmp::new();
    let e = denv(&t);
    let bad = dirs(
        &t,
        &[
            r"pd\Microsoft\Crypto", r"pd\Microsoft\Windows\Start Menu", r"pd\Package Cache\{0A1B2C3D-4E5F-6789-ABCD-EF0123456789}v1.0", r"pf\Common Files\Services", r"pf\Common Files\System",
            r"pf\Common Files\Microsoft Shared\ink", r"pf\Windows Defender Advanced Threat Protection\x", r"pf\Windows Defender\x", r"pf\WindowsApps\Foo.App_1.0", r"pf\Windows NT\Accessories",
            r"pf\Windows Mail\x", r"pf\Windows Media Player\x", r"pf\Windows Photo Viewer\x", r"pf\Windows Sidebar\x", r"pf\Reference Assemblies\Microsoft", r"pf\MSBuild\x", r"pf\dotnet\x",
            r"pf\Microsoft Office\root", r"pf\Microsoft SQL Server\150", r"pf\Internet Explorer\x", r"pf\ModifiableWindowsApps\x", r"pf\WindowsPowerShell\Modules", r"pf\Application Verifier\x",
            r"pf\Uninstall Information\x", r"pf\NVIDIA Corporation\Drs\x", r"pf86\Common Files\Services", r"pf86\Microsoft Visual Studio\2022", r"win\System32\x", r"win\WinSxS\x",
        ],
    );
    for b in &bad {
        // as the recorded install folder (the old code let the install record rescue a generic first folder), and as a path below it
        let mut p = foo();
        p.location = b.to_string_lossy().into_owned();
        let kid = b.join("Foo Editor");
        std::fs::create_dir_all(&kid).unwrap();
        for target in [b.clone(), kid.clone()] {
            assert!(guard_deep(&target, &e, &p).is_err(), "{} (install {})", target.display(), b.display());
            assert!(guard(&target, &e, Some(b)).is_err(), "strict guard {}", target.display());
        }
        assert!(!install_usable(b, &e), "install_usable {}", b.display());
        let req = Req { mode: Mode::Delete, name: "Foo Editor".into(), publisher: "Acme".into(), location: b.to_string_lossy().into_owned(), keys: vec![] };
        assert!(validate_req(&req, &e, &FakeReg::default()).is_err(), "validate {}", b.display());
    }
    // and a plain app folder next to them still works
    let ok = t.dir(r"pf\Foo Editor\data");
    let mut p = foo();
    p.location = t.0.join(r"pf\Foo Editor").to_string_lossy().into_owned();
    assert_eq!(guard_deep(&ok, &e, &p), Ok(()));
}

#[test]
fn denied_trees_are_case_blind_and_junction_proof() {
    let t = Tmp::new();
    let e = denv(&t);
    assert!(e.denied(&t.0.join(r"PD\MICROSOFT\crypto")));
    assert!(e.denied(&t.0.join(r"pf\windows defender")));
    assert!(!e.denied(&t.0.join(r"pf\Foo Editor")));
    // a junction named like an app that leads into a denied tree is refused after canonicalisation
    let target = t.dir(r"pd\Microsoft\Crypto");
    let link = t.0.join(r"pf\Foo Editor");
    let st = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&link).arg(&target).creation_flags(NO_WINDOW).output().unwrap();
    assert!(st.status.success());
    assert!(guard_deep(&link, &e, &foo()).is_err());
    let _ = std::process::Command::new("cmd").args(["/c", "rmdir"]).arg(&link).creation_flags(NO_WINDOW).output();
    assert!(target.exists());
}

// ---------------------------------------------------------------- 3a. no environment variables

#[test]
fn the_real_environment_ignores_environment_variables() {
    let honest = Env::real();
    let vars = ["windir", "ProgramFiles", "ProgramData", "PUBLIC", "USERPROFILE", "APPDATA", "LOCALAPPDATA", "ProgramW6432"];
    let saved: Vec<Option<std::ffi::OsString>> = vars.iter().map(|v| std::env::var_os(v)).collect();
    for v in vars {
        std::env::set_var(v, r"C:\Evil\Place");
    }
    let poisoned = Env::real();
    for (v, s) in vars.iter().zip(saved) {
        match s {
            Some(x) => std::env::set_var(v, x),
            None => std::env::remove_var(v),
        }
    }
    assert_eq!(honest.windows, poisoned.windows);
    assert_eq!(honest.pf, poisoned.pf);
    assert_eq!(honest.programdata, poisoned.programdata);
    assert_eq!(honest.profile, poisoned.profile);
    assert_eq!(honest.appdata, poisoned.appdata);
    assert_eq!(honest.local, poisoned.local);
    assert_eq!(honest.public_desktop, poisoned.public_desktop);
    assert!(!poisoned.windows.to_string_lossy().contains("Evil") && !poisoned.pf.to_string_lossy().contains("Evil"));
    assert!(poisoned.strict_drives);
    // %VAR% in registry data is expanded from the trusted places too
    let v = vec![("programfiles".to_string(), r"C:\Program Files".to_string())];
    assert_eq!(expand_with(r"%ProgramFiles%\Foo\a.exe", &v), r"C:\Program Files\Foo\a.exe");
    assert_eq!(expand_with(r"%Unknown%\x", &v), r"%Unknown%\x");
    assert!(!expand_env(r"%SystemRoot%\x").contains('%'));
}

// ---------------------------------------------------------------- 7. junctions at or above the allowed parent

#[test]
fn a_junction_as_the_install_folder_or_above_it_is_refused() {
    let t = Tmp::new();
    let e = denv(&t);
    let target = t.dir(r"outside\Precious\Foo Editor");
    t.file(r"outside\Precious\Foo Editor\keep.txt", 3);
    // the install folder itself is a junction
    let link = t.0.join(r"d\Apps\Foo Editor");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    let st = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&link).arg(&target).creation_flags(NO_WINDOW).output().unwrap();
    assert!(st.status.success());
    let mut p = foo();
    p.location = link.to_string_lossy().into_owned();
    assert!(guard(&link, &e, Some(&link)).is_err(), "the junction is its own allowed parent: still refused");
    assert!(guard(&link.join("keep.txt"), &e, Some(&link)).is_err());
    assert!(guard_deep(&link, &e, &p).is_err());
    assert!(!reparse_free(&link) && !reparse_free(&link.join("keep.txt")));
    // an ancestor above the install folder is a junction
    let up = t.0.join(r"d\Big");
    let st = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&up).arg(t.0.join("outside")).creation_flags(NO_WINDOW).output().unwrap();
    assert!(st.status.success());
    let deep = up.join(r"Precious\Foo Editor");
    let mut q = foo();
    q.location = deep.to_string_lossy().into_owned();
    assert!(guard(&deep, &e, Some(&deep)).is_err());
    assert!(!install_usable(&deep, &e) || !reparse_free(&deep));
    assert!(guard_deep(&deep.join("keep.txt"), &e, &q).is_err());
    for l in [&link, &up] {
        let _ = std::process::Command::new("cmd").args(["/c", "rmdir"]).arg(l).creation_flags(NO_WINDOW).output();
    }
    assert!(target.join("keep.txt").exists());
}

// ---------------------------------------------------------------- 4 (guard half). fixed drives only

#[test]
fn only_fixed_local_drives_are_accepted() {
    let t = Tmp::new();
    let mut e = denv(&t);
    e.strict_drives = true;
    let here = t.dir(r"pf\Foo Editor");
    assert!(drive_is_fixed(&here), "the temp folder is on a fixed drive");
    assert!(e.is_fixed(&here));
    // a drive letter that is not a fixed disk here (nothing mounted on Q: on a normal PC) and non-drive forms
    for bad in [r"Q:\Apps\Foo Editor", r"\\server\share\Foo Editor", r"\\?\C:\x", r"relative\x", r"C:x", ""] {
        assert!(!drive_is_fixed(Path::new(bad)) || Path::new(bad).exists(), "{bad}");
    }
    let off = Path::new(r"Q:\Apps\Foo Editor");
    if !drive_is_fixed(off) {
        let mut p = foo();
        p.location = off.to_string_lossy().into_owned();
        assert_eq!(guard_deep(off, &e, &p), Err("not on a fixed local drive"));
        assert!(!install_usable(off, &e));
    }
    // sync folders are not install folders
    for s in [r"u\OneDrive - Contoso\Foo Editor", r"u\Dropbox\Foo Editor"] {
        let d = t.dir(s);
        assert!(!install_usable(&d, &denv(&t)), "{s}");
    }
}

// ---------------------------------------------------------------- 8. split_cmd with a char whose lowercase is longer

#[test]
fn split_cmd_survives_characters_that_change_length_when_lowercased() {
    // 'İ' (U+0130) lowercases to two chars: byte offsets of to_lowercase() no longer match the original
    let line = r"C:\Program Files\İİİİ\x İ.exe /s";
    let r = split_cmd(line);
    assert!(r.is_some());
    assert_eq!(split_cmd(r"C:\İ\a.EXE --x").map(|x| x.1), Some("--x".to_string()));
    assert!(split_cmd("İİİ .exe").is_some());
    assert!(prog("İstanbul App", "", "").name.len() > 3);
    let _ = (sv, G);
}

#[test]
fn generic_first_component_is_not_rescued_by_an_install_record() {
    let t = Tmp::new();
    let e = denv(&t);
    // not on the deny list, but the first folder below an allowed root is a generic word: the install record must not make it deletable
    for rel in [r"pd\Data\Foo Editor", r"pd\Shared\Foo Editor", r"pd\Temp\Foo Editor", r"u\AppData\Local\Packages\Foo Editor", r"u\AppData\Roaming\Programs\Foo Editor", r"u\AppData\Local\Cache\Foo Editor"] {
        let d = t.dir(rel);
        let mut p = foo();
        p.location = d.to_string_lossy().into_owned();
        assert!(install_usable(&d, &e), "{rel}: the install folder itself looks believable");
        assert!(guard(&d, &e, Some(&d)).is_err(), "{rel} strict");
        assert!(guard_deep(&d, &e, &p).is_err(), "{rel} deep");
        assert!(guard_deep(&d.join("x"), &e, &p).is_err() || !d.join("x").exists(), "{rel} child");
    }
}

// ---------------------------------------------------------------- 2. the exe that gets elevated

#[test]
fn the_elevated_exe_must_be_the_pinned_one() {
    let t = Tmp::new();
    let exe = t.file("pill.exe", 5000);
    let other = t.file("other.exe", 5000);
    std::fs::write(&exe, vec![7u8; 3_000_000]).unwrap(); // bigger than the 1 MB read chunk
    let h = hash_path(&exe).unwrap();
    // the same content through a handle that is held open (the pin) gives the same digest
    let pinned = std::fs::OpenOptions::new().read(true).share_mode(1).open(&exe).unwrap();
    assert_eq!(hash_handle(&pinned).unwrap(), h);
    assert_eq!(check_exe(&h, &exe, &exe), Ok(()));
    // a different file at the path (a swap) is caught by the digest
    std::fs::write(&other, vec![8u8; 3_000_000]).unwrap();
    assert_eq!(check_exe(&h, &other, &other), Err("pill.exe does not match the copy that started"));
    // the path handed to the permission prompt is not the running module
    assert_eq!(check_exe(&h, &other, &exe), Err("the elevation path is not the running program"));
    assert_eq!(check_exe(&h, &t.0.join("missing.exe"), &t.0.join("missing.exe")), Err("pill.exe cannot be read"));
    // while the pin is open nobody can overwrite, rename or delete the file
    assert!(std::fs::write(&exe, b"evil").is_err(), "overwrite blocked by the pin");
    assert!(std::fs::remove_file(&exe).is_err(), "delete blocked by the pin");
    assert!(std::fs::rename(&exe, t.0.join("moved.exe")).is_err(), "rename blocked by the pin");
    drop(pinned);
    // the running module is what the system says it is
    assert!(module_path().is_some_and(|m| m.is_absolute() && m.is_file()));
}

#[test]
fn tools_are_started_by_absolute_path() {
    for n in ["sc.exe", "schtasks.exe", "msiexec.exe"] {
        let p = system_exe(n);
        assert!(p.is_absolute() && p.is_file(), "{}", p.display());
        assert!(p.starts_with(system32()));
    }
    // the source never starts a bare tool name
    let src = include_str!("../uninstall.rs");
    for bare in ["Command::new(\"sc.exe\")", "Command::new(\"schtasks.exe\")", "Command::new(\"powershell.exe\")", "Command::new(\"explorer.exe\")"] {
        let n = src.lines().filter(|l| l.contains(bare) && !l.trim_start().starts_with("//")).count();
        assert_eq!(n, 0, "{bare}");
    }
}

#[test]
fn session_tokens_are_random_hex() {
    let (a, b) = (token().unwrap(), token().unwrap());
    assert!(a.len() == 32 && a.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    assert_ne!(a, b);
    assert!(valid_port_token(&format!("5000:{a}")).is_some());
}

// ---------------------------------------------------------------- 3. forged request scope

#[test]
fn the_worker_finds_guids_itself_from_dead_records_and_installer_records_of_its_own_folder() {
    let t = Tmp::new();
    let e = denv(&t);
    let inst = t.dir(r"pf\Foo Editor");
    let p = prog("Foo Editor", "Acme", &inst.to_string_lossy());
    let un = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
    let mut r = FakeReg::default();
    // a dead record of this app: its GUID counts
    r.key(Hive::Hklm, &format!(r"{un}\{G}"), &[("displayname", sv("Foo Editor")), ("uninstallstring", sv(r"C:\gone\u.exe"))]);
    // a dead record of ANOTHER app, and an installer record pointing elsewhere: neither counts
    r.key(Hive::Hklm, &format!(r"{un}\{{99999999-9999-9999-9999-999999999999}}"), &[("displayname", sv("Victim")), ("uninstallstring", sv(r"C:\gone\v.exe"))]);
    let ud = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Installer\UserData\S-1-5-18\Products";
    let mine = pack_guid("{11111111-2222-3333-4444-555555555555}").unwrap();
    let theirs = pack_guid("{77777777-7777-7777-7777-777777777777}").unwrap();
    r.key(Hive::Hklm, &format!(r"{ud}\{mine}\InstallProperties"), &[("installlocation", sv(&inst.to_string_lossy()))]);
    r.key(Hive::Hklm, &format!(r"{ud}\{theirs}\InstallProperties"), &[("installlocation", sv(r"C:\Program Files\Elsewhere"))]);
    let g = derive_guids(&p, p.install().as_deref(), &r);
    assert!(g.contains(&G.to_string()) && g.contains(&"{11111111-2222-3333-4444-555555555555}".to_string()), "{g:?}");
    assert!(!g.iter().any(|x| x.starts_with("{99999999") || x.starts_with("{77777777")), "{g:?}");
    assert_eq!(unpack_guid(&mine).as_deref(), Some("{11111111-2222-3333-4444-555555555555}"));
    assert_eq!(unpack_guid(&pack_guid(G).unwrap()).as_deref(), Some(G));
    assert_eq!(unpack_guid("short"), None);
    // a LIVE entry of the same name stops the elevated work altogether, and its GUID is not collected
    let live = t.file(r"pf\Other\u.exe", 3);
    r.key(Hive::Hklm, &format!(r"{un}\{{88888888-8888-8888-8888-888888888888}}"), &[("displayname", sv("Foo Editor")), ("uninstallstring", sv(&live.to_string_lossy()))]);
    let req = Req { mode: Mode::Delete, name: "Foo Editor".into(), publisher: "Acme".into(), location: inst.to_string_lossy().into_owned(), keys: vec![] };
    assert_eq!(validate_req(&req, &e, &r).err(), Some("the app is still installed"));
    assert!(!derive_guids(&p, p.install().as_deref(), &r).iter().any(|x| x.starts_with("{88888888")));
}

#[test]
fn a_forged_guid_never_makes_class_or_installer_keys_certain() {
    let t = Tmp::new();
    let e = denv(&t);
    let mut p = foo();
    p.location = t.dir(r"pf\Foo Editor").to_string_lossy().into_owned();
    p.extra_guids = vec![OTHER_G.to_string()]; // even a GUID the worker believes in
    let mut r = FakeReg::default();
    let l = Hive::Hklm;
    let pk = pack_guid(OTHER_G).unwrap();
    // CLSID / Interface / AppID / TypeLib with the GUID but no server of the app: Probable at most
    for t_ in ["CLSID", "Interface", "AppID", "TypeLib"] {
        r.key(l, &format!(r"SOFTWARE\Classes\{t_}\{OTHER_G}"), &[("", sv("x"))]);
    }
    r.key(l, &format!(r"SOFTWARE\Classes\CLSID\{OTHER_G}\InprocServer32"), &[("", sv(r"C:\Windows\System32\victim.dll"))]);
    r.key(l, &format!(r"SOFTWARE\Classes\Installer\Products\{pk}"), &[("producticon", sv(r"C:\Windows\Installer\victim.ico"))]);
    let ud = format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Installer\UserData\S-1-5-18\Products\{pk}");
    r.key(l, &format!(r"{ud}\InstallProperties"), &[("installlocation", sv(r"C:\Program Files\Victim"))]);
    let comp = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Installer\UserData\S-1-5-18\Components\ABC";
    r.key(l, comp, &[(&pk.to_lowercase(), sv(r"C:\Program Files\Victim\v.dll"))]);
    for k in [format!(r"SOFTWARE\Classes\CLSID\{OTHER_G}"), format!(r"SOFTWARE\Classes\Interface\{OTHER_G}"), format!(r"SOFTWARE\Classes\AppID\{OTHER_G}"), format!(r"SOFTWARE\Classes\TypeLib\{OTHER_G}"),
        format!(r"SOFTWARE\Classes\Installer\Products\{pk}"), ud.clone()] {
        let got = guard_cand(&What::Key(l, k.clone()), &p, &e, &r);
        assert!(!matches!(got, Ok((Tier::Certain, _))), "{k}: {got:?}");
    }
    let v = guard_cand(&What::Value(l, comp.into(), pk.to_lowercase()), &p, &e, &r);
    assert!(matches!(v, Ok((Tier::Probable, _))), "{v:?}");
    // the same keys with their OWN files in the app folder are Certain
    r.key(l, &format!(r"SOFTWARE\Classes\CLSID\{OTHER_G}\InprocServer32"), &[("", sv(&format!(r"{}\ok.dll", p.location)))]);
    assert!(matches!(guard_cand(&What::Key(l, format!(r"SOFTWARE\Classes\CLSID\{OTHER_G}")), &p, &e, &r), Ok((Tier::Certain, _))));
    r.key(l, &format!(r"SOFTWARE\Classes\Installer\Products\{pk}"), &[("producticon", sv(&format!(r"{}\i.ico", p.location)))]);
    assert!(matches!(guard_cand(&What::Key(l, format!(r"SOFTWARE\Classes\Installer\Products\{pk}")), &p, &e, &r), Ok((Tier::Certain, _))));
    r.key(l, &format!(r"{ud}\InstallProperties"), &[("installlocation", sv(&p.location))]);
    assert!(matches!(guard_cand(&What::Key(l, ud), &p, &e, &r), Ok((Tier::Certain, _))));
    r.key(l, comp, &[(&pk.to_lowercase(), sv(&format!(r"{}\v.dll", p.location)))]);
    assert!(matches!(guard_cand(&What::Value(l, comp.into(), pk.to_lowercase()), &p, &e, &r), Ok((Tier::Certain, _))));
}

#[test]
fn an_msi_app_without_an_install_location_gets_name_evidence_only() {
    let t = Tmp::new();
    let e = denv(&t);
    let mut p = foo();
    p.location.clear();
    let r = reg_fixture();
    let found = scan_registry(&p, &e, &r, &mut Budget::new(100_000, 20), &|_, _| {});
    let keys: Vec<String> = found.iter().map(|c| cand_key(&c.what)).collect();
    // nothing that needs a folder to point into: no service, no firewall rule, no autorun, no App Paths, no COM server, no ProgID command
    for bad in ["svc:", "firewallrules", "currentversion\\run", "app paths", "fooeditor.document", "foohere", "fooshell"] {
        assert!(!keys.iter().any(|k| k.contains(bad)), "{bad}: {keys:?}");
    }
    assert!(keys.contains(&r"key:HKCU\software\foo editor".to_string()), "name evidence still works");
    let req = Req { mode: Mode::Scan, name: "Foo Editor".into(), publisher: "Acme".into(), location: String::new(), keys: vec![] };
    assert!(validate_req(&req, &e, &FakeReg::default()).is_ok());
}

#[test]
fn still_registered_notices_the_same_product_family() {
    let t = Tmp::new();
    let e = denv(&t);
    let un = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
    t.file(r"pf\Foo Suite\bin\suite.exe", 3);
    let icon = t.file(r"pf\Foo Suite\icons\app.ico", 3);
    let inst = t.dir(r"pf\Foo Suite\Editor");
    // a living entry whose DisplayIcon is inside the same Program Files subfolder
    let mut r = FakeReg::default();
    r.key(Hive::Hklm, &format!(r"{un}\Suite"), &[("displayname", sv("Foo Suite")), ("displayicon", sv(&format!("{},0", icon.display())))]);
    let req = Req { mode: Mode::Delete, name: "Foo Editor Pro".into(), publisher: "Acme".into(), location: inst.to_string_lossy().into_owned(), keys: vec![] };
    assert_eq!(validate_req(&req, &e, &r).err(), Some("the app is still installed"));
    // the short product name equal to a living entry's short name
    let mut r2 = FakeReg::default();
    let other = t.file(r"pf\Elsewhere\u.exe", 3);
    r2.key(Hive::Hklm, &format!(r"{un}\x"), &[("displayname", sv("Foo Editor Pro 2.1 (x64)")), ("uninstallstring", sv(&other.to_string_lossy()))]);
    let req2 = Req { mode: Mode::Delete, name: "Foo Editor Pro".into(), publisher: "Acme".into(), location: String::new(), keys: vec![] };
    assert!(validate_req(&req2, &e, &r2).is_err());
    // nothing registered: fine
    assert!(validate_req(&req, &e, &FakeReg::default()).is_ok());
}

#[test]
fn the_worker_asks_before_it_deletes_and_does_nothing_on_cancel() {
    let t = Tmp::new();
    let e = denv(&t);
    let inst = t.dir(r"pf\Foo Editor");
    t.file(r"pf\Foo Editor\foo.exe", 5);
    let p = prog("Foo Editor", "Acme", &inst.to_string_lossy());
    let c = Cand::new(What::Fs(inst.clone()), Tier::Certain, "install folder", 5);
    let seen = std::cell::RefCell::new(String::new());
    // cancel: nothing deleted
    assert!(!decide(&p, &[&c], &|t| {
        *seen.borrow_mut() = t.to_string();
        false
    }));
    assert!(seen.borrow().contains("Foo Editor") && seen.borrow().contains(&inst.to_string_lossy().into_owned()) && seen.borrow().contains("1 folder"));
    assert!(decide(&p, &[&c], &|_| true));
    assert!(!decide(&p, &[], &|_| true), "nothing picked: no dialog, no delete");
    // the text counts per kind and shows only the first 15 items
    let many: Vec<Cand> = (0..20).map(|i| Cand::new(What::Service(format!("Svc{i}")), Tier::Certain, "x", 0)).collect();
    let refs: Vec<&Cand> = many.iter().collect();
    let text = confirm_text(&p, &refs);
    assert!(text.contains("20 service") && text.contains("Svc14") && !text.contains("Svc15") && text.contains("and 5 more") && text.contains("Press OK"));
    // end to end through the worker: Cancel leaves the install folder in place
    let (l, port) = listener();
    let (e2, key) = (e.clone(), cand_key(&c.what));
    let h = std::thread::spawn(move || worker_with(port, TOKEN, &e2, &FakeReg::default(), &|_| false));
    let res = serve(&l, TOKEN, &p, Mode::Delete, vec![key], &|_, _| Err("no".into()), 10, &|| true).unwrap();
    h.join().unwrap().unwrap();
    assert_eq!(res["error"], "cancelled in the administrator dialog", "{res}");
    assert!(inst.exists());
    // Windows\Temp items are never deleted by the worker
    let wt = t.dir(r"win\Temp\Foo Editor");
    let ct = Cand::new(What::Fs(wt), Tier::Certain, "x", 0);
    assert!(worker_refuses(&ct, &e) && !worker_refuses(&c, &e));
}

// ---------------------------------------------------------------- 10. the loopback conversation

#[test]
fn strangers_cannot_take_the_place_of_the_helper() {
    let (l, port) = listener();
    let p = foo();
    let h = std::thread::spawn(move || {
        // 1: connects and says nothing, 2: wrong token, 3: garbage, then the real helper
        let _quiet = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let mut bad = TcpStream::connect(("127.0.0.1", port)).unwrap();
        send(&mut bad, &json!({ "hello": "ffffffffffffffffffffffffffffffff" })).unwrap();
        let mut junk = TcpStream::connect(("127.0.0.1", port)).unwrap();
        junk.write_all(b"not json\n").unwrap();
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let mut rd = BufReader::new(s.try_clone().unwrap());
        send(&mut s, &json!({ "hello": TOKEN })).unwrap();
        let req = recv(&mut rd).unwrap();
        send(&mut s, &json!({ "hb": 1 })).unwrap();
        send(&mut s, &json!({ "result": { "name": req["name"] } })).unwrap();
    });
    let res = serve(&l, TOKEN, &p, Mode::Scan, vec![], &|_, _| Ok(String::new()), 20, &|| true).unwrap();
    h.join().unwrap();
    assert_eq!(res["name"], "Foo Editor", "the real helper was served after the strangers were dropped");
}

#[test]
fn a_helper_that_dies_or_goes_quiet_is_noticed() {
    let (l, port) = listener();
    let h = std::thread::spawn(move || {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let mut rd = BufReader::new(s.try_clone().unwrap());
        send(&mut s, &json!({ "hello": TOKEN })).unwrap();
        let _ = recv(&mut rd);
        std::thread::sleep(Duration::from_secs(5)); // silent, and its process is reported gone
    });
    let t0 = Instant::now();
    let r = serve(&l, TOKEN, &foo(), Mode::Scan, vec![], &|_, _| Ok(String::new()), 10, &|| false);
    assert!(r.is_err() && r.unwrap_err().contains("stopped") && t0.elapsed() < Duration::from_secs(4));
    h.join().unwrap();
}

#[test]
fn the_reader_keeps_a_half_received_line_across_timeouts() {
    let (l, port) = listener();
    let h = std::thread::spawn(move || {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.write_all(b"{\"hb\":").unwrap();
        std::thread::sleep(Duration::from_millis(900));
        s.write_all(b"1}\n").unwrap();
        std::thread::sleep(Duration::from_millis(300));
    });
    let (c, _) = l.accept().unwrap();
    c.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
    let mut lines = Lines::new(c);
    let mut got = None;
    for _ in 0..20 {
        if let Some(v) = lines.next().unwrap() {
            got = Some(v);
            break;
        }
    }
    assert_eq!(got, Some(json!({ "hb": 1 })));
    h.join().unwrap();
}
