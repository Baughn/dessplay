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
fn keyed_repetition_uses_item_bindings_states_and_stable_geometry_identities() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("templates")).unwrap();
    std::fs::write(directory.path().join("templates/form.xml"), r#"<templates version="1"><template name="form"><column><repeat bind="tabs" style="flex-direction: row; gap: 2ch"><row id="tab"><text bind="label"/><text bind="missing" if="invalid"/></row></repeat></column></template></templates>"#).unwrap();
    std::fs::write(
        directory.path().join("style.css"),
        "#tab:selected { color: red; }",
    )
    .unwrap();
    let item = |key: &str, text: &str, selected: bool| PresentedItem {
        key: key.into(),
        data: Presentation::default()
            .text("label", text)
            .selected(selected),
    };
    let mut renderer = Renderer::new(LayoutBundle::load(directory.path()).unwrap());
    let area = Rect::new(3, 2, 30, 6);
    let first = renderer
        .arrange(
            "form",
            area,
            &Presentation::default().list(
                "tabs",
                vec![item("a", "Alpha", false), item("b", "Beta", true)],
            ),
        )
        .unwrap();
    let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
    terminal.draw(|frame| first.paint(frame)).unwrap();
    assert_eq!(first.items.len(), 2);
    assert_eq!(first.items[1].bounds.x, 10);
    assert_eq!(terminal.backend().buffer()[(10, 2)].symbol(), "B");
    assert_eq!(
        terminal.backend().buffer()[(10, 2)].fg,
        tuirealm::ratatui::style::Color::Red
    );
    let next = renderer
        .arrange(
            "form",
            area,
            &Presentation::default().list(
                "tabs",
                vec![item("b", "Beta", true), item("a", "Alpha", false)],
            ),
        )
        .unwrap();
    assert_eq!(next.items[0].instance, first.items[1].instance);
    assert_eq!(next.items[0].bounds.x, 3);
    let duplicate = Presentation::default().list(
        "tabs",
        vec![item("a", "Alpha", false), item("a", "Beta", true)],
    );
    assert!(
        renderer
            .arrange("form", area, &duplicate)
            .unwrap_err()
            .message
            .contains("unique keys")
    );
    let empty_key = Presentation::default().list("tabs", vec![item("", "Alpha", false)]);
    assert!(renderer.arrange("form", area, &empty_key).is_err());
}

#[test]
fn repeated_items_validate_their_own_contract() {
    for content in [
        "<repeat bind=\"title\"><text bind=\"label\"/></repeat>",
        "<repeat bind=\"tabs\"><text bind=\"title\"/></repeat>",
        "<repeat bind=\"tabs\"><text bind=\"label\"/><text bind=\"missing\"/></repeat>",
        "<repeat bind=\"tabs\"><slot name=\"editor\"/></repeat>",
    ] {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("templates")).unwrap();
        std::fs::write(
            directory.path().join("templates/form.xml"),
            format!(
                "<templates version=\"1\"><template name=\"form\">{content}</template></templates>"
            ),
        )
        .unwrap();
        assert!(LayoutBundle::load(directory.path()).is_err(), "{content}");
    }
}

