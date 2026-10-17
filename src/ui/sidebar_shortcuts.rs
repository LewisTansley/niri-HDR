//! Hover-revealed sidebar shortcuts dock (icons + add button + app picker).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use niri_config::{
    Color, Config, CornerRadius, GradientInterpolation, SidebarSide, SidebarShortcut,
};
use pangocairo::cairo::{self, Format, ImageSurface};
use pangocairo::pango::FontDescription;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::utils::{Logical, Point, Rectangle, Size, Transform};

use crate::animation::{Animation, Clock};
use crate::niri_render_elements;
use crate::render_helpers::border::BorderRenderElement;
use crate::render_helpers::primary_gpu_texture::PrimaryGpuTextureRenderElement;
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::texture::{TextureBuffer, TextureRenderElement};
use crate::utils::desktop::{
    find_desktop_entry, list_desktop_entries, load_icon_rgba, resolve_icon_path, DesktopEntry,
};
use crate::utils::to_physical_precise_round;

const DOCK_WIDTH: f64 = 56.;
const ICON_SIZE: f64 = 36.;
const CELL_GAP: f64 = 10.;
const PAD: f64 = 10.;
const FONT: &str = "sans 18px";
const NAME_FONT: &str = "sans 12px";

const PICKER_WIDTH: f64 = 280.;
const PICKER_ROW_H: f64 = 40.;
const PICKER_ICON: f64 = 28.;
const PICKER_MAX_H: f64 = 420.;
const PICKER_GAP: f64 = 8.;

niri_render_elements! {
    SidebarShortcutsRenderElement => {
        Border = BorderRenderElement,
        Texture = PrimaryGpuTextureRenderElement,
    }
}

/// Resolved panel chrome for the dock / app picker (matches sidebar theming).
#[derive(Debug, Clone, Copy)]
pub struct DockChrome {
    pub fill: Color,
    pub border_color: Color,
    pub border_width: f64,
    pub draw_border: bool,
    pub corner_radius: f64,
}

impl DockChrome {
    pub fn default_fallback() -> Self {
        Self {
            fill: Color::from_rgba8_unpremul(31, 31, 36, 235),
            border_color: Color::from_rgba8_unpremul(80, 80, 80, 255),
            border_width: 0.,
            draw_border: false,
            corner_radius: 0.,
        }
    }

