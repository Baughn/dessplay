use super::compiler::Node;
use super::css::{Computed, resolve};
use super::{Diagnostic, Fragment, LayoutBundle, measure_text};
use std::collections::{BTreeMap, HashMap};
use taffy::prelude::*;
use tuirealm::ratatui::{
    Frame, layout::Rect, style::Style as PaintStyle, text::Line, widgets::Paragraph,
};
use unicode_width::UnicodeWidthStr;

/// Typed presentation fields. Strings are always data, never parsed as markup.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Presentation {
    texts: BTreeMap<String, String>,
    bools: BTreeMap<String, bool>,
    slots: BTreeMap<String, (u16, u16)>,
    states: BTreeMap<String, Vec<String>>,
    styles: BTreeMap<String, PaintStyle>,
    preserve_end: Vec<String>,
    inherited_style: PaintStyle,
    shares: BTreeMap<String, u16>,
}
impl Presentation {
    /// Drag proportions in basis points, separate from authored CSS.
    pub(crate) fn shares(mut self, shares: BTreeMap<String, u16>) -> Self {
        self.shares = shares;
        self
    }
    /// Supply a plain-text binding.
    pub fn text(mut self, name: &str, value: impl Into<String>) -> Self {
        self.texts.insert(name.into(), value.into());
        self
    }
    /// Supply a condition binding.
    pub fn boolean(mut self, name: &str, value: bool) -> Self {
        self.bools.insert(name.into(), value);
        self
    }
    /// Supply the intrinsic unwrapped size of a controller-owned primitive.
    pub fn slot(mut self, name: &str, width: u16, height: u16) -> Self {
        self.slots.insert(name.into(), (width, height));
        self
    }
    /// Supply semantic focus/selection/disabled state for a stable node id.
    pub fn state(mut self, id: &str, state: &str) -> Self {
        self.states.entry(id.into()).or_default().push(state.into());
        self
    }
    /// Semantic text style; authored CSS has precedence over this default.
    pub fn style(mut self, binding: &str, style: PaintStyle) -> Self {
        self.styles.insert(binding.into(), style);
        self
    }
    /// Keep the filename end of a path visible when its primitive is clipped.
    pub fn preserve_end(mut self, binding: &str) -> Self {
        self.preserve_end.push(binding.into());
        self
    }
}

/// A stable, controller-owned row in a virtualized collection.
pub struct PresentedRow {
    /// Semantic identity, independent of ordering.
    pub key: String,
    /// Typed fields exposed before padding or clipping.
    pub data: Presentation,
    /// A blank, noninteractive row following the item.
    pub gap_after: bool,
}

/// Painted geometry and semantic slots, published as one immutable frame result.
#[derive(Clone, Debug, Default)]
pub struct RenderedScene {
    nodes: Vec<Arranged>,
    pub(crate) splits: Vec<SplitRegion>,
}

