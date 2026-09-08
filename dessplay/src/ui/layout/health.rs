//! Measured terminal content-priority policy, shared by the editable health row.
use super::{Diagnostic, Presentation, Renderer};
use tuirealm::ratatui::{Frame, layout::Rect, style::Style, widgets::Paragraph};
use unicode_width::UnicodeWidthStr;

impl Renderer {
    /// Preserve metrics, reserve useful middle content, then truncate progress.
    /// All enclosing composition and appearance comes from the bundle.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn paint_health(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        progress: &Presentation,
        metrics: &Presentation,
        middle: &str,
        tone: Style,
        marquee: Option<usize>,
    ) -> Result<usize, Diagnostic> {
        let middle_data = Presentation::default()
            .text("body", middle)
            .boolean("suggestion", true)
            .style("body", tone);
        let probe = self.arrange_named(
            "health",
            "health:probe",
            area,
            &Presentation::default()
                .slot("metrics", 1, 1)
                .slot("progress", 1, 1)
                .slot("middle", 1, 1),
        )?;
        let metrics_width = if probe.slot("metrics").is_empty() {
            0
        } else {
            self.intrinsic_width("health-metrics", metrics)?
        };
        let progress_width = if probe.slot("progress").is_empty() {
            0
        } else {
            self.intrinsic_width("health-progress", progress)?
        };
        let remaining = probe.root_content().width.saturating_sub(metrics_width);
        let chrome_width =
            self.intrinsic_width("health-middle", &middle_data.clone().text("body", ""))?;
        let minimum = chrome_width.saturating_add(4);
        let reserve = if middle.is_empty() || probe.slot("middle").is_empty() {
            0
        } else {
            let desired = self
                .intrinsic_width("health-middle", &middle_data)?
                .max(minimum)
                .min(remaining);
            if desired >= minimum { desired } else { 0 }
        };
        let progress_width =
            progress_width.min(remaining.saturating_sub(if reserve > 0 { reserve } else { 2 }));
        let free = remaining.saturating_sub(progress_width);
        let scene = self.arrange(
            "health",
            area,
            &Presentation::default()
                .slot("metrics", metrics_width, 1)
                .slot("progress", progress_width, 1)
                .slot("middle", free, 1),
        )?;
        let mut window_width = 0;
        let mut error = None;
        scene.paint_with_slots(frame, |name, frame, area, style| {
            let result = (|| {
                match name {
                    "progress" | "metrics" => {
                        let (template, data) = if name == "progress" {
                            ("health-progress", progress)
                        } else {
                            ("health-metrics", metrics)
                        };
                        self.arrange(template, area, &data.clone().inherit(style, 0))?
                            .paint_with_slots(frame, |_, _, _, _| {});
                    }
                    "middle" => {
                        let data = middle_data
                            .clone()
                            .inherit(style, 0)
                            .boolean("suggestion", marquee.is_none() && area.width >= minimum)
                            .boolean("animation", marquee.is_some() && area.width >= minimum)
                            .slot("marquee", middle.width().min(u16::MAX as usize) as u16, 1);
                        let scene = self.arrange("health-middle", area, &data)?;
                        // Measure the available window even while a warning owns it.
                        window_width = scene.root_content().width as usize;
                        scene.paint_with_slots(frame, |name, frame, area, style| {
                            if name == "marquee" {
                                window_width = usize::from(area.width);
                                if let Some((text, pad)) = marquee.and_then(|offset| {
                                    crate::ui::props::marquee_window(middle, window_width, offset)
                                }) {
                                    let pad = pad.min(usize::from(area.width)) as u16;
                                    frame.render_widget(
                                        Paragraph::new(text).style(self.paint_style(style)),
                                        Rect {
                                            x: area.x.saturating_add(pad),
                                            width: area.width.saturating_sub(pad),
                                            ..area
                                        },
                                    );
                                }
                            }
                        });
                    }
                    _ => {}
                }
                Ok::<_, Diagnostic>(())
            })();
            if let Err(diagnostic) = result {
                error = Some(diagnostic);
            }
        });
        error.map_or(Ok(window_width), Err)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::ui::layout::{LayoutBundle, PresentedItem};
    use tuirealm::ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn intrinsic_measurements_include_authored_spacing_without_wrapped_feedback() {
        let mut renderer = Renderer::new(LayoutBundle::builtin().unwrap());
        let data = Presentation::default()
            .text("body", "wide 世界")
            .boolean("suggestion", true);
        assert_eq!(
            renderer.intrinsic_width("health-middle", &data).unwrap(),
            13
        );
        assert_eq!(
            renderer
                .measure_content("health-middle", "narrow", 8, &data)
                .unwrap()
                .height(),
            1
        );
        assert_eq!(
            renderer.intrinsic_width("health-middle", &data).unwrap(),
            13
        );
    }

    #[test]
    fn health_templates_reorder_fields_and_hidden_metrics_release_their_budget() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("templates")).unwrap();
        std::fs::write(directory.path().join("templates/health.xml"), r#"<templates version="1"><template name="health-progress"><flow if="available"><text bind="duration"/><text bind="elapsed"/></flow></template><template name="health-metric"><flow><text bind="value"/><text bind="label"/></flow></template></templates>"#).unwrap();
        std::fs::write(
            directory.path().join("style.css"),
            "#health-line { margin: 0 1ch; } #health-middle { padding: 0 1ch; }",
        )
        .unwrap();
        let mut renderer = Renderer::new(LayoutBundle::load(directory.path()).unwrap());
        let metrics = Presentation::default().list(
            "metrics",
            vec![PresentedItem {
                key: "rtt".into(),
                data: Presentation::default()
                    .text("label", "rtt")
                    .text("value", "89ms"),
            }],
        );
        let progress = Presentation::default()
            .boolean("available", true)
            .text("elapsed", "12:34")
            .text("duration", "24:00");
        let mut terminal = Terminal::new(TestBackend::new(80, 3)).unwrap();
        let mut paint = |renderer: &mut Renderer| {
            let frame = terminal
                .draw(|frame| {
                    renderer
                        .paint_health(
                            frame,
                            Rect::new(3, 1, 70, 1),
                            &progress,
                            &metrics,
                            "suggestion",
                            Style::default(),
                            None,
                        )
                        .unwrap();
                })
                .unwrap();
            (0..80)
                .map(|x| frame.buffer[(x, 1)].symbol())
                .collect::<String>()
        };
        let line = paint(&mut renderer);
        assert!(line.starts_with("    24:00 12:34"), "{line:?}");
        assert!(line.ends_with("89ms rtt        "), "{line:?}");
        std::fs::write(
            directory.path().join("style.css"),
            "#health-metrics { display: none; }",
        )
        .unwrap();
        renderer.install(LayoutBundle::load(directory.path()).unwrap());
        let line = paint(&mut renderer);
        assert!(!line.contains("rtt"), "{line:?}");
        assert!(line.contains("suggestion"), "{line:?}");
    }
}