    fn is_dark_fill(self) -> bool {
        let [r, g, b, _] = self.fill.to_array_unpremul();
        0.2126 * r + 0.7152 * g + 0.0722 * b < 0.55
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockHit {
    Shortcut(usize),
    Add,
    /// Index into the filtered picker entry list.
    PickerApp(usize),
}

struct SideDock {
    open: bool,
    open_anim: Option<Animation>,
    /// After handoff close, ignore edge until pointer leaves the strip.
    suppress_until_leave: bool,
    hover_enter: Option<Instant>,
}

impl SideDock {
    fn new() -> Self {
        Self {
            open: false,
            open_anim: None,
            suppress_until_leave: false,
            hover_enter: None,
        }
    }

    fn open_progress(&self) -> f64 {
        if let Some(anim) = &self.open_anim {
            anim.value()
        } else if self.open {
            1.
        } else {
            0.
        }
    }

    fn set_open(&mut self, open: bool, clock: &Clock, anim: niri_config::Animation) {
        let to = if open { 1. } else { 0. };
        if self.open == open
            && self
                .open_anim
                .as_ref()
                .is_none_or(|a| (a.to() - to).abs() < 1e-6)
        {
            return;
        }
        let from = self.open_progress();
        self.open = open;
        if (from - to).abs() < 1e-6 {
            self.open_anim = None;
            return;
        }
        self.open_anim = Some(Animation::new(clock.clone(), from, to, 0., anim));
    }

    fn advance(&mut self) {
        if let Some(anim) = &self.open_anim {
            if anim.is_done() {
                self.open_anim = None;
            }
        }
    }

    fn animations_ongoing(&self) -> bool {
        self.open_anim.is_some() || self.hover_enter.is_some()
    }
}

struct AppPicker {
    side: SidebarSide,
    entries: Vec<DesktopEntry>,
    scroll_y: f64,
    open: bool,
    open_anim: Option<Animation>,
}

impl AppPicker {
    fn open_progress(&self) -> f64 {
        if let Some(anim) = &self.open_anim {
            anim.value()
        } else if self.open {
            1.
        } else {
            0.
        }
    }

    fn set_open(&mut self, open: bool, clock: &Clock, anim: niri_config::Animation) {
        let to = if open { 1. } else { 0. };
        if self.open == open
            && self
                .open_anim
                .as_ref()
                .is_none_or(|a| (a.to() - to).abs() < 1e-6)
        {
            return;
        }
        let from = self.open_progress();
        self.open = open;
        if (from - to).abs() < 1e-6 {
            self.open_anim = None;
            return;
        }
        self.open_anim = Some(Animation::new(clock.clone(), from, to, 0., anim));
    }

    fn advance(&mut self) {
        if let Some(anim) = &self.open_anim {
            if anim.is_done() {
                self.open_anim = None;
            }
        }
        if !self.open && self.open_anim.is_none() {
            self.entries.clear();
            self.scroll_y = 0.;
        }
    }

    fn animations_ongoing(&self) -> bool {
        self.open_anim.is_some()
    }

    fn content_height(&self) -> f64 {
        let n = self.entries.len().max(1) as f64;
        (PAD * 2. + n * PICKER_ROW_H).min(PICKER_MAX_H)
    }

    fn max_scroll(&self) -> f64 {
        let n = self.entries.len() as f64;
        let content = PAD * 2. + n * PICKER_ROW_H;
        (content - self.content_height()).max(0.)
    }
}

pub struct SidebarShortcutsUi {
    left: SideDock,
    right: SideDock,
    picker: Option<AppPicker>,
    clock: Clock,
    config: Rc<RefCell<Config>>,
    icon_cache: RefCell<HashMap<(String, i32), Option<TextureBuffer<GlesTexture>>>>,
    /// Keyed by physical size and whether the fill is dark (for + contrast).
    add_cache: RefCell<HashMap<(i32, bool), TextureBuffer<GlesTexture>>>,
    name_cache: RefCell<HashMap<(String, i32), TextureBuffer<GlesTexture>>>,
}

impl SidebarShortcutsUi {
    pub fn new(clock: Clock, config: Rc<RefCell<Config>>) -> Self {
        Self {
            left: SideDock::new(),
            right: SideDock::new(),
            picker: None,
            clock,
            config,
            icon_cache: RefCell::new(HashMap::new()),
            add_cache: RefCell::new(HashMap::new()),
            name_cache: RefCell::new(HashMap::new()),
        }
    }

    fn side_mut(&mut self, side: SidebarSide) -> &mut SideDock {
        match side {
            SidebarSide::Left => &mut self.left,
            SidebarSide::Right => &mut self.right,
        }
    }

    fn side(&self, side: SidebarSide) -> &SideDock {
        match side {
            SidebarSide::Left => &self.left,
            SidebarSide::Right => &self.right,
        }
    }

    pub fn open_progress(&self, side: SidebarSide) -> f64 {
        self.side(side).open_progress()
    }

    pub fn is_interactable(&self, side: SidebarSide) -> bool {
        self.side(side).open_progress() > 0.01 && !self.side(side).suppress_until_leave
    }

    pub fn picker_is_open(&self) -> bool {
        self.picker
            .as_ref()
            .is_some_and(|p| p.open || p.open_progress() > 0.01)
    }

    pub fn picker_side(&self) -> Option<SidebarSide> {
        self.picker.as_ref().map(|p| p.side)
    }

    pub fn set_open(&mut self, side: SidebarSide, open: bool) {
        let clock = self.clock.clone();
        let anim = self.config.borrow().animations.sidebar_open_close.0;
        self.side_mut(side).set_open(open, &clock, anim);
        if !open {
            self.close_picker();
        }
    }

    pub fn toggle(&mut self, side: SidebarSide) {
        let open = !self.side(side).open;
        self.set_open(side, open);
    }

    /// Close dock as part of handing off to the full sidebar.
    pub fn handoff_close(&mut self, side: SidebarSide) {
        self.close_picker();
        self.set_open(side, false);
        self.side_mut(side).suppress_until_leave = true;
        self.side_mut(side).hover_enter = None;
    }

    pub fn open_picker(&mut self, side: SidebarSide, pinned_app_ids: &[String]) {
        let mut entries = list_desktop_entries();
        entries.retain(|e| !pinned_app_ids.iter().any(|id| id == &e.app_id));

        let clock = self.clock.clone();
        let anim = self.config.borrow().animations.sidebar_open_close.0;

        // Ensure dock stays open.
        self.side_mut(side).set_open(true, &clock, anim);

        let mut picker = AppPicker {
            side,
            entries,
            scroll_y: 0.,
            open: false,
            open_anim: None,
        };
        picker.set_open(true, &clock, anim);
        self.picker = Some(picker);
    }

    pub fn close_picker(&mut self) {
        let Some(picker) = self.picker.as_mut() else {
            return;
        };
        let clock = self.clock.clone();
        let anim = self.config.borrow().animations.sidebar_open_close.0;
        picker.set_open(false, &clock, anim);
    }

    pub fn advance_animations(&mut self) {
        self.left.advance();
        self.right.advance();
        if let Some(picker) = &mut self.picker {
            picker.advance();
            if !picker.open && picker.open_anim.is_none() {
                self.picker = None;
            }
        }
    }

    pub fn are_animations_ongoing(&self) -> bool {
        self.left.animations_ongoing()
            || self.right.animations_ongoing()
            || self
                .picker
                .as_ref()
                .is_some_and(AppPicker::animations_ongoing)
    }

    /// Update hover state for a side. Returns true if a redraw is needed.
    pub fn pointer_edge(
        &mut self,
        side: SidebarSide,
        in_strip: bool,
        sidebar_fully_visible: bool,
        delay: Duration,
    ) -> bool {
        if self.side(side).suppress_until_leave {
            if !in_strip {
                self.side_mut(side).suppress_until_leave = false;
            } else {
                return false;
            }
        }

        let clock = self.clock.clone();
        let anim = self.config.borrow().animations.sidebar_open_close.0;

        // Keep dock open while its picker is visible.
        let picker_keeps_open = self
            .picker
            .as_ref()
            .is_some_and(|p| p.side == side && (p.open || p.open_progress() > 0.01));

        if sidebar_fully_visible {
            let dock = self.side_mut(side);
            if dock.open || dock.open_progress() > 0. {
                dock.hover_enter = None;
                dock.set_open(false, &clock, anim);
                self.close_picker();
                return true;
            }
            dock.hover_enter = None;
            return false;
        }

        if in_strip || picker_keeps_open {
            if self.side(side).open {
                return false;
            }
            if self.side(side).hover_enter.is_none() {
                self.side_mut(side).hover_enter = Some(Instant::now());
                return true;
            }
            if self.side(side).hover_enter.unwrap().elapsed() >= delay {
                let dock = self.side_mut(side);
                dock.hover_enter = None;
                dock.set_open(true, &clock, anim);
                return true;
            }
            true
        } else {
            let mut changed = false;
            let dock = self.side_mut(side);
            if dock.hover_enter.take().is_some() {
                changed = true;
            }
            if dock.open || dock.open_progress() > 0. {
                dock.set_open(false, &clock, anim);
                changed = true;
            }
            if self
                .picker
                .as_ref()
                .is_some_and(|p| p.side == side && (p.open || p.open_progress() > 0.))
            {
                self.close_picker();
                changed = true;
            }
            changed
        }
    }

    pub fn dock_width(&self) -> f64 {
        DOCK_WIDTH
    }

    pub fn panel_rect(
        &self,
        side: SidebarSide,
        parent_area: Rectangle<f64, Logical>,
        shortcut_count: usize,
    ) -> Rectangle<f64, Logical> {
        let progress = self.open_progress(side).clamp(0., 1.);
        let cells = shortcut_count + 1; // + Add
        let height =
            PAD * 2. + cells as f64 * ICON_SIZE + (cells.saturating_sub(1) as f64) * CELL_GAP;
        let y = parent_area.loc.y + (parent_area.size.h - height).max(0.) / 2.;
        let slide = (1. - progress) * DOCK_WIDTH;
        let x = match side {
            SidebarSide::Left => parent_area.loc.x - slide,
            SidebarSide::Right => parent_area.loc.x + parent_area.size.w - DOCK_WIDTH + slide,
        };
        Rectangle::new(Point::from((x, y)), Size::from((DOCK_WIDTH, height)))
    }

    pub fn picker_rect(
        &self,
        parent_area: Rectangle<f64, Logical>,
        shortcut_count: usize,
    ) -> Option<Rectangle<f64, Logical>> {
        let picker = self.picker.as_ref()?;
        let progress = picker.open_progress().clamp(0., 1.);
        if progress <= 0.001 {
            return None;
        }
        let dock = self.panel_rect(picker.side, parent_area, shortcut_count);
        let height = picker.content_height();
        let y = parent_area.loc.y + (parent_area.size.h - height).max(0.) / 2.;
        let slide = (1. - progress) * PICKER_WIDTH;
        let x = match picker.side {
            SidebarSide::Left => dock.loc.x + dock.size.w + PICKER_GAP - slide,
            SidebarSide::Right => dock.loc.x - PICKER_GAP - PICKER_WIDTH + slide,
        };
        Some(Rectangle::new(
            Point::from((x, y)),
            Size::from((PICKER_WIDTH, height)),
        ))
    }

    pub fn hit(
        &self,
        side: SidebarSide,
        pos: Point<f64, Logical>,
        parent_area: Rectangle<f64, Logical>,
        shortcut_count: usize,
    ) -> Option<DockHit> {
        // Prefer picker hits when open on this side.
        if let Some(hit) = self.hit_picker(pos, parent_area, shortcut_count) {
            if self.picker.as_ref().is_some_and(|p| p.side == side) {
                return Some(hit);
            }
        }

        if !self.is_interactable(side) {
            return None;
        }
        let panel = self.panel_rect(side, parent_area, shortcut_count);
        if !panel.contains(pos) {
            return None;
        }
        let rel_y = pos.y - panel.loc.y - PAD;
        if rel_y < 0. {
            return None;
        }
        let stride = ICON_SIZE + CELL_GAP;
        let idx = (rel_y / stride).floor() as isize;
        if idx < 0 {
            return None;
        }
        let idx = idx as usize;
        let within = rel_y - idx as f64 * stride;
        if within > ICON_SIZE {
            return None;
        }
        if idx < shortcut_count {
            Some(DockHit::Shortcut(idx))
        } else if idx == shortcut_count {
            Some(DockHit::Add)
        } else {
            None
        }
    }

    pub fn hit_picker(
        &self,
        pos: Point<f64, Logical>,
        parent_area: Rectangle<f64, Logical>,
        shortcut_count: usize,
    ) -> Option<DockHit> {
        let picker = self.picker.as_ref()?;
        if picker.open_progress() <= 0.01 {
            return None;
        }
        let panel = self.picker_rect(parent_area, shortcut_count)?;
        if !panel.contains(pos) {
            return None;
        }
        let rel_y = pos.y - panel.loc.y - PAD + picker.scroll_y;
        if rel_y < 0. {
            return None;
        }
        let idx = (rel_y / PICKER_ROW_H).floor() as isize;
        if idx < 0 {
            return None;
        }
        let idx = idx as usize;
        if idx >= picker.entries.len() {
            return None;
        }
        Some(DockHit::PickerApp(idx))
    }

    pub fn scroll_picker(&mut self, delta: f64) -> bool {
        let Some(picker) = self.picker.as_mut() else {
            return false;
        };
        if !picker.open && picker.open_progress() <= 0.01 {
            return false;
        }
        let before = picker.scroll_y;
        picker.scroll_y = (picker.scroll_y + delta).clamp(0., picker.max_scroll());
        (picker.scroll_y - before).abs() > 0.01
    }

    /// Take a picker entry by index and close the picker. Returns the entry if valid.
    pub fn take_picker_entry(&mut self, idx: usize) -> Option<DesktopEntry> {
        let picker = self.picker.as_ref()?;
        let entry = picker.entries.get(idx)?.clone();
        self.close_picker();
        Some(entry)
    }

    pub fn contains_pointer(
        &self,
        side: SidebarSide,
        pos: Point<f64, Logical>,
        parent_area: Rectangle<f64, Logical>,
        shortcut_count: usize,
    ) -> bool {
        if self
            .panel_rect(side, parent_area, shortcut_count)
            .contains(pos)
        {
            return true;
        }
        if self.picker.as_ref().is_some_and(|p| p.side == side) {
            if let Some(pr) = self.picker_rect(parent_area, shortcut_count) {
                return pr.contains(pos);
            }
        }
        false
    }

    pub fn render(
        &self,
        renderer: &mut impl NiriRenderer,
        side: SidebarSide,
        parent_area: Rectangle<f64, Logical>,
        shortcuts: &[SidebarShortcut],
        scale: f64,
        chrome: DockChrome,
        push: &mut dyn FnMut(SidebarShortcutsRenderElement),
    ) {
        let progress = self.open_progress(side);
        if progress <= 0.001 {
            return;
        }

        let panel = self.panel_rect(side, parent_area, shortcuts.len());
        let alpha = progress as f32;

        let icon_px = to_physical_precise_round(scale, ICON_SIZE);
        let gles = renderer.as_gles_renderer();
        // Inset content slightly when corners are rounded so icons clear the curve.
        let inset = (chrome.corner_radius * 0.35).clamp(0., 8.);
        let mut y = panel.loc.y + PAD.max(inset);

        // Textures first (first-pushed = on top), then chrome behind.
        for shortcut in shortcuts {
            let loc = Point::from((panel.loc.x + (DOCK_WIDTH - ICON_SIZE) / 2., y));
            if let Some(tex) =
                self.icon_texture(gles, &shortcut.app_id, shortcut.title.as_deref(), icon_px, scale)
            {
                let elem = TextureRenderElement::from_texture_buffer(
                    tex,
                    loc,
                    alpha,
                    None,
                    Some(Size::from((ICON_SIZE, ICON_SIZE))),
                    Kind::Unspecified,
                );
                push(PrimaryGpuTextureRenderElement(elem).into());
            }
            y += ICON_SIZE + CELL_GAP;
        }

        let add_loc = Point::from((panel.loc.x + (DOCK_WIDTH - ICON_SIZE) / 2., y));
        if let Some(tex) = self.add_texture(gles, icon_px, scale, chrome.is_dark_fill()) {
            let elem = TextureRenderElement::from_texture_buffer(
                tex,
                add_loc,
                alpha,
                None,
                Some(Size::from((ICON_SIZE, ICON_SIZE))),
                Kind::Unspecified,
            );
            push(PrimaryGpuTextureRenderElement(elem).into());
        }

        push_panel_chrome(renderer, panel, chrome, alpha, scale, push);

        // App picker for this side.
        if let Some(picker) = &self.picker {
            if picker.side == side {
                self.render_picker(
                    renderer,
                    picker,
                    parent_area,
                    shortcuts.len(),
                    scale,
                    chrome,
                    push,
                );
            }
        }
    }

    fn render_picker(
        &self,
        renderer: &mut impl NiriRenderer,
        picker: &AppPicker,
        parent_area: Rectangle<f64, Logical>,
        shortcut_count: usize,
        scale: f64,
        chrome: DockChrome,
        push: &mut dyn FnMut(SidebarShortcutsRenderElement),
    ) {
        let progress = picker.open_progress();
        if progress <= 0.001 {
            return;
        }
        let Some(panel) = self.picker_rect(parent_area, shortcut_count) else {
            return;
        };
        let alpha = progress as f32;
        let inset = (chrome.corner_radius * 0.35).clamp(0., 8.);

        let gles = renderer.as_gles_renderer();
        let icon_px = to_physical_precise_round(scale, PICKER_ICON);
        let first = (picker.scroll_y / PICKER_ROW_H).floor() as usize;
        let visible = (panel.size.h / PICKER_ROW_H).ceil() as usize + 1;
        let max_text_w = (panel.size.w - PAD * 2. - PICKER_ICON - 10.).max(8.);

        // Textures first, then chrome.
        for (i, entry) in picker
            .entries
            .iter()
            .enumerate()
            .skip(first)
            .take(visible)
        {
            let row_y = panel.loc.y + PAD + i as f64 * PICKER_ROW_H - picker.scroll_y;
            // Skip rows that would paint outside the panel box.
            if row_y < panel.loc.y + inset - 0.5
                || row_y + PICKER_ROW_H > panel.loc.y + panel.size.h - inset + 0.5
            {
                continue;
            }

            let icon_loc = Point::from((
                panel.loc.x + PAD,
                row_y + (PICKER_ROW_H - PICKER_ICON) / 2.,
            ));
            if let Some(tex) =
                self.icon_texture(gles, &entry.app_id, entry.name.as_deref(), icon_px, scale)
            {
                let elem = TextureRenderElement::from_texture_buffer(
                    tex,
                    icon_loc,
                    alpha,
                    None,
                    Some(Size::from((PICKER_ICON, PICKER_ICON))),
                    Kind::Unspecified,
                );
                push(PrimaryGpuTextureRenderElement(elem).into());
            }

            let label = entry.name.as_deref().unwrap_or(entry.app_id.as_str());
            if let Some(tex) = self.name_texture(gles, label, scale, chrome.is_dark_fill()) {
                let text_size = tex.logical_size();
                let text_h = text_size.h;
                let draw_w = text_size.w.min(max_text_w);
                let text_loc = Point::from((
                    panel.loc.x + PAD + PICKER_ICON + 10.,
                    row_y + (PICKER_ROW_H - text_h) / 2.,
                ));
                let (src, size) = if text_size.w > max_text_w {
                    (
                        Some(Rectangle::new(
                            Point::from((0., 0.)),
                            Size::from((max_text_w, text_h)),
                        )),
                        Some(Size::from((draw_w, text_h))),
                    )
                } else {
                    (None, None)
                };
                let elem = TextureRenderElement::from_texture_buffer(
                    tex,
                    text_loc,
                    alpha,
                    src,
                    size,
                    Kind::Unspecified,
                );
                push(PrimaryGpuTextureRenderElement(elem).into());
            }
        }

        push_panel_chrome(renderer, panel, chrome, alpha, scale, push);
    }

    fn icon_texture(
        &self,
        renderer: &mut GlesRenderer,
        app_id: &str,
        title: Option<&str>,
        icon_px: i32,
        scale: f64,
    ) -> Option<TextureBuffer<GlesTexture>> {
        let key = (app_id.to_owned(), icon_px);
        if let Some(cached) = self.icon_cache.borrow().get(&key) {
            return cached.clone();
        }

        let mut buffer = None;
        if let Some(entry) = find_desktop_entry(app_id) {
            if let Some(icon_name) = entry.icon.as_deref() {
                if let Some(path) = resolve_icon_path(icon_name, icon_px as u32) {
                    if let Ok((w, h, rgba)) = load_icon_rgba(&path) {
                        let mut argb = Vec::with_capacity(rgba.len());
                        for px in rgba.chunks(4) {
                            argb.push(px[2]);
                            argb.push(px[1]);
                            argb.push(px[0]);
                            argb.push(px[3]);
                        }
                        buffer = TextureBuffer::from_memory(
                            renderer,
                            &argb,
                            Fourcc::Argb8888,
                            (w as i32, h as i32),
                            false,
                            scale,
                            Transform::Normal,
                            Vec::new(),
                        )
                        .ok();
                    }
                }
            }
        }

        if buffer.is_none() {
            let ch = title
                .or(Some(app_id))
                .and_then(|s| s.chars().next())
                .unwrap_or('?')
                .to_ascii_uppercase();
            buffer = render_letter(renderer, ch, icon_px, scale).ok();
        }

        self.icon_cache.borrow_mut().insert(key, buffer.clone());
        buffer
    }

    fn add_texture(
        &self,
        renderer: &mut GlesRenderer,
        icon_px: i32,
        scale: f64,
        dark_fill: bool,
    ) -> Option<TextureBuffer<GlesTexture>> {
        let key = (icon_px, dark_fill);
        if let Some(buf) = self.add_cache.borrow().get(&key) {
            return Some(buf.clone());
        }
        let fg = if dark_fill {
            (0.95, 0.95, 0.97)
        } else {
            (0.12, 0.12, 0.14)
        };
        let buf = render_add_glyph(renderer, icon_px, scale, fg).ok()?;
        self.add_cache.borrow_mut().insert(key, buf.clone());
        Some(buf)
    }

    fn name_texture(
        &self,
        renderer: &mut GlesRenderer,
        text: &str,
        scale: f64,
        dark_fill: bool,
    ) -> Option<TextureBuffer<GlesTexture>> {
        let key = (
            format!("{}:{}", if dark_fill { 'd' } else { 'l' }, text),
            to_physical_precise_round(scale, 12.),
        );
        if let Some(buf) = self.name_cache.borrow().get(&key) {
            return Some(buf.clone());
        }
        let fg = if dark_fill {
            (0.92, 0.92, 0.94)
        } else {
            (0.12, 0.12, 0.14)
        };
        let buf = render_text(renderer, text, scale, fg).ok()?;
        self.name_cache.borrow_mut().insert(key, buf.clone());
        Some(buf)
    }
}

fn push_panel_chrome(
    renderer: &mut impl NiriRenderer,
    panel: Rectangle<f64, Logical>,
    chrome: DockChrome,
    alpha: f32,
    scale: f64,
    push: &mut dyn FnMut(SidebarShortcutsRenderElement),
) {
    if !BorderRenderElement::has_shader(renderer) {
        return;
    }

    let radius = CornerRadius::from(chrome.corner_radius as f32)
        .fit_to(panel.size.w as f32, panel.size.h as f32);
    let geo = Rectangle::from_size(panel.size);

    // Border ring above fill (still behind textures since those were pushed first).
    if chrome.draw_border && chrome.border_width > 0. {
        let border = chrome.border_color * alpha;
        let elem = BorderRenderElement::new(
            panel.size,
            geo,
            GradientInterpolation::default(),
            border,
            border,
            0.,
            geo,
            chrome.border_width as f32,
            radius,
            scale as f32,
            1.,
        )
        .with_location(panel.loc);
        push(elem.into());
    }

    let fill = chrome.fill * alpha;
    let elem = BorderRenderElement::new(
        panel.size,
        geo,
        GradientInterpolation::default(),
        fill,
        fill,
        0.,
        geo,
        0.,
        radius,
        scale as f32,
        1.,
    )
    .with_location(panel.loc);
    push(elem.into());
}

fn render_letter(
    renderer: &mut GlesRenderer,
    ch: char,
    size_px: i32,
    scale: f64,
) -> anyhow::Result<TextureBuffer<GlesTexture>> {
    let surface = ImageSurface::create(Format::ARgb32, size_px, size_px)?;
    let cr = cairo::Context::new(&surface)?;
    cr.set_source_rgb(0.22, 0.22, 0.26);
    cr.paint()?;
    cr.set_source_rgb(0.9, 0.9, 0.92);

    let mut font = FontDescription::from_string(FONT);
    font.set_absolute_size(to_physical_precise_round(scale, font.size()));
    let layout = pangocairo::functions::create_layout(&cr);
    layout.context().set_round_glyph_positions(false);
    layout.set_font_description(Some(&font));
    layout.set_text(&ch.to_string());
    let (tw, th) = layout.pixel_size();
    cr.move_to((size_px - tw) as f64 / 2., (size_px - th) as f64 / 2.);
    pangocairo::functions::show_layout(&cr, &layout);
    drop(cr);

    let data = surface.take_data().unwrap();
    TextureBuffer::from_memory(
        renderer,
        &data,
        Fourcc::Argb8888,
        (size_px, size_px),
        false,
        scale,
        Transform::Normal,
        Vec::new(),
    )
    .map_err(|e| anyhow::anyhow!("texture: {e:?}"))
}

fn render_add_glyph(
    renderer: &mut GlesRenderer,
    size_px: i32,
    scale: f64,
    fg: (f64, f64, f64),
) -> anyhow::Result<TextureBuffer<GlesTexture>> {
    let surface = ImageSurface::create(Format::ARgb32, size_px, size_px)?;
    let cr = cairo::Context::new(&surface)?;
    // Transparent so the rounded panel shows through.
    cr.set_source_rgba(0., 0., 0., 0.);
    cr.paint()?;
    cr.set_source_rgb(fg.0, fg.1, fg.2);

    let mut font = FontDescription::from_string(FONT);
    font.set_absolute_size(to_physical_precise_round(scale, font.size()));
    let layout = pangocairo::functions::create_layout(&cr);
    layout.context().set_round_glyph_positions(false);
    layout.set_font_description(Some(&font));
    layout.set_text("+");
    let (tw, th) = layout.pixel_size();
    cr.move_to((size_px - tw) as f64 / 2., (size_px - th) as f64 / 2.);
    pangocairo::functions::show_layout(&cr, &layout);
    drop(cr);

    let data = surface.take_data().unwrap();
    TextureBuffer::from_memory(
        renderer,
        &data,
        Fourcc::Argb8888,
        (size_px, size_px),
        false,
        scale,
        Transform::Normal,
        Vec::new(),
    )
    .map_err(|e| anyhow::anyhow!("texture: {e:?}"))
}

fn render_text(
    renderer: &mut GlesRenderer,
    text: &str,
    scale: f64,
    fg: (f64, f64, f64),
) -> anyhow::Result<TextureBuffer<GlesTexture>> {
    let mut font = FontDescription::from_string(NAME_FONT);
    font.set_absolute_size(to_physical_precise_round(scale, font.size()));

    let surface = ImageSurface::create(Format::ARgb32, 0, 0)?;
    let cr = cairo::Context::new(&surface)?;
    let layout = pangocairo::functions::create_layout(&cr);
    layout.context().set_round_glyph_positions(false);
    layout.set_font_description(Some(&font));
    // Truncate long names.
    let display = if text.chars().count() > 32 {
        let t: String = text.chars().take(31).collect();
        format!("{t}…")
    } else {
        text.to_owned()
    };
    layout.set_text(&display);
    let (tw, th) = layout.pixel_size();
    drop(cr);

    let w = tw.max(1);
    let h = th.max(1);
    let surface = ImageSurface::create(Format::ARgb32, w, h)?;
    let cr = cairo::Context::new(&surface)?;
    cr.set_source_rgba(0., 0., 0., 0.);
    cr.paint()?;
    cr.set_source_rgb(fg.0, fg.1, fg.2);
    let layout = pangocairo::functions::create_layout(&cr);
    layout.context().set_round_glyph_positions(false);
    layout.set_font_description(Some(&font));
    layout.set_text(&display);
    cr.move_to(0., 0.);
    pangocairo::functions::show_layout(&cr, &layout);
    drop(cr);

    let data = surface.take_data().unwrap();
    TextureBuffer::from_memory(
        renderer,
        &data,
        Fourcc::Argb8888,
        (w, h),
        false,
        scale,
        Transform::Normal,
        Vec::new(),
    )
    .map_err(|e| anyhow::anyhow!("texture: {e:?}"))
}
