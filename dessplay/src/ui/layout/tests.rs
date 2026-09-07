#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use proptest::prelude::*;
use tuirealm::ratatui::{Terminal, backend::TestBackend, layout::Rect};

#[test]
fn defaults_are_valid_and_transferable() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<LayoutBundle>();
    assert!(LayoutBundle::builtin().is_ok());
}
#[test]
fn export_never_overwrites_and_missing_files_fall_back() {
    let dir = tempfile::tempdir().unwrap();
    LayoutBundle::init(dir.path()).unwrap();
    std::fs::write(dir.path().join("style.css"), "#form { padding: 1ch; }").unwrap();
    LayoutBundle::init(dir.path()).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("style.css")).unwrap(),
        "#form { padding: 1ch; }"
    );
    let empty = tempfile::tempdir().unwrap();
    assert_eq!(
        LayoutBundle::load(empty.path()).unwrap().revision,
        LayoutBundle::builtin().unwrap().revision
    );
}
#[test]
fn rejects_incomplete_unknown_and_mistyped_sources() {
    for source in [
        "<templates version=\"2\"></templates>",
        "<templates version=\"1\"><template name=\"form\"><column>",
        "<templates version=\"1\"><template name=\"form\"><script/></template></templates>",
        "<templates version=\"1\"><template name=\"form\"><text bind=\"editing\"/></template></templates>",
        "<templates version=\"1\"><template name=\"form\"><column><slot name=\"body\"/><slot name=\"body\"/></column></template></templates>",
        "<templates version=\"1\"><template name=\"form\"><column><box id=\"a\"/><box id=\"a\"/></column></template></templates>",
    ] {
        let dir = tempfile::tempdir().unwrap();
        LayoutBundle::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("templates/form.xml"), source).unwrap();
        assert!(LayoutBundle::load(dir.path()).is_err(), "{source}");
    }
    for css in [
        "#form { width: 4px; }",
        "#form { position: absolute; }",
        "#form { width: 1ch;",
        "#form:hover { color: red; }",
        "#form { color: var(--missing); }",
        "#form { --a: var(--b); --b: var(--a); color: var(--a); }",
        "#form { color: url(https://example.com); }",
    ] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("style.css"), css).unwrap();
        let error = LayoutBundle::load(dir.path()).unwrap_err();
        assert!(error.line > 0 && error.column > 0, "{error}");
    }
}
#[test]
fn cascade_specificity_inheritance_and_custom_variables() {
    let dir = tempfile::tempdir().unwrap();
    LayoutBundle::init(dir.path()).unwrap();
    std::fs::write(dir.path().join("style.css"), "#form { --accent: #123456; color: var(--accent); } column > scroll > slot { color: blue; } #form slot { color: red; } slot { color: green; }").unwrap();
    let bundle = LayoutBundle::load(dir.path()).unwrap();
    let scene = Renderer::new(bundle)
        .arrange(
            "form",
            Rect::new(0, 0, 30, 15),
            &Presentation::default().slot("body", 0, 0),
        )
        .unwrap();
    assert_eq!(
        scene.style("body").fg,
        Some(tuirealm::ratatui::style::Color::Red)
    );
}
proptest! {
    #[test]
    fn geometry_is_translation_invariant_and_contained(width in 0u16..150, height in 0u16..80, x in 0u16..200, y in 0u16..100) {
        let mut renderer = Renderer::new(LayoutBundle::builtin().unwrap());
        let data = Presentation::default().text("title","Form").slot("header",0,3).slot("body",0,0).slot("notes",0,1).slot("save",0,1);
        let a = renderer.arrange("form",Rect::new(0,0,width,height),&data).unwrap();
        let area = Rect::new(x,y,width,height);
        let b = renderer.arrange("form",area,&data).unwrap();
        for name in ["header","body","notes","save"] {
            let one = a.slot(name); let two = b.slot(name);
            prop_assert_eq!(one.width,two.width); prop_assert_eq!(one.height,two.height);
            if !two.is_empty() { prop_assert_eq!(two.x,one.x+x); prop_assert_eq!(two.y,one.y+y); prop_assert_eq!(two.intersection(area),two); }
        }
    }
    #[test]
    fn text_fragment_ranges_are_valid(text in ".{0,100}", width in 0usize..80) {
        for fragment in measure_text(&text,width,0,2,true,false) {
            prop_assert!(fragment.source.end <= text.chars().count());
            prop_assert!(fragment.width + fragment.indent <= width);
        }
    }
}
#[test]
fn rendering_uses_measured_bounds_at_nonzero_origin() {
    let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
    let mut renderer = Renderer::new(LayoutBundle::builtin().unwrap());
    let scene = renderer
        .arrange(
            "form",
            Rect::new(7, 3, 30, 12),
            &Presentation::default()
                .text("title", "Settings")
                .slot("body", 0, 0)
                .slot("save", 0, 1),
        )
        .unwrap();
    terminal.draw(|f| scene.paint(f)).unwrap();
    assert_eq!(terminal.backend().buffer()[(7, 3)].symbol(), "┌");
    assert_eq!(scene.slot("save"), Rect::new(8, 13, 28, 1));
}

