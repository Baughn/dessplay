use super::*;
use crate::ui::widgets::search::{Effect, Entry, Search};

/// Stable source identities, resolved again when leaving the search dialog.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Playlist(Ed2kHash),
    Franchise(dessplay_core::franchise::FranchiseKey),
    User(String),
    List { heading: String, id: ListEntryId },
    Subtitle(u64),
    Log(u64),
}

pub struct PaneSearch {
    title: String,
    pub(crate) search: Search<Target>,
}

impl PaneSearch {
    pub fn new(title: &str, entries: Vec<Entry<Target>>) -> Self {
        Self {
            title: format!("Search {title}"),
            search: Search::new(entries),
        }
    }

    pub fn keybindings(&self) -> Vec<(&'static str, &'static str)> {
        self.search.keybindings()
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        if let Ok(bundle) = crate::ui::layout::LayoutBundle::builtin() {
            self.render_layout(frame, area, &mut crate::ui::layout::Renderer::new(bundle));
        }
    }

    pub(crate) fn render_layout(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        renderer: &mut crate::ui::layout::Renderer,
    ) {
        use crate::ui::layout::{Presentation, PresentedRow};
        let data = Presentation::default()
            .text("title", &self.title)
            .text("message", "No matches")
            .boolean("editing", true)
            .boolean("has-message", self.search.matches.is_empty())
            .boolean("has-results", !self.search.matches.is_empty());
        let rows: Vec<_> = self
            .search
            .matches
            .iter()
            .map(|i| PresentedRow {
                key: format!("{:?}", self.search.entries[*i].key),
                data: Presentation::default().text("text", &self.search.entries[*i].text),
                gap_after: false,
            })
            .collect();
        let Ok(scene) = renderer.arrange("pane-search", area, &data) else {
            return;
        };
        let mut visible = false;
        scene.paint_with_slots(frame, |name, frame, area, style| match name {
            "editor" => self.search.editor.render_content_styled(
                frame,
                area,
                true,
                false,
                style,
                renderer.color_depth(),
            ),
            "body" => {
                visible = true;
                let _ = renderer.paint_cursor_collection(
                    frame,
                    area,
                    "pane-search-result",
                    &rows,
                    &mut self.search.cursor,
                    rows.len(),
                    style,
                );
            }
            _ => {}
        });
        if !visible {
            self.search.cursor.hide_all(rows.len());
        }
    }
}

passive_modal!(PaneSearch);
impl AppComponent<Msg, NoUserEvent> for PaneSearch {
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        Some(match self.search.on(ev) {
            Effect::Close => Msg::CloseModal,
            Effect::Accept(key) => Msg::SearchChosen(key),
            _ => Msg::None,
        })
    }
}
