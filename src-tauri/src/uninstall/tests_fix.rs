// Round-3 hardening tests: every finding of the security review has a test here that fails without its fix.
use super::tests::{prog, sv, Tmp};
use super::tests_deep::{denv, foo, hkcu_exists, listener, reg_add, reg_fixture, FakeReg, HkcuKey, G, OTHER_G, TOKEN};
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

// ---------------------------------------------------------------- 4. the Recycle Bin never turns into a permanent delete

#[test]
fn recycle_decisions_fail_closed() {
    let normal = BinPolicy { nuke: false, max_bytes: None, volume_bytes: 1000 * 1_000_000 };
    assert!(bin_decision(10_000, &normal, 100, true).is_ok());
    // not a fixed drive, a path longer than MAX_PATH, "remove immediately", bigger than the configured bin, bigger than the default tenth of the volume
    assert!(bin_decision(1, &normal, 100, false).unwrap_err().contains("fixed"));
    assert!(bin_decision(1, &normal, 260, true).unwrap_err().contains("259"));
    assert!(bin_decision(1, &normal, 259, true).is_ok());
    assert!(bin_decision(1, &BinPolicy { nuke: true, ..normal }, 100, true).unwrap_err().contains("immediately"));
    assert!(bin_decision(51_000_000, &BinPolicy { max_bytes: Some(50_000_000), ..normal }, 100, true).unwrap_err().contains("not recycled"));
    assert!(bin_decision(50_000_000, &BinPolicy { max_bytes: Some(50_000_000), ..normal }, 100, true).is_ok());
    assert!(bin_decision(101_000_000, &normal, 100, true).is_err(), "a tenth of the volume is the default capacity");
    // every refusal says nothing was deleted
    for e in [bin_decision(1, &normal, 100, false), bin_decision(1, &normal, 300, true), bin_decision(1, &BinPolicy { nuke: true, ..normal }, 100, true)] {
        assert!(e.unwrap_err().contains("nothing deleted"));
    }
    // the real policy of this PC's temp drive is readable
    let t = Tmp::new();
    assert!(bin_policy(&t.0).volume_bytes > 0);
}

#[test]
fn recycle_refuses_long_paths_links_and_other_drives_without_touching_anything() {
    let t = Tmp::new();
    let long = t.0.join("a".repeat(150)).join("b".repeat(150));
    std::fs::create_dir_all(&long).unwrap_or(());
    if long.exists() {
        assert!(recycle(&long).unwrap_err().contains("259"), "long path");
        assert!(long.exists());
    }
    let target = t.dir(r"outside\Precious");
    t.file(r"outside\Precious\keep.txt", 3);
    let link = t.0.join("link");
    let st = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&link).arg(&target).creation_flags(NO_WINDOW).output().unwrap();
    assert!(st.status.success());
    assert!(recycle(&link).is_err() && recycle(&link.join("keep.txt")).is_err());
    let _ = std::process::Command::new("cmd").args(["/c", "rmdir"]).arg(&link).creation_flags(NO_WINDOW).output();
    assert!(target.join("keep.txt").exists());
    assert!(recycle(Path::new(r"Q:\nothing\here")).is_err());
}

// ---------------------------------------------------------------- 5. over-eager Certain

#[test]
fn a_folder_at_the_top_of_a_drive_is_only_probable() {
    let t = Tmp::new();
    let e = denv(&t);
    let p = prog("Blender", "Blender Foundation", "");
    t.dir(r"d\Blender");
    t.dir(r"d\Projects\Blender");
    t.dir(r"d\Blender Foundation");
    let found = scan_fs(&p, &e, false, &|_| None, &mut Budget::new(10_000, 10));
    assert_eq!(found.len(), 3, "{found:?}");
    assert!(found.iter().all(|c| c.tier == Tier::Probable && !c.ticked), "{found:?}");
}