#[test]
fn natural_row_margins_count_in_collection_and_document_spacing() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("templates")).unwrap();
    std::fs::write(directory.path().join("templates/row.xml"), r#"<templates version="1"><template name="playlist-row"><text bind="title" style="margin: 1lh 0"/></template></templates>"#).unwrap();
    let rows: Vec<_> = ["Alpha", "Beta"]
        .into_iter()
        .map(|label| PresentedRow {
            key: label.into(),
            data: Presentation::default().text("title", label),
            gap_after: false,
        })
        .collect();
    for document in [false, true] {
        let mut renderer = Renderer::new(LayoutBundle::load(directory.path()).unwrap());
        let mut terminal = Terminal::new(TestBackend::new(30, 15)).unwrap();
        let area = Rect::new(2, 3, 20, 10);
        let mut hits = RenderedCollection::default();
        terminal
            .draw(|frame| {
                if document {
                    renderer
                        .paint_document(
                            frame,
                            area,
                            "playlist-row",
                            &rows,
                            &mut DocumentScroll::default(),
                            Default::default(),
                        )
                        .unwrap();
                } else {
                    hits = renderer
                        .paint_collection(
                            frame,
                            area,
                            "playlist-row",
                            &rows,
                            None,
                            None,
                            Default::default(),
                        )
                        .unwrap();
                }
            })
            .unwrap();
        assert_eq!(terminal.backend().buffer()[(2, 4)].symbol(), "A");
        assert_eq!(terminal.backend().buffer()[(2, 7)].symbol(), "B");
        if !document {
            assert_eq!(hits.hit(2, 4), Some(0));
            assert_eq!(hits.hit(2, 7), Some(1));
            assert_eq!(hits.hit(2, 5), None);
        }
    }
}
#[test]
fn component_root_margins_reduce_wrapping_width_and_explicit_sizes_are_honored() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("templates")).unwrap();
    let path = directory.path().join("templates/row.xml");
    std::fs::write(&path, r#"<templates version="1"><template name="playlist-row"><text id="cell" bind="title" style="margin: 1lh 2ch; white-space: normal"/></template></templates>"#).unwrap();
    let mut renderer = Renderer::new(LayoutBundle::load(directory.path()).unwrap());
    let data = Presentation::default().text("title", "alpha beta");
    let scene = renderer
        .measure_content("playlist-row", "row", 10, &data)
        .unwrap();
    assert_eq!(scene.bounds("cell").width, 6);
    assert_eq!(scene.height(), 4);
    std::fs::write(&path, r#"<templates version="1"><template name="playlist-row"><text id="cell" bind="title" style="width: 4ch; height: 2lh"/></template></templates>"#).unwrap();
    renderer.install(LayoutBundle::load(directory.path()).unwrap());
    let scene = renderer
        .arrange("playlist-row", Rect::new(3, 2, 20, 10), &data)
        .unwrap();
    assert_eq!(scene.bounds("cell"), Rect::new(3, 2, 4, 2));
}
#[test]
fn nested_overlay_slots_paint_after_their_content_and_before_later_overlays() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("templates")).unwrap();
    std::fs::write(directory.path().join("templates/form.xml"), r#"<templates version="1"><template name="form"><box><overlay placement="center" style="width: 20ch; height: 8lh"><overlay style="height: 1lh"><slot name="error"/></overlay><slot name="body" style="flex-grow: 1"/></overlay><overlay placement="center" style="width: 2ch; height: 2lh"><slot name="editor" style="flex-grow: 1"/></overlay></box></template></templates>"#).unwrap();
    let scene = Renderer::new(LayoutBundle::load(directory.path()).unwrap())
        .arrange(
            "form",
            Rect::new(0, 0, 30, 12),
            &Presentation::default().slot("error", 0, 1),
        )
        .unwrap();
    let mut terminal = Terminal::new(TestBackend::new(30, 12)).unwrap();
    let mut order = Vec::new();
    terminal
        .draw(|frame| {
            scene.paint_with_slots(frame, |name, frame, area, _| {
                order.push(name.to_string());
                for y in area.y..area.bottom() {
                    for x in area.x..area.right() {
                        frame.buffer_mut()[(x, y)].set_symbol(match name {
                            "body" => "B",
                            "error" => "E",
                            _ => "I",
                        });
                    }
                }
            })
        })
        .unwrap();
    assert_eq!(order, ["body", "error", "editor"]);
    let error = scene.slot("error");
    assert_eq!(
        terminal.backend().buffer()[(error.x, error.y)].symbol(),
        "E"
    );
    let editor = scene.slot("editor");
    assert_eq!(
        terminal.backend().buffer()[(editor.x, editor.y)].symbol(),
        "I"
    );
}
#[test]
fn rejects_ambiguous_prefixes_and_repeated_action_bindings() {
    for content in [
        "<prefix><text bind=\"timestamp\"/></prefix>",
        "<flow><rich bind=\"body\"/><prefix><text bind=\"timestamp\"/></prefix></flow>",
        "<flow><flow><prefix><text bind=\"timestamp\"/></prefix></flow></flow>",
        "<column><rich bind=\"body\"/><rich bind=\"body\"/></column>",
    ] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("templates")).unwrap();
        std::fs::write(dir.path().join("templates/message.xml"), format!("<templates version=\"1\"><template name=\"chat-message\">{content}</template></templates>")).unwrap();
        let error = LayoutBundle::load(dir.path()).unwrap_err();
        assert!(error.line > 0 && error.column > 0, "{error}");
        assert!(
            error.message.contains("prefix") || error.message.contains("rich bindings"),
            "{error}"
        );
    }
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
    fn repeated_unicode_text_keeps_item_source_mappings(width in 10u16..60, x in 0u16..20, y in 0u16..10, labels in prop::collection::vec("[a-z界😀 ]{1,20}", 1..8)) {
        use unicode_width::UnicodeWidthStr;
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("templates")).unwrap();
        std::fs::write(directory.path().join("templates/form.xml"), r#"<templates version="1"><template name="form"><repeat bind="tabs"><text bind="label" style="width: 100%; white-space: normal"/></repeat></template></templates>"#).unwrap();
        let items = labels.iter().enumerate().map(|(index, label)| PresentedItem { key: index.to_string(), data: Presentation::default().text("label", label) }).collect();
        let scene = Renderer::new(LayoutBundle::load(directory.path()).unwrap()).arrange("form", Rect::new(x,y,width,80), &Presentation::default().list("tabs", items)).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(x + width, y + 80)).unwrap();
        terminal.draw(|frame| scene.paint(frame)).unwrap();
        for item in &scene.items {
            let label = &labels[item.key.parse::<usize>().unwrap()];
            for region in scene.text_regions.iter().filter(|region| region.node.starts_with(&item.instance)) {
                let mut column = region.bounds.x;
                for ch in label.chars().skip(region.source.start).take(region.source.len()) {
                    let expected = ch.to_string();
                    prop_assert_eq!(terminal.backend().buffer()[(column, region.bounds.y)].symbol(), expected.as_str());
                    column += expected.width() as u16;
                }
            }
        }
    }
    #[test]
    fn modal_placement_preserves_browser_sizing_and_translation(width in 0u16..180, height in 0u16..90, x in 0u16..30, y in 0u16..20) {
        let area = Rect::new(x,y,width,height);
        let scene = Renderer::new(LayoutBundle::builtin().unwrap()).arrange("episode-browser", area, &Presentation::default()).unwrap();
        let actual = scene.bounds("episode-browser-frame");
        let expected = crate::ui::widgets::overlay(area,70,70);
        if expected.is_empty() {prop_assert!(actual.is_empty());} else {prop_assert_eq!(actual, expected);}
    }
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

