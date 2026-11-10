//! `capture.annotate` and the text tool's placement (helpers come from the parent test module).
use super::*;

#[test]
fn text_box_stays_inside_the_selection() {
    let (mut m, mut io) = area();
    drag(&mut m, &mut io, (100.0, 100.0), (500.0, 400.0));
    click_item(&mut m, &mut io, "text");
    // near the bottom-right corner the box is pulled in: one 20 px line and 40 px of letters stay inside
    down(&mut m, &mut io, 480.0, 385.0);
    up(&mut m, &mut io, 480.0, 385.0);
    assert_eq!(m.view(&EN).text_edit.map(|t| t.pos), Some(pt(460.0, 380.0)));
    m.on_input(Input::Text("Hi".into()), &mut io);
    key(&mut m, &mut io, Key::Enter, Mods::NONE);
    // the op sits exactly where the box was (frame px == css px on this 1:1 rig): what the editor shows is what the export draws
    assert_eq!(m.ops()[0].kind, OpKind::Text { s: "Hi".into(), a: pt(460.0, 380.0) });
}

#[test]
fn annotate_off_leaves_only_the_commit_buttons() {
    let off = Toolbar::with_annotate(true, false, false);
    let ids: Vec<String> = off.items().filter(|i| **i != ItemId::Sep).map(|i| i.id()).collect();
    assert_eq!(ids, ["copy", "save", "record", "both", "cancel"]);
    assert!(Toolbar::with_annotate(true, false, true).items().any(|i| *i == ItemId::Tool(Tool::Highlighter)));
    assert_eq!(Toolbar::new(false, false), Toolbar::with_annotate(false, false, true), "the old constructor is the annotating bar");
    let (mut m, mut io) = rig(1920.0, 1080.0, 1920, 1080, "area", Options { annotate: false, extended_tools: true, ..Options::default() });
    drag(&mut m, &mut io, (100.0, 100.0), (500.0, 400.0));
    m.apply(Action::SetTool(Tool::Pen), &mut io);
    assert_eq!(m.view(&EN).tool, Tool::Select, "no drawing with annotate off");
    drag(&mut m, &mut io, (150.0, 150.0), (250.0, 250.0));
    assert!(m.ops().is_empty());
}

#[test]
fn annotate_on_offers_every_markup_tool() {
    let (mut m, mut io) = rig(1920.0, 1080.0, 1920, 1080, "area", Options { extended_tools: true, ..Options::default() });
    drag(&mut m, &mut io, (100.0, 100.0), (500.0, 400.0));
    let bar = m.view(&EN).toolbar.expect("the bar is up");
    let ids: Vec<&str> = bar.items.iter().map(|i| i.id.as_str()).collect();
    for want in ["rect", "arrow", "pen", "highlighter", "text", "blur", "undo"] {
        assert!(ids.contains(&want), "{want}");
    }
    // a highlighter stroke lands as an op and is undone by undo
    click_item(&mut m, &mut io, "highlighter");
    drag(&mut m, &mut io, (150.0, 150.0), (300.0, 160.0));
    assert!(matches!(m.ops()[0].kind, OpKind::Highlighter { .. }));
    m.apply(Action::Undo, &mut io);
    assert!(m.ops().is_empty());
}