/// A handle and its adjacent children, derived from the painted scene.
#[derive(Clone, Debug)]
pub(crate) struct SplitRegion {
    pub id: String,
    pub handle: Rect,
    horizontal: bool,
    origin: u16,
    extent: u16,
    before: usize,
    children: Vec<(String, u16)>,
}
impl SplitRegion {
    pub fn drag(&self, position: tuirealm::ratatui::layout::Position) -> BTreeMap<String, u16> {
        let coordinate = if self.horizontal {
            position.x
        } else {
            position.y
        };
        let offset = u32::from(coordinate.saturating_sub(self.origin));
        let pointer = (offset * 10_000 / u32::from(self.extent.max(1))).min(10_000) as u16;
        let mut children = self.children.clone();
        let preceding: u16 = children[..self.before].iter().map(|(_, n)| n).sum();
        let pair = children[self.before].1 + children[self.before + 1].1;
        // Preserve the default ten-percent minimum of the entire container.
        let minimum = 1000.min(pair / 2);
        let share = pointer
            .saturating_sub(preceding)
            .clamp(minimum, pair - minimum);
        children[self.before].1 = share;
        children[self.before + 1].1 = pair - share;
        children.into_iter().collect()
    }
}
#[derive(Clone, Debug)]
struct Arranged {
    overlay: bool,
    overlay_root: bool,
    id: String,
    slot: String,
    bounds: Rect,
    content: Rect,
    clip: Rect,
    style: Computed,
    title: String,
    fragments: Vec<Fragment>,
}
impl RenderedScene {
    /// Arranged visible bounds of a named component.
    pub fn bounds(&self, id: &str) -> Rect {
        self.nodes
            .iter()
            .find(|n| n.id == id)
            .map_or(Rect::default(), |n| n.bounds.intersection(n.clip))
    }
    /// Visible controller slots, in markup order (independent of box placement).
    pub fn visible_slots(&self) -> Vec<&str> {
        self.nodes
            .iter()
            .filter(|n| !n.slot.is_empty() && !n.content.intersection(n.clip).is_empty())
            .map(|n| n.slot.as_str())
            .collect()
    }
    /// Visible content rectangle for a controller primitive, from painted geometry.
    pub fn slot(&self, name: &str) -> Rect {
        self.nodes
            .iter()
            .find(|n| n.slot == name)
            .map_or(Rect::default(), |n| n.content.intersection(n.clip))
    }
    /// Resolved text appearance for a controller primitive.
    pub fn style(&self, name: &str) -> PaintStyle {
        self.nodes
            .iter()
            .find(|n| n.slot == name)
            .map_or(PaintStyle::default(), |n| n.style.paint)
    }
    /// Arranged bounds and matched declarations for the layout inspector.
    pub fn inspect(&self) -> Vec<String> {
        self.nodes
            .iter()
            .map(|n| {
                format!(
                    "{} {} {:?} clip {:?}\n{}",
                    n.id,
                    n.slot,
                    n.bounds,
                    n.clip,
                    n.style
                        .matched
                        .iter()
                        .map(|d| format!(
                            "  {}: {} ({}:{})",
                            d.name,
                            d.value,
                            d.location.file.display(),
                            d.location.line
                        ))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            })
            .collect()
    }
    /// Paint container chrome and already-measured text. Slots are filled by primitives.
    pub fn paint(&self, frame: &mut Frame<'_>) {
        self.paint_layer(frame, false);
    }
    /// Paint declared overlays after ordinary content primitives.
    pub fn paint_overlays(&self, frame: &mut Frame<'_>) {
        self.paint_layer(frame, true);
    }
    fn paint_layer(&self, frame: &mut Frame<'_>, overlay: bool) {
        for n in self.nodes.iter().filter(|n| n.overlay == overlay) {
            let visible = n.bounds.intersection(n.clip).intersection(frame.area());
            if visible.is_empty() {
                continue;
            }
            if n.overlay_root {
                frame.render_widget(tuirealm::ratatui::widgets::Clear, visible);
            }
            frame.buffer_mut().set_style(visible, n.style.paint);
            if n.style.layout.border.left != LengthPercentage::Length(0.0) || !n.title.is_empty() {
                // Draw only original frame edges inside the clip. Memory use is
                // bounded by the terminal, even for enormous authored dimensions.
                if n.style.layout.border.left != LengthPercentage::Length(0.0) {
                    for y in visible.y..visible.bottom() {
                        for x in visible.x..visible.right() {
                            let left = x == n.bounds.x;
                            let right = x == n.bounds.right().saturating_sub(1);
                            let top = y == n.bounds.y;
                            let bottom = y == n.bounds.bottom().saturating_sub(1);
                            let glyph = match (left, right, top, bottom) {
                                (true, _, true, _) => "┌",
                                (_, true, true, _) => "┐",
                                (true, _, _, true) => "└",
                                (_, true, _, true) => "┘",
                                (_, _, true, _) | (_, _, _, true) => "─",
                                (true, _, _, _) | (_, true, _, _) => "│",
                                _ => continue,
                            };
                            frame.buffer_mut()[(x, y)]
                                .set_symbol(glyph)
                                .set_style(n.style.paint.fg(n.style.border));
                        }
                    }
                }
                let title = Rect::new(
                    n.bounds.x.saturating_add(1),
                    n.bounds.y,
                    n.bounds.width.saturating_sub(2),
                    1,
                );
                paint_line(
                    frame,
                    &n.title,
                    title,
                    n.clip,
                    n.style.paint,
                    Default::default(),
                );
            }
            for (i, fragment) in n.fragments.iter().enumerate() {
                let y = n.content.y.saturating_add(i.min(u16::MAX as usize) as u16);
                if y >= n.content.bottom() {
                    break;
                }
                let row = Rect::new(
                    n.content.x.saturating_add(fragment.indent as u16),
                    y,
                    n.content.width.saturating_sub(fragment.indent as u16),
                    1,
                );
                paint_line(
                    frame,
                    &fragment.text,
                    row,
                    n.clip,
                    n.style.paint,
                    n.style.align,
                );
            }
        }
    }
}

fn paint_line(
    frame: &mut Frame<'_>,
    text: &str,
    row: Rect,
    clip: Rect,
    style: PaintStyle,
    alignment: tuirealm::ratatui::layout::Alignment,
) {
    let visible = row.intersection(clip).intersection(frame.area());
    if visible.is_empty() || text.is_empty() {
        return;
    }
    let padding = row
        .width
        .saturating_sub(text.width().min(u16::MAX as usize) as u16);
    let indent = match alignment {
        tuirealm::ratatui::layout::Alignment::Center => padding / 2,
        tuirealm::ratatui::layout::Alignment::Right => padding,
        _ => 0,
    };
    let origin = row.x.saturating_add(indent);
    let painted =
        Rect::new(origin, row.y, row.width.saturating_sub(indent), 1).intersection(visible);
    if !painted.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::raw(text.to_owned()))
                .style(style)
                .scroll((0, painted.x.saturating_sub(origin))),
            painted,
        );
    }
}