#[test]
fn drag_sizes_are_local_revision_scoped_and_legacy_import_happens_once() {
    let storage = crate::storage::Storage::open_in_memory().unwrap();
    let builtin = LayoutBundle::builtin().unwrap();
    let mut sizes = LayoutSettings::default();
    let legacy = crate::config::PaneLayout {
        chat_width: 65,
        ..Default::default()
    };
    assert!(sizes.activate(&builtin, legacy));
    assert_eq!(sizes.shares("builtin")["chat-column"], 6500);
    sizes.save(&storage).unwrap();
    let mut restored = LayoutSettings::load(&storage).unwrap();
    assert_eq!(restored, sizes);
    assert!(!restored.activate(&builtin, Default::default()));
    let mut changed = builtin.clone();
    changed.revision.push('x');
    assert!(restored.activate(&changed, legacy));
    assert!(restored.shares("builtin").is_empty());
    restored.save(&storage).unwrap();
    let mut restored = LayoutSettings::load(&storage).unwrap();
    assert!(!restored.activate(&changed, legacy));
    assert!(
        restored.shares("builtin").is_empty(),
        "legacy sizes never reappear"
    );
    let dir = tempfile::tempdir().unwrap();
    let custom = LayoutBundle::load(dir.path()).unwrap();
    restored.activate(&custom, legacy);
    let source = LayoutSettings::source(&custom);
    restored.set(
        &source,
        "panes",
        [("chat-column".into(), 7000), ("right-column".into(), 3000)].into(),
    );
    restored.save(&storage).unwrap();
    storage
        .save_settings(&crate::config::Settings::default())
        .unwrap();
    assert_eq!(
        LayoutSettings::load(&storage).unwrap(),
        restored,
        "ordinary settings saves cannot overwrite drag sizes"
    );
    std::fs::write(
        dir.path().join("style.css"),
        "#chat-column { flex-basis: 40%; }",
    )
    .unwrap();
    assert!(restored.activate(&LayoutBundle::load(dir.path()).unwrap(), legacy));
    assert!(
        restored.shares(&source).is_empty(),
        "file changes win across restart"
    );
}

#[test]
fn resizable_containers_require_stable_children_and_flex_layout() {
    for content in [
        r#"<row resizable="true"><slot name="chat"/><slot name="users"/></row>"#,
        r#"<row id="s" resizable="true"><slot name="chat"/><slot id="u" name="users"/></row>"#,
        r#"<row id="s" resizable="false"><slot id="c" name="chat"/><slot id="u" name="users"/></row>"#,
        r#"<row id="s" resizable="true" style="display: grid"><slot id="c" name="chat"/><slot id="u" name="users"/></row>"#,
    ] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("templates")).unwrap();
        std::fs::write(
            dir.path().join("templates/app.xml"),
            format!(
                "<templates version=\"1\"><template name=\"app\">{content}</template></templates>"
            ),
        )
        .unwrap();
        assert!(LayoutBundle::load(dir.path()).is_err(), "{content}");
    }
}