#[test]
fn helpers_validate_in_the_callers_schema_and_cycles_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("templates")).unwrap();
    let file = dir.path().join("templates/custom.xml");
    std::fs::write(&file, r#"<templates version="1"><template name="form"><use template="section"/></template><template name="section"><column><slot name="body"/></column></template></templates>"#).unwrap();
    assert!(LayoutBundle::load(dir.path()).is_ok());
    std::fs::write(&file, r#"<templates version="1"><template name="form"><use template="section"/></template><template name="section"><use template="form"/></template></templates>"#).unwrap();
    assert!(
        LayoutBundle::load(dir.path())
            .unwrap_err()
            .message
            .contains("cycle")
    );
}

#[test]
fn rejects_incomplete_comments_functions_and_invalid_ancestor_state_variables() {
    let dir = tempfile::tempdir().unwrap();
    for css in [
        "/* an incomplete editor save",
        "#form { --unused: var(--x; }",
        "#form { --size: 1ch; } #form:focus { --size: red; } #form slot { width: var(--size); }",
    ] {
        std::fs::write(dir.path().join("style.css"), css).unwrap();
        assert!(LayoutBundle::load(dir.path()).is_err(), "{css}");
    }
}

#[test]
fn huge_authored_box_paints_only_its_original_clipped_edges() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("templates")).unwrap();
    std::fs::write(dir.path().join("templates/form.xml"), r#"<templates version="1"><template name="form"><column><box style="width: 65535ch; height: 65535lh; border: 1"/></column></template></templates>"#).unwrap();
    let mut renderer = Renderer::new(LayoutBundle::load(dir.path()).unwrap());
    let scene = renderer
        .arrange("form", Rect::new(0, 0, 10, 5), &Presentation::default())
        .unwrap();
    let mut terminal = Terminal::new(TestBackend::new(10, 5)).unwrap();
    terminal.draw(|f| scene.paint(f)).unwrap();
    assert_eq!(terminal.backend().buffer()[(0, 0)].symbol(), "┌");
    assert_eq!(terminal.backend().buffer()[(9, 4)].symbol(), " ");
}

#[test]
fn measured_rows_are_reused_until_the_content_or_width_changes() {
    let mut renderer = Renderer::new(LayoutBundle::builtin().unwrap());
    let first = renderer.measured_text("a long line with Unicode 日本語", 12);
    let same = renderer.measured_text("a long line with Unicode 日本語", 12);
    let resized = renderer.measured_text("a long line with Unicode 日本語", 13);
    assert!(std::sync::Arc::ptr_eq(&first, &same));
    assert!(!std::sync::Arc::ptr_eq(&first, &resized));
}