#[test]
fn a_product_family_key_is_probable_unless_every_subkey_is_this_version() {
    let t = Tmp::new();
    let e = denv(&t);
    let mut p = prog("Python 3.12.1 (64-bit)", "Python Software Foundation", "");
    p.version = "3.12.1150.0".into();
    let mut r = FakeReg::default();
    let u = Hive::Hkcu;
    r.key(u, r"Software\Python\PythonCore\3.11\InstallPath", &[]);
    r.key(u, r"Software\Python\PythonCore\3.12\InstallPath", &[]);
    let k = What::Key(u, r"Software\Python".into());
    assert_eq!(guard_cand(&k, &p, &e, &r).map(|x| x.0), Ok(Tier::Probable), "other versions live in it");
    // only this version (and structural names): Certain
    let mut r2 = FakeReg::default();
    r2.key(u, r"Software\Python\PythonCore\3.12\InstallPath", &[]);
    assert_eq!(guard_cand(&k, &p, &e, &r2).map(|x| x.0), Ok(Tier::Certain));
    let mut r3 = FakeReg::default();
    r3.key(u, r"Software\Python\Settings", &[]);
    assert_eq!(guard_cand(&k, &p, &e, &r3).map(|x| x.0), Ok(Tier::Certain));
    // under a publisher key too
    let mut r4 = FakeReg::default();
    let mut p2 = p.clone();
    p2.publisher = "Acme Inc".into();
    r4.key(u, r"Software\Acme\Python\3.9", &[]);
    assert_eq!(guard_cand(&What::Key(u, r"Software\Acme\Python".into()), &p2, &e, &r4).map(|x| x.0), Ok(Tier::Probable));
}

#[test]
fn a_publisher_folder_read_from_a_cut_listing_is_never_sole() {
    let t = Tmp::new();
    let e = denv(&t);
    let p = foo();
    t.dir(r"u\AppData\Roaming\Acme\Foo Editor");
    t.dir(r"u\AppData\Roaming\Acme\Zed Other App"); // sorts after the app folder: a cut listing shows only the app
    for n in 1..120 {
        let mut b = Budget::new(n, 10);
        let found = scan_fs(&p, &e, false, &|_| None, &mut b);
        assert!(!found.iter().any(|c| c.note == "publisher folder holding only this app"), "budget {n}: {found:?}");
    }
    // sanity: with a real sole folder and enough budget it is Certain
    let t2 = Tmp::new();
    let e2 = denv(&t2);
    t2.dir(r"u\AppData\Roaming\Acme\Foo Editor");
    let found = scan_fs(&p, &e2, false, &|_| None, &mut Budget::new(100_000, 10));
    assert!(found.iter().any(|c| c.note == "publisher folder holding only this app" && c.tier == Tier::Certain), "{found:?}");
    // the registry reader's cap: a listing at the cap is not "sole"
    struct Capped;
    impl Reg for Capped {
        fn subkeys(&self, _: Hive, sub: &str) -> Vec<String> {
            if sub.ends_with("Acme") { (0..20_000).map(|i| if i == 0 { "Foo Editor".to_string() } else { format!("X{i}") }).collect() } else { vec![] }
        }
        fn values(&self, _: Hive, _: &str) -> Vals {
            Vals::new()
        }
        fn exists(&self, _: Hive, _: &str) -> bool {
            true
        }
    }
    assert!(guard_cand(&What::Key(Hive::Hkcu, r"Software\Acme".into()), &p, &e, &Capped).is_err());
}