proptest! {
    #[test]
    fn split_handles_translate_with_paint_and_trade_only_adjacent_children(width in 30u16..150, height in 20u16..80, x in 0u16..100, y in 0u16..100, pointer in 0u16..200) {
        use tuirealm::ratatui::layout::Position;
        let mut renderer = Renderer::new(LayoutBundle::builtin().unwrap());
        let data = Presentation::default().boolean("separate-subtitles", true);
        let a = renderer.arrange("app", Rect::new(0,0,width,height), &data).unwrap();
        let b = renderer.arrange("app", Rect::new(x,y,width,height), &data).unwrap();
        prop_assert_eq!(a.splits.len(), b.splits.len());
        for (a,b) in a.splits.iter().zip(b.splits.iter()) {
            prop_assert_eq!(&a.id,&b.id);
            prop_assert_eq!(b.handle.x,a.handle.x+x);
            prop_assert_eq!(b.handle.y,a.handle.y+y);
            prop_assert_eq!(b.handle.width,a.handle.width);
            prop_assert_eq!(b.handle.height,a.handle.height);
            let one = a.drag(Position::new(pointer,pointer));
            let two = b.drag(Position::new(pointer+x,pointer+y));
            prop_assert_eq!(&one,&two);
            prop_assert_eq!(one.values().map(|n| u32::from(*n)).sum::<u32>(),10_000);
            let initial = a.drag(Position::new(a.handle.x+1,a.handle.y+1));
            prop_assert!(one.iter().filter(|(id,n)| initial[*id] != **n).count() <= 2);
        }
    }
}

proptest! {
    #[test]
    fn wrapped_collection_paint_and_hits_agree_at_arbitrary_origins(width in 2u16..65, height in 1u16..25, x in 0u16..8, y in 0u16..8, center in 0usize..20) {
        use tuirealm::ratatui::style::{Color, Style};
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("templates")).unwrap();
        std::fs::write(dir.path().join("templates/form-row.xml"), r#"<templates version="1"><template name="form-row"><column style="padding: 1ch"><text bind="value" style="white-space: normal"/></column></template></templates>"#).unwrap();
        let mut renderer = Renderer::new(LayoutBundle::load(dir.path()).unwrap());
        let rows = (0..20).map(|i| PresentedRow {
            key: i.to_string(),
            data: Presentation::default().text("value", "界 words and more words ".repeat(i % 3 + 1)).style("value", Style::default().bg(Color::Indexed(i as u8 + 1))),
            gap_after: false,
        }).collect::<Vec<_>>();
        let area = Rect::new(x,y,width,height);
        let mut terminal = Terminal::new(TestBackend::new(x+width+2,y+height+2)).unwrap();
        let mut interactions = RenderedCollection::default();
        let frame = terminal.draw(|frame| {
            interactions = renderer.paint_collection(frame,area,"form-row",&rows,Some(center),Some(center),Style::default()).unwrap();
        }).unwrap();
        for yy in 0..frame.area.height {
            for xx in 0..frame.area.width {
                if let Color::Indexed(index) = frame.buffer[(xx,yy)].bg {
                    prop_assert_eq!(interactions.hit(xx,yy),Some(usize::from(index-1)));
                    prop_assert!(area.contains(tuirealm::ratatui::layout::Position::new(xx,yy)));
                }
                if !area.contains(tuirealm::ratatui::layout::Position::new(xx,yy)) {
                    prop_assert!(interactions.hit(xx,yy).is_none());
                }
            }
        }
    }
}