struct Built<'a> {
    node: &'a Node,
    id: NodeId,
    style: Computed,
    children: Vec<Built<'a>>,
    text: String,
}
#[derive(Clone)]
struct Measure {
    width: f32,
    height: f32,
    text: String,
    wrap: bool,
}

/// UI-thread renderer. Its Taffy state never crosses the controller's thread move.
pub struct Renderer {
    bundle: LayoutBundle,
    tree: TaffyTree<Measure>,
    cache: HashMap<String, (Rect, Presentation, RenderedScene)>,
    text_cache: HashMap<(String, u16), std::sync::Arc<Vec<Fragment>>>,
}
impl Renderer {
    /// Construct locally after entering the UI thread.
    pub fn new(bundle: LayoutBundle) -> Self {
        Self {
            bundle,
            tree: TaffyTree::new(),
            cache: HashMap::new(),
            text_cache: HashMap::new(),
        }
    }
    /// Replace definitions atomically; controller state is external to this renderer.
    pub fn install(&mut self, bundle: LayoutBundle) {
        if self.bundle.revision != bundle.revision {
            self.cache.clear();
            self.text_cache.clear();
            self.tree.clear();
        }
        self.bundle = bundle;
    }
    /// Currently installed immutable definitions.
    pub fn bundle(&self) -> &LayoutBundle {
        &self.bundle
    }
    /// Cache immutable wrapped log rows by their text and frozen width.
    pub fn measured_text(&mut self, text: &str, width: u16) -> std::sync::Arc<Vec<Fragment>> {
        let key = (text.to_string(), width);
        if let Some(rows) = self.text_cache.get(&key) {
            return rows.clone();
        }
        // The log tail is capped at 2000 lines. Two widths retain useful work
        // across a resize without allowing unbounded historical measurements.
        if self.text_cache.len() >= 4096 {
            self.text_cache.clear();
        }
        let rows = std::sync::Arc::new(measure_text(text, width as usize, 0, 0, true, false));
        self.text_cache.insert(key, rows.clone());
        rows
    }
    /// Latest arranged templates, including matched declarations and bounds.
    pub fn inspect(&self) -> String {
        let mut names = self.cache.keys().collect::<Vec<_>>();
        names.sort();
        names
            .into_iter()
            .map(|name| format!("{name}\n{}", self.cache[name].2.inspect().join("\n")))
            .collect::<Vec<_>>()
            .join("\n")
    }
    /// Expand, style, and allocate a template in terminal cells.
    pub fn arrange(
        &mut self,
        template: &str,
        area: Rect,
        data: &Presentation,
    ) -> Result<RenderedScene, Diagnostic> {
        self.arrange_instance(template, template, area, data)
    }
    fn arrange_instance(
        &mut self,
        template: &str,
        key: &str,
        area: Rect,
        data: &Presentation,
    ) -> Result<RenderedScene, Diagnostic> {
        if let Some((old_area, old_data, scene)) = self.cache.get(key)
            && *old_area == area
            && old_data == data
        {
            return Ok(scene.clone());
        }
        let root = self.bundle.templates.get(template).ok_or_else(|| {
            Diagnostic::at(
                std::path::Path::new("<renderer>"),
                "",
                0,
                "unknown template",
            )
        })?;
        self.tree.clear();
        let built = build(
            &mut self.tree,
            &self.bundle,
            root,
            data,
            &mut Vec::new(),
            &Computed {
                paint: data.inherited_style,
                ..Computed::default()
            },
        )?;
        let available = Size {
            width: AvailableSpace::Definite(area.width as f32),
            height: AvailableSpace::Definite(area.height as f32),
        };
        let mut root_style = built.style.layout.clone();
        root_style.size = Size {
            width: Dimension::Length(area.width as f32),
            height: Dimension::Length(area.height as f32),
        };
        self.tree
            .set_style(built.id, root_style.clone())
            .map_err(internal)?;
        // Pass one depends only on unwrapped width, never on wrapped height.
        self.tree
            .compute_layout_with_measure(built.id, available, |known, _, _, context, _| {
                let Some(m) = context else {
                    return Size::ZERO;
                };
                Size {
                    width: known.width.unwrap_or(m.width),
                    height: known.height.unwrap_or(m.height),
                }
            })
            .map_err(internal)?;
        freeze_widths(&mut self.tree, &built)?;
        self.tree
            .compute_layout_with_measure(built.id, available, |known, available, _, context, _| {
                let Some(m) = context else {
                    return Size::ZERO;
                };
                let width = known.width.unwrap_or(match available.width {
                    AvailableSpace::Definite(w) => w,
                    _ => m.width,
                });
                let height = if m.text.is_empty() || !m.wrap {
                    m.height
                } else {
                    measure_text(&m.text, width.max(0.0) as usize, 0, 0, true, false).len() as f32
                };
                Size {
                    width,
                    height: known.height.unwrap_or(height),
                }
            })
            .map_err(internal)?;
        let mut scene = RenderedScene::default();
        collect(
            &self.tree,
            &built,
            data,
            (area.x as f32, area.y as f32),
            area,
            false,
            &mut scene,
        )?;
        if self.cache.len() > 512 {
            self.cache.clear();
        }
        self.cache
            .insert(key.into(), (area, data.clone(), scene.clone()));
        Ok(scene)
    }
    /// Paint the visible slice of a one-line semantic collection. Offscreen
    /// entries remain in the controller and never become Taffy nodes.
    pub fn paint_rows(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        template: &str,
        rows: &[PresentedRow],
        selected: Option<usize>,
        style: PaintStyle,
    ) -> Result<(), Diagnostic> {
        let mut keys = std::collections::BTreeSet::new();
        if rows.iter().any(|row| !keys.insert(&row.key)) {
            return Err(internal("duplicate semantic collection key"));
        }
        let mut positions = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            positions.push(Some(index));
            if row.gap_after {
                positions.push(None);
            }
        }
        let target = selected.and_then(|s| positions.iter().position(|p| *p == Some(s)));
        let top = target.map_or(0, |i| {
            i.saturating_sub(area.height as usize / 2)
                .min(positions.len().saturating_sub(area.height as usize))
        });
        for (y, index) in positions
            .iter()
            .skip(top)
            .take(area.height as usize)
            .enumerate()
        {
            let Some(index) = index else {
                continue;
            };
            let row = &rows[*index];
            let rect = Rect::new(area.x, area.y + y as u16, area.width, 1);
            let mut data = row.data.clone();
            data.inherited_style = style;
            if selected == Some(*index) {
                data = data.state("form-row", "selected");
            }
            let scene =
                self.arrange_instance(template, &format!("{template}/{}", row.key), rect, &data)?;
            scene.paint(frame);
            if selected == Some(*index) {
                frame
                    .buffer_mut()
                    .set_style(rect, crate::ui::theme::highlight_style());
            }
        }
        Ok(())
    }
}
fn internal(e: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::at(
        std::path::Path::new("<renderer>"),
        "",
        0,
        format!("layout allocation: {e}"),
    )
}
fn build<'a>(
    tree: &mut TaffyTree<Measure>,
    bundle: &LayoutBundle,
    node: &'a Node,
    data: &'a Presentation,
    path: &mut Vec<(&'a Node, Vec<&'a str>)>,
    parent: &Computed,
) -> Result<Built<'a>, Diagnostic> {
    let states = data
        .states
        .get(node.attr("id"))
        .map(|s| s.iter().map(String::as_str).collect())
        .unwrap_or_default();
    path.push((node, states));
    let mut style = resolve(bundle, path, parent)?;
    if let Some(share) = data.shares.get(node.attr("id")) {
        style.layout.flex_basis = Dimension::Percent(f32::from(*share) / 10_000.0);
        // Percentage shares divide the available track after authored gaps.
        style.layout.flex_shrink = 1.0;
    }
    if let Some(semantic) = data.styles.get(node.attr("bind")) {
        let semantic = if style.paint.fg.is_some() {
            semantic.remove_modifier(tuirealm::ratatui::style::Modifier::DIM)
        } else {
            *semantic
        };
        style.paint = semantic.patch(style.paint);
    }
    if !node.attr("if").is_empty() && !data.bools.get(node.attr("if")).copied().unwrap_or(false) {
        style.layout.display = Display::None;
    }
    let mut children = Vec::new();
    for child in &node.children {
        children.push(build(tree, bundle, child, data, path, &style)?);
    }
    path.pop();
    let text = data
        .texts
        .get(node.attr("bind"))
        .cloned()
        .unwrap_or_default();
    let (width, height) = data.slots.get(node.attr("name")).copied().unwrap_or((0, 0));
    let measure = Measure {
        width: if text.is_empty() {
            width as f32
        } else {
            text.lines().map(UnicodeWidthStr::width).max().unwrap_or(0) as f32
        },
        height: if text.is_empty() {
            height as f32
        } else {
            text.lines().count().max(1) as f32
        },
        text: text.clone(),
        wrap: style.wrap,
    };
    let id = if children.is_empty() {
        tree.new_leaf_with_context(style.layout.clone(), measure)
    } else {
        tree.new_with_children(
            style.layout.clone(),
            &children.iter().map(|c| c.id).collect::<Vec<_>>(),
        )
    }
    .map_err(internal)?;
    Ok(Built {
        node,
        id,
        style,
        children,
        text,
    })
}
fn freeze_widths(tree: &mut TaffyTree<Measure>, built: &Built<'_>) -> Result<(), Diagnostic> {
    let width = tree.layout(built.id).map_err(internal)?.size.width;
    let mut style = tree.style(built.id).map_err(internal)?.clone();
    style.size.width = Dimension::Length(width);
    style.min_size.width = Dimension::Length(width);
    style.max_size.width = Dimension::Length(width);
    tree.set_style(built.id, style).map_err(internal)?;
    for child in &built.children {
        freeze_widths(tree, child)?;
    }
    Ok(())
}
fn collect(
    tree: &TaffyTree<Measure>,
    built: &Built<'_>,
    data: &Presentation,
    origin: (f32, f32),
    clip: Rect,
    in_overlay: bool,
    scene: &mut RenderedScene,
) -> Result<(), Diagnostic> {
    if built.style.layout.display == Display::None {
        return Ok(());
    }
    let layout = tree.layout(built.id).map_err(internal)?;
    let x = origin.0 + layout.location.x;
    let y = origin.1 + layout.location.y;
    let cell = |n: f32| n.round().clamp(0.0, u16::MAX as f32) as u16;
    let bounds = Rect::new(
        cell(x),
        cell(y),
        cell(x + layout.size.width).saturating_sub(cell(x)),
        cell(y + layout.size.height).saturating_sub(cell(y)),
    );
    let left = layout.border.left + layout.padding.left;
    let top = layout.border.top + layout.padding.top;
    let content = Rect::new(
        cell(x + left),
        cell(y + top),
        cell(layout.size.width - left - layout.border.right - layout.padding.right),
        cell(layout.size.height - top - layout.border.bottom - layout.padding.bottom),
    );
    let text = if data
        .preserve_end
        .iter()
        .any(|name| name == built.node.attr("bind"))
    {
        crate::ui::widgets::table::truncate_display_start(&built.text, content.width as usize).0
    } else {
        built.text.clone()
    };
    let fragments = measure_text(
        &text,
        content.width as usize,
        0,
        0,
        built.style.wrap,
        built.style.ellipsis,
    );
    scene.nodes.push(Arranged {
        overlay: in_overlay || built.node.tag == "overlay",
        overlay_root: built.node.tag == "overlay",
        id: built.node.attr("id").into(),
        slot: if built.node.tag == "slot" {
            built.node.attr("name").into()
        } else {
            String::new()
        },
        bounds,
        content,
        clip,
        style: built.style.clone(),
        title: data
            .texts
            .get(built.node.attr("title"))
            .cloned()
            .unwrap_or_default(),
        fragments,
    });
    let mut split_children = Vec::new();
    for child in &built.children {
        let index = scene.nodes.len();
        collect(
            tree,
            child,
            data,
            (x, y),
            clip.intersection(content),
            in_overlay || built.node.tag == "overlay",
            scene,
        )?;
        if let Some(node) = scene.nodes.get(index)
            && !node.bounds.intersection(node.clip).is_empty()
        {
            split_children.push((child.node.attr("id").to_string(), node.bounds));
        }
    }
    if built.node.attr("resizable") == "true" && split_children.len() >= 2 {
        let horizontal = built.style.layout.flex_direction == FlexDirection::Row;
        let origin = if horizontal { content.x } else { content.y };
        let extent = if horizontal {
            content.width
        } else {
            content.height
        };
        let total: u32 = split_children
            .iter()
            .map(|(_, r)| u32::from(if horizontal { r.width } else { r.height }))
            .sum();
        let saved: Option<Vec<_>> = split_children
            .iter()
            .map(|(id, _)| data.shares.get(id).map(|n| (id.clone(), *n)))
            .collect();
        let children = saved
            .filter(|shares| shares.iter().map(|(_, n)| u32::from(*n)).sum::<u32>() == 10_000)
            .unwrap_or_else(|| {
                let mut used = 0;
                let mut assigned = 0;
                split_children
                    .iter()
                    .map(|(id, r)| {
                        used += u32::from(if horizontal { r.width } else { r.height });
                        let edge = used * 10_000 / total.max(1);
                        let share = edge - assigned;
                        assigned = edge;
                        (id.clone(), share as u16)
                    })
                    .collect()
            });
        // Parent handles precede nested handles at intersecting boundaries.
        let insertion = 0;
        for (before, pair) in split_children.windows(2).enumerate() {
            let a = pair[0].1;
            let b = pair[1].1;
            let handle = if horizontal {
                Rect::new(
                    a.right().saturating_sub(1),
                    a.y.max(b.y),
                    b.x.saturating_sub(a.right()).saturating_add(2),
                    a.bottom().min(b.bottom()).saturating_sub(a.y.max(b.y)),
                )
            } else {
                Rect::new(
                    a.x.max(b.x),
                    a.bottom().saturating_sub(1),
                    a.right().min(b.right()).saturating_sub(a.x.max(b.x)),
                    b.y.saturating_sub(a.bottom()).saturating_add(2),
                )
            }
            .intersection(content)
            .intersection(clip);
            scene.splits.insert(
                insertion,
                SplitRegion {
                    id: built.node.attr("id").into(),
                    handle,
                    horizontal,
                    origin,
                    extent,
                    before,
                    children: children.clone(),
                },
            );
        }
    }
    Ok(())
}