#[test]
fn doubtful_install_locations_are_probable() {
    let t = Tmp::new();
    let mut e = denv(&t);
    // a one-word folder of its own at the top of a drive that is not named like the app
    let top = t.dir(r"d\Games");
    let mut p = prog("Foo Editor", "Acme", &top.to_string_lossy());
    e.drives = vec![t.0.join("d")];
    let found = scan_fs(&p, &e, false, &|_| None, &mut Budget::new(10_000, 10));
    // generic names never make it as an install folder at all; use a non-generic one-word folder
    assert!(found.iter().all(|c| c.tier != Tier::Certain || !c.shown.ends_with("Games")));
    let top = t.dir(r"d\Stuff");
    p.location = top.to_string_lossy().into_owned();
    let found = scan_fs(&p, &e, false, &|_| None, &mut Budget::new(10_000, 10));
    let c = found.iter().find(|c| c.shown.ends_with("Stuff")).expect("listed");
    assert!(c.tier == Tier::Probable && !c.ticked && c.note.contains("top of a drive"), "{c:?}");
    // named like the app at the top of a drive is still the app's own install folder... but drive-level matches stay Probable by the other rule
    // a folder that holds several programs
    let multi = t.dir(r"pf\Suite");
    for s in ["a", "b", "c"] {
        t.file(&format!(r"pf\Suite\{s}\{s}.exe"), 3);
    }
    p.location = multi.to_string_lossy().into_owned();
    let found = scan_fs(&p, &e, false, &|_| None, &mut Budget::new(10_000, 10));
    let c = found.iter().find(|c| c.shown.ends_with("Suite")).expect("listed");
    assert!(c.tier == Tier::Probable && c.note.contains("several programs"), "{c:?}");
    // an ordinary Program Files install stays Certain
    let ok = t.dir(r"pf\Foo Editor");
    t.file(r"pf\Foo Editor\foo.exe", 3);
    p.location = ok.to_string_lossy().into_owned();
    let found = scan_fs(&p, &e, false, &|_| None, &mut Budget::new(10_000, 10));
    assert!(found.iter().any(|c| c.shown.ends_with(r"pf\Foo Editor") && c.tier == Tier::Certain && c.note == "install folder"), "{found:?}");
    // shared with another registered location: Probable (round-2 rule, still there)
    assert!(scan_fs(&p, &e, true, &|_| None, &mut Budget::new(10_000, 10)).iter().any(|c| c.tier == Tier::Probable && c.note.contains("shared")));
}

// ---------------------------------------------------------------- 6. delete order

#[test]
fn deletes_run_tasks_and_services_first_and_keys_last() {
    let mix = [
        What::Key(Hive::Hkcu, "k".into()),
        What::Value(Hive::Hkcu, "k".into(), "v".into()),
        What::Fs(PathBuf::from(r"C:\x\y")),
        What::Service("S".into()),
        What::Task(r"\T".into()),
    ];
    let mut sorted: Vec<&What> = mix.iter().collect();
    sorted.sort_by_key(|w| delete_rank(w));
    let order: Vec<&str> = sorted.iter().map(|w| match w { What::Task(_) => "task", What::Service(_) => "service", What::Fs(_) => "fs", What::Value(..) => "value", _ => "key" }).collect();
    assert_eq!(order, ["task", "service", "fs", "value", "key"]);
}

#[test]
fn a_service_that_is_still_running_or_only_marked_is_not_reported_deleted() {
    assert!(service_verdict(SvcState::Stopped, false).is_ok());
    assert!(service_verdict(SvcState::Missing, false).is_ok());
    assert!(service_verdict(SvcState::Running, false).unwrap_err().contains("still running"));
    assert!(service_verdict(SvcState::Stopped, true).unwrap_err().contains("marked for deletion"));
    assert_eq!(parse_sc_state("SERVICE_NAME: x\r\n        TYPE               : 10  WIN32_OWN_PROCESS\r\n        STATE              : 1  STOPPED\r\n"), SvcState::Stopped);
    assert_eq!(parse_sc_state("        STATE              : 4  RUNNING\r\n"), SvcState::Running);
    assert_eq!(parse_sc_state("        STATE              : 3  STOP_PENDING\r\n"), SvcState::Running, "pending is not stopped");
    assert_eq!(parse_sc_state("[SC] EnumQueryServicesStatus:OpenService FAILED 1060:\r\nThe specified service does not exist as an installed service."), SvcState::Missing);
    assert_eq!(parse_sc_state("garbage"), SvcState::Running, "unclear means do not claim it stopped");
}

#[test]
fn files_values_and_keys_are_removed_in_that_order() {
    let t = Tmp::new();
    let e = denv(&t);
    let name = format!("PillFixOrder{}", std::process::id());
    let sub = format!(r"Software\{name}");
    let _g = HkcuKey(sub.clone());
    reg_add(&sub, &["/v", "V", "/d", "1"]);
    let dir = t.dir(&format!(r"u\AppData\Local\{name}"));
    let p = prog(&name, "", "");
    let key = Cand::new(What::Key(Hive::Hkcu, sub.clone()), Tier::Certain, "x", 0);
    let fsc = Cand::new(What::Fs(dir), Tier::Certain, "x", 0);
    let order = std::cell::RefCell::new(Vec::<String>::new());
    let bk = t.dir("bk");
    let r = delete_cands(&p, &[&key, &fsc], &e, &RealReg, "7", false, &|n, b| file_store(&bk, "o", n, b), &|s, ok, _, _, _| order.borrow_mut().push(format!("{ok}:{}", if s.starts_with("HKCU") { "key" } else { "fs" })));
    assert_eq!((r.done.len(), r.failed.len()), (2, 0), "{:?}", r.failed);
    assert_eq!(*order.borrow(), vec!["true:fs", "true:key"]);
}