#[test]
fn semantic_color_variables_are_typed_and_authored_values_take_precedence() {
    use tuirealm::ratatui::style::Color;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("style.css"),
        "#form { color: var(--accent); }",
    )
    .unwrap();
    let data = Presentation::default().color_variable("--accent", Color::LightGreen);
    let scene = Renderer::new(LayoutBundle::load(dir.path()).unwrap())
        .arrange("form", Rect::new(0, 0, 40, 20), &data)
        .unwrap();
    assert_eq!(scene.style("body").fg, Some(Color::LightGreen));
    std::fs::write(
        dir.path().join("style.css"),
        "#form { --accent: blue; color: var(--accent); }",
    )
    .unwrap();
    let scene = Renderer::new(LayoutBundle::load(dir.path()).unwrap())
        .arrange("form", Rect::new(0, 0, 40, 20), &data)
        .unwrap();
    assert_eq!(scene.style("body").fg, Some(Color::Blue));
    std::fs::write(
        dir.path().join("style.css"),
        "#rogue-frame { width: var(--dungeon-border); }",
    )
    .unwrap();
    assert!(
        LayoutBundle::load(dir.path()).is_err(),
        "semantic color variables cannot supply dimensions"
    );
}

#[test]
fn inline_flow_wraps_semantic_children_and_rejects_ignored_box_rules() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("templates")).unwrap();
    std::fs::write(dir.path().join("templates/form-row.xml"), r#"<templates version="1"><template name="form-row"><flow style="white-space: normal"><text id="label" bind="label" style="color: red"/><text id="value" bind="value" style="color: green"/></flow></template></templates>"#).unwrap();
    let data = Presentation::default()
        .text("label", "Blood")
        .text("value", "100 道 next");
    let mut renderer = Renderer::new(LayoutBundle::load(dir.path()).unwrap());
    let scene = renderer
        .arrange("form-row", Rect::new(2, 3, 10, 4), &data)
        .unwrap();
    let mut terminal = Terminal::new(TestBackend::new(20, 10)).unwrap();
    terminal.draw(|frame| scene.paint(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(2, 3)].symbol(), "B");
    assert_eq!(buffer[(8, 3)].symbol(), "1");
    assert_eq!(buffer[(2, 4)].symbol(), "道");
    assert_eq!(buffer[(8, 3)].fg, tuirealm::ratatui::style::Color::Green);
    let value = scene
        .text_regions
        .iter()
        .find(|region| region.bounds.y == 4)
        .unwrap();
    assert_eq!(value.binding, "value");
    assert_eq!(value.source, 4..10);
    std::fs::write(
        dir.path().join("style.css"),
        "flow > text { padding: 1ch; }",
    )
    .unwrap();
    assert!(
        LayoutBundle::load(dir.path())
            .unwrap_err()
            .message
            .contains("inline flow")
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(64)))]
    #[test]
    fn rich_text_paint_and_source_regions_agree(width in 1u16..60, height in 1u16..10, x in 0u16..20, y in 0u16..10) {
        use tuirealm::ratatui::style::{Color, Style};
        use unicode_width::UnicodeWidthStr;
        let prefix = "name 道 ";
        let body = "literal <b>spoiler</b> words 界 repeated words";
        let source = format!("{prefix}{body}");
        let data = Presentation::default().rich("body", vec![
            RichSpan { text: prefix.into(), style: Style::default().fg(Color::Red), action: None, ..Default::default() },
            RichSpan { text: body.into(), style: Style::default().fg(Color::Green), action: Some("spoiler:7".into()), ..Default::default() },
        ]);
        let area = Rect::new(x, y, width, height);
        let scene = Renderer::new(LayoutBundle::builtin().unwrap()).arrange("rich-text", area, &data).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(x + width, y + height)).unwrap();
        terminal.draw(|frame| scene.paint(frame)).unwrap();
        for region in &scene.text_regions {
            let text: String = source.chars().skip(region.source.start).take(region.source.len()).collect();
            prop_assert_eq!(text.width(), usize::from(region.bounds.width));
            prop_assert_eq!(region.bounds.intersection(area), region.bounds);
            let mut cell = region.bounds.x;
            for ch in text.chars() {
                let expected = ch.to_string();
                prop_assert_eq!(terminal.backend().buffer()[(cell, region.bounds.y)].symbol(), expected.as_str());
                cell += expected.width() as u16;
            }
            let action = region.source.start >= prefix.chars().count();
            prop_assert_eq!(region.action.as_deref(), action.then_some("spoiler:7"));
        }
    }
    #[test]
    fn centered_recovery_overlay_has_stable_cell_bounds(width in 0u16..160, height in 0u16..60, x in 0u16..20, y in 0u16..20) {
        let area = Rect::new(x,y,width,height);
        let data = Presentation::default().text("phase", "Preparing care");
        let scene = Renderer::new(LayoutBundle::builtin().unwrap()).arrange("rogue-recovery", area, &data).unwrap();
        let bounds = scene.bounds("rogue-recovery");
        if !bounds.is_empty() {
            prop_assert_eq!(bounds.intersection(area), bounds);
            prop_assert_eq!(bounds.width, width.saturating_sub(2).min(68), "{:?}", scene.inspect());
            prop_assert_eq!(bounds.height, height.saturating_sub(2).min(10));
            prop_assert!((i32::from(bounds.x - x) * 2 - i32::from(width - bounds.width)).abs() <= 1);
            prop_assert!((i32::from(bounds.y - y) * 2 - i32::from(height - bounds.height)).abs() <= 1);
        }
    }
}

