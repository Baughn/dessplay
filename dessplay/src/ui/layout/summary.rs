//! Whole-entry budgets for bounded wound and threat summaries.
use super::{Diagnostic, Presentation, PresentedRow, Renderer};
use tuirealm::ratatui::{Frame, layout::Rect, style::Style};

impl Renderer {
    fn summary_height(
        &mut self,
        template: &str,
        rows: &[PresentedRow],
        width: u16,
        style: Style,
    ) -> Result<u16, Diagnostic> {
        let mut height = 0u16;
        for row in rows {
            height = height.saturating_add(
                self.measure_content(
                    template,
                    &format!("summary:{template}/{}", row.key),
                    width,
                    &row.data.clone().inherit(style, 0),
                )?
                .height(),
            );
        }
        Ok(height)
    }

    /// Reserve room for both stories, retaining complete entries and an omission marker.
    pub(crate) fn paint_rogue_summary(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        wounds: &[PresentedRow],
        threats: &[PresentedRow],
        style: Style,
    ) -> Result<(), Diagnostic> {
        let has_threats = !threats.is_empty() && area.height >= 3;
        let probe = self.arrange_named(
            "rogue-summary",
            "rogue-summary:probe",
            area,
            &Presentation::default()
                .boolean("has-threats", has_threats)
                .slot("wounds", 0, area.height)
                .inherit(style, 0),
        )?;
        let content = probe.root_content();
        let wound_need = self.summary_height(
            "rogue-condition",
            wounds,
            probe.slot("wounds").width,
            probe.style("wounds"),
        )?;
        let threat_need = self.summary_height(
            "rogue-threat",
            threats,
            probe.slot("threats").width,
            probe.style("threats"),
        )?;
        // The probe includes authored gaps, borders, and padding before budgets.
        let chrome = content.height.saturating_sub(
            probe
                .slot("wounds")
                .height
                .saturating_add(probe.slot("threats").height),
        );
        let available = content.height.saturating_sub(chrome);
        let threat_height = if !has_threats {
            0
        } else if wound_need.saturating_add(threat_need) <= available {
            threat_need
        } else {
            threat_need.min(available.saturating_sub(wound_need.min(available - available / 2)))
        };
        let wound_height = self
            .bounded_scenes(
                Rect::new(
                    0,
                    0,
                    probe.slot("wounds").width,
                    available.saturating_sub(threat_height),
                ),
                "rogue-condition",
                wounds,
                "More wounds:",
                "v",
                probe.style("wounds"),
            )?
            .iter()
            .fold(0u16, |height, scene| height.saturating_add(scene.height()))
            .min(available.saturating_sub(threat_height));
        let data = Presentation::default()
            .boolean("has-threats", has_threats)
            .slot("wounds", 0, wound_height)
            .slot("threats", 0, threat_height)
            .inherit(style, 0);
        let scene = self.arrange("rogue-summary", area, &data)?;
        let mut result = Ok(());
        scene.paint_with_slots(frame, |name, frame, area, style| {
            let next = match name {
                "wounds" => self.paint_bounded(
                    frame,
                    area,
                    "rogue-condition",
                    wounds,
                    "More wounds:",
                    "v",
                    style,
                ),
                "threats" => self.paint_bounded(
                    frame,
                    area,
                    "rogue-threat",
                    threats,
                    "more threats",
                    "+",
                    style,
                ),
                _ => Ok(()),
            };
            if next.is_err() {
                result = next;
            }
        });
        result
    }

    /// Fit whole measured entries; never wrap or draw them a second time.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn paint_bounded(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        template: &str,
        rows: &[PresentedRow],
        omission: &str,
        compact: &str,
        style: Style,
    ) -> Result<(), Diagnostic> {
        let scenes = self.bounded_scenes(area, template, rows, omission, compact, style)?;
        let mut offset = 0;
        for scene in scenes {
            scene.paint_scrolled(frame, area, offset);
            offset += i32::from(scene.height());
        }
        Ok(())
    }

    fn bounded_scenes(
        &mut self,
        area: Rect,
        template: &str,
        rows: &[PresentedRow],
        omission: &str,
        compact: &str,
        style: Style,
    ) -> Result<Vec<super::RenderedScene>, Diagnostic> {
        if area.is_empty() {
            return Ok(Vec::new());
        }
        let mut entries = Vec::new();
        let mut heights = vec![0u16];
        for row in rows {
            let scene = self.measure_content(
                template,
                &format!("summary:{template}/{}", row.key),
                area.width,
                &row.data.clone().inherit(style, 0),
            )?;
            if scene.height() > 0 {
                heights.push(
                    heights
                        .last()
                        .copied()
                        .unwrap_or(0)
                        .saturating_add(scene.height()),
                );
                entries.push(scene);
            }
        }
        let mut count = entries.len();
        let mut marker = None;
        while heights[count].saturating_add(
            marker
                .as_ref()
                .map_or(0, |scene: &super::RenderedScene| scene.height()),
        ) > area.height
        {
            if count == 0 {
                marker = Some(
                    self.measure_content(
                        "rogue-omission",
                        &format!("summary:{template}/compact"),
                        area.width,
                        &Presentation::default()
                            .text("label", compact)
                            .inherit(style, 0),
                    )?,
                );
                break;
            }
            count -= 1;
            let counted = compact == "+";
            let data = Presentation::default()
                .text("label", omission)
                .text("key", if counted { "" } else { compact })
                .text("count", (entries.len() - count).to_string())
                .boolean("counted", counted)
                .inherit(style, 0);
            marker = Some(self.measure_content(
                "rogue-omission",
                &format!("summary:{template}/omission"),
                area.width,
                &data,
            )?);
        }
        entries.truncate(count);
        entries.extend(marker);
        Ok(entries)
    }
}
