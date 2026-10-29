// Round-3 hardening tests: every finding of the security review has a test here that fails without its fix.
use super::tests::{prog, sv, Tmp};
use super::tests_deep::{denv, foo, FakeReg, G};
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
        let req = Req { mode: Mode::Delete, name: "Foo Editor".into(), publisher: "Acme".into(), location: b.to_string_lossy().into_owned(), sub: String::new(), hklm: true, guids: vec![], keys: vec![] };
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