#[test]
fn filename_ellipsis_has_no_source_and_retains_the_suffix_identity() {
    let data = Presentation::default()
        .text("value", "/long/path/道/file.mkv")
        .preserve_end("value");
    let scene = Renderer::new(LayoutBundle::builtin().unwrap())
        .arrange("form-row", Rect::new(0, 0, 10, 2), &data)
        .unwrap();
    let region = scene
        .text_regions
        .iter()
        .find(|region| region.binding == "value")
        .unwrap();
    let source: String = "/long/path/道/file.mkv"
        .chars()
        .skip(region.source.start)
        .take(region.source.len())
        .collect();
    assert_eq!(source, "/file.mkv");
    assert_eq!(region.bounds.x, 1);
}

#[test]
fn page_shell_preserves_the_two_thirds_boundary_and_accepts_reordering() {
    let mut renderer = Renderer::new(LayoutBundle::builtin().unwrap());
    for height in 1..100u16 {
        let area = Rect::new(7, 3, 80, height);
        let scene = renderer
            .arrange("page-shell", area, &Presentation::default())
            .unwrap();
        let page = scene.slot("page");
        assert_eq!(page.height, height * 2 / 3, "height={height}");
        if !page.is_empty() {
            assert_eq!(page.y, 3);
        }
        assert!(scene.slot("recent").is_empty() || scene.slot("recent").y >= page.bottom());
    }
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("templates")).unwrap();
    std::fs::write(directory.path().join("templates/page.xml"), r#"<templates version="1"><template name="page-shell"><row><slot name="recent" style="width: 20ch"/><slot name="page" style="flex-grow: 1"/></row></template></templates>"#).unwrap();
    renderer.install(LayoutBundle::load(directory.path()).unwrap());
    let scene = renderer
        .arrange(
            "page-shell",
            Rect::new(7, 3, 80, 30),
            &Presentation::default(),
        )
        .unwrap();
    assert_eq!(scene.slot("recent"), Rect::new(7, 3, 20, 30));
    assert_eq!(scene.slot("page"), Rect::new(27, 3, 60, 30));
}

#[test]
fn authored_color_overrides_a_dimmed_component_before_painting() {
    use crate::ui::theme::ColorDepth;
    use tuirealm::ratatui::style::{Color, Modifier, Style};
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("style.css"),
        "#health-metric { color: #123456; background-color: #456789; }",
    )
    .unwrap();
    let mut renderer = Renderer::new(LayoutBundle::load(directory.path()).unwrap());
    renderer.set_color_depth(ColorDepth::TrueColor);
    let data = Presentation::default()
        .boolean("ordinary", true)
        .text("label", "rtt")
        .text("value", "89ms")
        .component_style(
            "health-metric",
            Style::default().add_modifier(Modifier::DIM),
        );
    let mut terminal = Terminal::new(TestBackend::new(20, 3)).unwrap();
    terminal
        .draw(|frame| {
            renderer.clear(frame, frame.area());
            renderer
                .arrange("health-metric", Rect::new(2, 1, 12, 1), &data)
                .unwrap()
                .paint(frame);
        })
        .unwrap();
    let cell = &terminal.backend().buffer()[(2, 1)];
    assert_eq!(cell.fg, Color::Rgb(0x12, 0x34, 0x56));
    assert_eq!(cell.bg, Color::Rgb(0x45, 0x67, 0x89));
    assert!(!cell.modifier.contains(Modifier::DIM));
    let count = renderer.arrangement_count();
    renderer.set_color_depth(ColorDepth::Limited);
    terminal
        .draw(|frame| {
            renderer
                .arrange("health-metric", Rect::new(2, 1, 12, 1), &data)
                .unwrap()
                .paint(frame);
        })
        .unwrap();
    assert_eq!(
        renderer.arrangement_count(),
        count,
        "palette changes do not remeasure text"
    );
    assert!(matches!(
        terminal.backend().buffer()[(2, 1)].fg,
        Color::Indexed(_)
    ));
}