// ---------------------------------------------------------------- 8. no panic leaves the panel spinning

#[test]
fn a_panicking_worker_body_is_caught_and_reported() {
    assert_eq!(catch_body(|| {}), None);
    assert_eq!(catch_body(|| panic!("boom")).as_deref(), Some("boom"));
    assert_eq!(catch_body(|| panic!("{}", String::from("formatted"))).as_deref(), Some("formatted"));
    assert_eq!(catch_body(|| std::panic::panic_any(5u8)).as_deref(), Some("unknown error"));
    // every thread of the flow goes through spawn_guarded
    let src = include_str!("../uninstall.rs");
    let n = src.lines().filter(|l| l.contains("std::thread::spawn(move ||") && !l.trim_start().starts_with("//")).count();
    assert_eq!(n, 2, "only the output reader of ps() and spawn_guarded itself spawn a bare thread");
}

// ---------------------------------------------------------------- 9. backups: complete or nothing

#[test]
fn a_backup_that_hits_a_cap_fails_instead_of_being_partial() {
    let pid = std::process::id();
    let base = format!(r"Software\PillFixDump{pid}");
    let _g = HkcuKey(base.clone());
    reg_add(&format!(r"{base}\a\b\c\d\e\f"), &["/v", "Deep", "/d", "x"]);
    for i in 0..5 {
        reg_add(&format!(r"{base}\wide\k{i}"), &["/v", "V", "/d", "y"]);
    }
    let item = [(Hive::Hkcu, base.clone(), None)];
    assert!(reg_dump(&item).is_ok());
    let shallow = DumpLimits { depth: 4, ..DumpLimits::default() };
    assert!(reg_dump_with(&item, &shallow).unwrap_err().contains("deeper than 4"));
    let narrow = DumpLimits { subkeys: 3, ..DumpLimits::default() };
    assert!(reg_dump_with(&[(Hive::Hkcu, format!(r"{base}\wide"), None)], &narrow).unwrap_err().contains("more than 3 subkeys"));
    let small = DumpLimits { budget: 2, ..DumpLimits::default() };
    assert!(reg_dump_with(&item, &small).is_err());
    // a 20-level key is fine with the real limits (the old fixed depth of 16 silently dropped everything below it)
    let deep = (0..20).map(|i| format!("L{i}")).collect::<Vec<_>>().join(r"\");
    reg_add(&format!(r"{base}\{deep}"), &["/v", "Bottom", "/d", "z"]);
    let txt = reg_dump(&item).unwrap();
    assert!(txt.contains("\"Bottom\"=\"z\""), "the deepest value is in the backup");
    // a key that does not exist, and a key name that cannot be written
    assert!(reg_dump(&[(Hive::Hkcu, format!(r"{base}\nope"), None)]).is_err());
    assert!(reg_dump(&[(Hive::Hkcu, format!(r"{base}\bad[1]"), None)]).is_err());
    assert!(verify_dump("Windows Registry Editor Version 5.00\r\n\r\n", &item).is_err());
    assert!(verify_dump("nothing", &item).is_err());
}

#[test]
fn line_breaks_quotes_and_backslashes_survive_a_backup_and_restore() {
    let t = Tmp::new();
    let pid = std::process::id();
    let sub = format!(r"Software\PillFixEsc{pid}");
    let _g = HkcuKey(sub.clone());
    // build the value through regedit's own importer: "a\r\n\"b\" \\ c" as exact hex(1) bytes (with the terminating NUL)
    let text = "a\r\n\"b\" \\ c";
    let bytes: Vec<String> = text.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).map(|b| format!("{b:02x}")).collect();
    let src = format!("Windows Registry Editor Version 5.00\r\n\r\n[HKEY_CURRENT_USER\\{sub}]\r\n\"Multi\"=hex(1):{}\r\n\"Plain\"=\"x\\\\y\"\r\n", bytes.join(","));
    let f = t.0.join("seed.reg");
    std::fs::write(&f, reg_bytes(&src)).unwrap();
    let imp = std::process::Command::new(system_exe("reg.exe")).arg("import").arg(&f).creation_flags(NO_WINDOW).output().unwrap();
    assert!(imp.status.success(), "{}", String::from_utf8_lossy(&imp.stderr));
    let before = Key::open(HKEY_CURRENT_USER, &sub, KEY_READ).unwrap().raw_values().unwrap();
    assert_eq!(before.len(), 2);
    let dump = reg_dump(&[(Hive::Hkcu, sub.clone(), None)]).unwrap();
    assert!(dump.contains("hex(1):") && !dump.contains("a\r\nb"), "the line break is written as hex, never as a raw newline: {dump}");
    assert!(dump.contains(r#""Plain"="x\\y""#));
    // delete, import the backup, read again: identical bytes
    assert_eq!(unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(sub.encode_utf16().chain(Some(0)).collect::<Vec<u16>>().as_ptr())) }, OK);
    let back = t.0.join("back.reg");
    std::fs::write(&back, reg_bytes(&dump)).unwrap();
    let imp = std::process::Command::new(system_exe("reg.exe")).arg("import").arg(&back).creation_flags(NO_WINDOW).output().unwrap();
    assert!(imp.status.success(), "{}", String::from_utf8_lossy(&imp.stderr));
    let mut after = Key::open(HKEY_CURRENT_USER, &sub, KEY_READ).unwrap().raw_values().unwrap();
    let mut before = before;
    before.sort();
    after.sort();
    assert_eq!(before, after);
    // names that cannot be written are refused, never mangled
    assert!(reg_line("a\nb", 1, &[0, 0]).is_err());
    assert!(reg_line("ok", 1, &[b'a', 0, 0, 0]).unwrap().contains("\"ok\"=\"a\""));
    assert!(reg_line("ok", 1, &[b'a', 0, b'\n', 0, 0, 0]).unwrap().contains("hex(1):"));
}

