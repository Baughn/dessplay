use super::*;
use ratatui_image::{Image, Resize, picker::Picker, protocol::Protocol};
use tuirealm::event::MouseEventKind;
use tuirealm::ratatui::layout::Size;
use tuirealm::ratatui::style::Color;
use tuirealm::ratatui::widgets::Block;

/// A chrome-free view with its own source and encoding, independent of the
/// inline image's fitted size and lifetime in the chat window.
pub(crate) struct ImageModal {
    image: image::DynamicImage,
    picker: Picker,
    encoded: Option<(Size, Option<Protocol>)>,
}

impl ImageModal {
    pub(crate) fn new(image: image::DynamicImage, mut picker: Picker) -> Self {
        picker.set_background_color(Some(image::Rgba([0, 0, 0, 255])));
        Self {
            image,
            picker,
            encoded: None,
        }
    }

    pub(crate) fn render(&mut self, frame: &mut Frame, area: Rect) -> Rect {
        frame.render_widget(
            Block::default().style(Style::default().bg(Color::Black)),
            area,
        );
        if area.is_empty() {
            return Rect::default();
        }
        let size = Size::new(area.width, area.height);
        if self.encoded.as_ref().map(|(size, _)| *size) != Some(size) {
            let protocol = self
                .picker
                .new_protocol(self.image.clone(), size, Resize::Scale(None));
            self.encoded = Some((
                size,
                match protocol {
                    Ok(protocol) => Some(protocol),
                    Err(error) => {
                        tracing::debug!(%error, "encoding fullscreen chat image failed");
                        None
                    }
                },
            ));
        }
        let Some((_, Some(protocol))) = &self.encoded else {
            return Rect::default();
        };
        let size = protocol.size();
        let pixels = Rect::new(
            area.x + area.width.saturating_sub(size.width) / 2,
            area.y + area.height.saturating_sub(size.height) / 2,
            size.width,
            size.height,
        );
        frame.render_widget(Image::new(protocol), pixels);
        pixels
    }
}

impl AppComponent<Msg, NoUserEvent> for ImageModal {
    fn on(&mut self, event: &Event<NoUserEvent>) -> Option<Msg> {
        match event {
            Event::Keyboard(_) => Some(Msg::CloseModal),
            Event::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::Down(_)) => {
                Some(Msg::CloseModal)
            }
            _ => None,
        }
    }
}

passive_modal!(ImageModal);