#[test]
fn progress_fill_clips_original_geometry_and_has_no_selectable_text() {
    let data = Presentation::default()
        .text("open", "[")
        .text("close", "]")
        .progress("progress", 50, 100);
    let mut renderer = Renderer::new(LayoutBundle::builtin().unwrap());
    let scene = renderer
        .measure_content("work-row", "job", 12, &data)
        .unwrap();
    assert!(
        scene
            .text_regions
            .iter()
            .all(|region| region.binding != "progress")
    );
    let mut terminal = Terminal::new(TestBackend::new(20, 5)).unwrap();
    terminal
        .draw(|frame| scene.paint_scrolled(frame, Rect::new(3, 2, 12, 1), 0))
        .unwrap();
    let text: String = (3..15)
        .map(|x| terminal.backend().buffer()[(x, 2)].symbol())
        .collect();
    assert_eq!(text, "[#####     ]");
    let oversized = renderer
        .arrange(
            "work-overlay",
            Rect::new(0, 0, 30000, 30000),
            &Presentation::default().list(
                "jobs",
                (0..3)
                    .map(|i| PresentedItem {
                        key: i.to_string(),
                        data: data.clone().text("filename", "file"),
                    })
                    .collect(),
            ),
        )
        .unwrap();
    assert_eq!(
        oversized.bounds("work-overlay"),
        Rect::new(6000, 14996, 18000, 8)
    );
}

#[test]
fn attached_dropdown_follows_its_reordered_control_and_escapes_header_clip() {
    let dir = tempfile::tempdir().unwrap();
    LayoutBundle::init(dir.path()).unwrap();
    std::fs::write(dir.path().join("style.css"), "#log-header { padding: 0 2ch; } #log-app-anchor { margin: 2lh 0 0 0; } #log-header { max-height: 5lh; }").unwrap();
    let mut renderer = Renderer::new(LayoutBundle::load(dir.path()).unwrap());
    let data = Presentation::default()
        .boolean("choose-app", true)
        .text("app-label", "DessPlay")
        .text("app-level", "trace");
    let scene = renderer
        .arrange("log", Rect::new(5, 7, 40, 20), &data)
        .unwrap();
    let anchor = scene.bounds("log-app-anchor");
    let popup = scene.bounds("log-app-picker");
    assert_eq!(popup.x, anchor.x);
    assert_eq!(popup.y, anchor.bottom());
    assert_eq!(
        popup.height, 9,
        "the header clip must not truncate its attached popup"
    );
    assert_eq!(
        scene.slot("app-options"),
        popup.inner(tuirealm::ratatui::layout::Margin::new(1, 1))
    );
    assert!(scene.has_overlay());
    std::fs::write(
        dir.path().join("style.css"),
        "#log-header { display: none; }",
    )
    .unwrap();
    renderer.install(LayoutBundle::load(dir.path()).unwrap());
    assert!(
        !renderer
            .arrange("log", Rect::new(5, 7, 40, 20), &data)
            .unwrap()
            .has_overlay()
    );
}

#[test]
fn first_line_prefix_shares_the_body_row_and_preserves_independent_sources() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("templates")).unwrap();
    std::fs::write(dir.path().join("templates/form-row.xml"), r#"<templates version="1"><template name="form-row"><flow style="white-space: normal; hanging-indent: 2ch"><prefix><text bind="label"/></prefix><text bind="value"/></flow></template></templates>"#).unwrap();
    let mut renderer = Renderer::new(LayoutBundle::load(dir.path()).unwrap());
    let data = Presentation::default()
        .text("label", "12:00 Sam:")
        .text("value", "hello more words");
    let scene = renderer
        .measure_content("form-row", "prefix", 20, &data)
        .unwrap();
    assert_eq!(scene.height(), 2);
    let body: Vec<_> = scene
        .text_regions
        .iter()
        .filter(|region| region.binding == "value")
        .collect();
    assert_eq!(body[0].source, 0..5);
    assert_eq!(body[0].bounds, Rect::new(11, 0, 5, 1));
    assert_eq!(body[1].source, 6..16);
    assert_eq!(body[1].bounds, Rect::new(2, 1, 10, 1));
    let mut terminal = Terminal::new(TestBackend::new(20, 2)).unwrap();
    terminal.draw(|frame| scene.paint(frame)).unwrap();
    assert_eq!(terminal.backend().buffer()[(11, 0)].symbol(), "h");
    assert_eq!(terminal.backend().buffer()[(2, 1)].symbol(), "m");
    let narrow = renderer
        .measure_content("form-row", "prefix", 8, &data)
        .unwrap();
    let first = narrow
        .text_regions
        .iter()
        .find(|region| region.binding == "value")
        .unwrap();
    assert_eq!(
        first.source.start, 0,
        "a long prefix must not discard the beginning of the message"
    );
    assert_eq!(first.bounds.y, 1);
}