#[test]
fn backups_are_tracked_per_item_and_read_back() {
    let t = Tmp::new();
    let e = denv(&t);
    let pid = std::process::id();
    let name = format!("PillFixItem{pid}");
    let publisher = format!("PillFixPub{pid}");
    let good = format!(r"Software\{name}");
    let bad = format!(r"Software\{publisher}\{name}");
    let _g1 = HkcuKey(good.clone());
    let _g2 = HkcuKey(format!(r"Software\{publisher}"));
    reg_add(&good, &["/v", "V", "/d", "1"]);
    reg_add(&format!(r"{bad}\sub[x]"), &["/v", "V", "/d", "1"]); // a key name a .reg file cannot carry: this item has no backup
    let p = prog(&name, &publisher, "");
    let a = Cand::new(What::Key(Hive::Hkcu, good.clone()), Tier::Certain, "x", 0);
    let b = Cand::new(What::Key(Hive::Hkcu, bad.clone()), Tier::Certain, "x", 0);
    let bk = t.dir("bk");
    let r = delete_cands(&p, &[&a, &b], &e, &RealReg, "8", false, &|n, by| file_store(&bk, "i", n, by), &|_, _, _, _, _| {});
    assert_eq!(r.done.len(), 1, "{:?}", r.failed);
    assert!(r.done[0].1.ends_with(&name) && r.failed.len() == 1 && r.failed[0].1.contains("no backup of this item"), "{:?} {:?}", r.done, r.failed);
    assert!(!hkcu_exists(&good) && hkcu_exists(&bad), "only the backed-up item was deleted");
    let reg = std::fs::read(r.backup.unwrap()).unwrap();
    assert!(String::from_utf16_lossy(&reg.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>()).contains(&name));
    // a store that reports success but wrote something else is an error
    assert!(file_store(&bk, "z", "reg", b"").unwrap_err().contains("empty"));
    let p2 = file_store(&bk, "z", "reg", b"abc").unwrap();
    assert_eq!(std::fs::read(p2).unwrap(), b"abc");
}