#[test]
fn prefix_leaves_a_wide_first_body_glyph_for_the_next_line() {
    let fragments = super::text::measure_flow("12345678 界hello", 10, 9, 2, true, false);
    let wide = fragments
        .iter()
        .find(|fragment| fragment.text.starts_with('界'));
    assert!(
        wide.is_some(),
        "an available continuation row must retain the wide glyph: {fragments:?}"
    );
    let wide = wide.unwrap();
    assert_eq!(wide.row, 1);
    assert_eq!(wide.source.start, 9);
}

#[test]
fn combining_decorations_do_not_change_source_or_action_geometry() {
    let mut renderer = Renderer::new(LayoutBundle::builtin().unwrap());
    let data = Presentation::default().rich(
        "body",
        vec![RichSpan {
            text: "abcd efgh".into(),
            action: Some("hidden:0".into()),
            marks: [(0, "\u{0301}".into()), (6, "\u{0300}".into())].into(),
            ..Default::default()
        }],
    );
    let scene = renderer
        .measure_content("rich-text", "marks", 5, &data)
        .unwrap();
    assert_eq!(scene.height(), 2);
    assert_eq!(scene.text_regions[0].source, 0..4);
    assert_eq!(scene.text_regions[1].source, 5..9);
    assert_eq!(scene.text_regions[1].bounds.width, 4);
    assert_eq!(scene.text_regions[1].action.as_deref(), Some("hidden:0"));
    let mut terminal = Terminal::new(TestBackend::new(5, 2)).unwrap();
    terminal.draw(|frame| scene.paint(frame)).unwrap();
    assert_eq!(terminal.backend().buffer()[(0, 0)].symbol(), "a\u{0301}");
    assert_eq!(terminal.backend().buffer()[(1, 1)].symbol(), "f\u{0300}");
    for (index, mark) in [(20, "\u{0301}"), (0, "x"), (0, "\n"), (0, "\u{200d}")] {
        let data = Presentation::default().rich(
            "body",
            vec![RichSpan {
                text: "x".into(),
                marks: [(index, mark.into())].into(),
                ..Default::default()
            }],
        );
        assert!(
            renderer
                .measure_content("rich-text", "invalid", 5, &data)
                .is_err()
        );
    }
}

#[test]
fn fixed_and_natural_height_requests_do_not_share_a_cache_entry() {
    let mut renderer = Renderer::new(LayoutBundle::builtin().unwrap());
    let data = Presentation::default().text("value", "one row");
    assert_eq!(
        renderer
            .arrange("form-row", Rect::new(0, 0, 40, u16::MAX), &data)
            .unwrap()
            .height(),
        u16::MAX
    );
    assert_eq!(
        renderer
            .measure_content("form-row", "form-row", 40, &data)
            .unwrap()
            .height(),
        1
    );
}

#[test]
fn equal_width_scramble_and_marks_reuse_measured_geometry() {
    let mut renderer = Renderer::new(LayoutBundle::builtin().unwrap());
    let before = Presentation::default().rich(
        "body",
        vec![RichSpan {
            text: "abcd efgh".into(),
            action: Some("hidden:0".into()),
            ..Default::default()
        }],
    );
    let first = renderer
        .measure_content("rich-text", "animation", 5, &before)
        .unwrap();
    let count = renderer.arrangement_count();
    let after = Presentation::default().rich(
        "body",
        vec![RichSpan {
            text: "zyxw vuts".into(),
            action: Some("hidden:0".into()),
            marks: [(0, "\u{0301}".into())].into(),
            ..Default::default()
        }],
    );
    let second = renderer
        .measure_content("rich-text", "animation", 5, &after)
        .unwrap();
    assert_eq!(
        renderer.arrangement_count(),
        count,
        "paint-only scramble animation must reuse its measured breaks"
    );
    assert_eq!(first.height(), second.height());
    let mut terminal = Terminal::new(TestBackend::new(5, 2)).unwrap();
    terminal.draw(|frame| second.paint(frame)).unwrap();
    assert_eq!(terminal.backend().buffer()[(0, 0)].symbol(), "z\u{0301}");
    assert_eq!(terminal.backend().buffer()[(0, 1)].symbol(), "v");
}
