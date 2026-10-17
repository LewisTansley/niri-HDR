//! Left/right sidebars that stack windows and can overlay or reserve tiling space.

use std::rc::Rc;

use niri_config::{Color, PresetSize, SidebarMode, SidebarSide};
use niri_ipc::SizeChange;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::output::Output;
use smithay::utils::{Logical, Point, Rectangle, Scale, Serial, Size};

use super::closing_window::{ClosingWindow, ClosingWindowRenderElement};
use super::scrolling::ColumnWidth;
use super::tile::{Tile, TileRenderElement, TileRenderSnapshot};
use super::{HitType, LayoutElement, Options, RemovedTile};
use crate::animation::{Animation, Clock};
use crate::niri_render_elements;
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::sidebar_chrome::SidebarChromeRenderElement;
use crate::render_helpers::xray::{Xray, XrayPos};
use crate::render_helpers::RenderCtx;
use crate::utils::dms_theme::DmsSidebarTheme;
use crate::utils::transaction::{Transaction, TransactionBlocker};
use crate::utils::ResizeEdge;

/// Moves up to this logical pixel distance are not animated during stack reflow.
const REFLOW_ANIMATION_THRESHOLD_SQ: f64 = 10. * 10.;

niri_render_elements! {
    SidebarRenderElement<R> => {
        Tile = TileRenderElement<R>,
        ClosingWindow = ClosingWindowRenderElement,
        Chrome = crate::render_helpers::sidebar_chrome::SidebarChromeRenderElement,
    }
}

/// A left or right sidebar holding a vertical stack of windows.
#[derive(Debug)]
pub struct Sidebar<W: LayoutElement> {
    side: SidebarSide,
    mode: SidebarMode,
    tiles: Vec<Tile<W>>,
    /// Index of the focused window within the stack.
    active_idx: usize,
    /// Target width in logical pixels.
    width: f64,
    /// In-flight width animation (from → target `width`).
    width_anim: Option<Animation>,
    preset_width_idx: Option<usize>,
    /// Logical show/hide target.
    visible: bool,
    /// Hidden by auto-hide (maximized/fullscreen); restore when that ends.
    auto_hidden: bool,
    /// User forced the sidebar open while auto-hide would otherwise apply.
    auto_hide_override: bool,
    /// In-flight open (1) / close (0) animation.
    open_anim: Option<Animation>,
    focused: bool,
    closing_windows: Vec<ClosingWindow>,
    view_size: Size<f64, Logical>,
    /// Parent area (layer-shell non-exclusive zone) used for proportion resolution.
    parent_area: Rectangle<f64, Logical>,
    scale: f64,
    clock: Clock,
    options: Rc<Options>,
}

impl<W: LayoutElement> Sidebar<W> {
    pub fn new(
        side: SidebarSide,
        mode: SidebarMode,
        view_size: Size<f64, Logical>,
        parent_area: Rectangle<f64, Logical>,
        scale: f64,
        clock: Clock,
        options: Rc<Options>,
    ) -> Self {
        let width = resolve_preset_width(
            options
                .layout
                .sidebar
                .preset_widths
                .first()
                .copied()
                .unwrap_or(PresetSize::Proportion(0.25)),
            parent_area.size.w,
        );
        Self {
            side,
            mode,
            tiles: Vec::new(),
            active_idx: 0,
            width,
            width_anim: None,
            preset_width_idx: Some(0),
            visible: true,
            auto_hidden: false,
            auto_hide_override: false,
            open_anim: None,
            focused: false,
            closing_windows: Vec::new(),
            view_size,
            parent_area,
            scale,
            clock,
            options,
        }
    }

    pub fn side(&self) -> SidebarSide {
        self.side
    }

    pub fn mode(&self) -> SidebarMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: SidebarMode) {
        self.mode = mode;
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// Whether this sidebar can be discarded (no tiles and no closing animations).
    pub fn can_discard(&self) -> bool {
        self.tiles.is_empty() && self.closing_windows.is_empty()
    }

    /// Logically open and non-empty (focus / actions).
    pub fn is_visible(&self) -> bool {
        self.visible && !self.tiles.is_empty()
    }

    /// Whether the panel currently occupies space or paints (including mid close/open anim).
    pub fn is_rendered(&self) -> bool {
        (!self.tiles.is_empty() && self.open_progress() > 0.) || !self.closing_windows.is_empty()
    }

    pub fn is_focused(&self) -> bool {
        self.focused && self.is_visible()
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    pub fn open_progress(&self) -> f64 {
        if let Some(anim) = &self.open_anim {
            anim.value()
        } else if self.visible && !self.tiles.is_empty() {
            1.
        } else if self.visible && self.tiles.is_empty() {
            // Empty but marked visible (just created): treat as open for when a window arrives.
            1.
        } else {
            0.
        }
    }

    pub fn display_width(&self) -> f64 {
        self.width_anim
            .as_ref()
            .map(|a| a.value())
            .unwrap_or(self.width)
    }

    pub fn set_visible(&mut self, visible: bool) {
        let to = if visible { 1. } else { 0. };
        if self.visible == visible {
            // Already targeting this state; only restart if mid-animation the other way.
            if self.open_anim.as_ref().is_none_or(|a| (a.to() - to).abs() < 1e-6) {
                return;
            }
        }
        // Read progress before flipping `visible` — open_progress() uses that flag when idle.
        let from = self.open_progress();
        self.visible = visible;
        if (from - to).abs() < 1e-6 {
            self.open_anim = None;
            return;
        }
        self.open_anim = Some(Animation::new(
            self.clock.clone(),
            from,
            to,
            0.,
            self.options.animations.sidebar_open_close.0,
        ));
    }

    /// Show the sidebar from a user action (toggle / focus / add window).
    ///
    /// If the panel was auto-hidden for maximize/fullscreen, this holds it open until
    /// that condition ends.
    pub fn reveal_for_user(&mut self) {
        if self.auto_hidden {
            self.auto_hidden = false;
            self.auto_hide_override = true;
        }
        self.set_visible(true);
    }

    pub fn toggle_visible(&mut self) {
        if self.auto_hidden {
            self.reveal_for_user();
            return;
        }
        if self.visible {
            self.auto_hide_override = false;
            self.set_visible(false);
        } else {
            self.set_visible(true);
        }
    }

    /// Apply or clear maximize/fullscreen auto-hide for this sidebar.
    ///
    /// Returns `true` if visibility targeting changed.
    pub fn apply_auto_hide(&mut self, should_hide: bool) -> bool {
        if should_hide {
            if self.auto_hide_override {
                return false;
            }
            if self.visible {
                self.auto_hidden = true;
                self.set_visible(false);
                return true;
            }
            false
        } else {
            let mut changed = false;
            if self.auto_hide_override {
                self.auto_hide_override = false;
                changed = true;
            }
            if self.auto_hidden {
                self.auto_hidden = false;
                self.set_visible(true);
                changed = true;
            }
            changed
        }
    }

    pub fn width(&self) -> f64 {
        self.width
    }

    /// Reserved inset contributed to the tiling area (animated with open + width).
    pub fn reserved_inset(&self) -> f64 {
        if self.tiles.is_empty() || self.mode != SidebarMode::Reserve {
            return 0.;
        }
        self.display_width() * self.open_progress().clamp(0., 1.)
    }

    /// True when open/close or width animation should drive per-frame inset sync.
    pub fn needs_inset_sync(&self) -> bool {
        self.open_anim.is_some() || self.width_anim.is_some()
    }

    pub fn contains(&self, id: &W::Id) -> bool {
        self.tiles.iter().any(|t| t.window().id() == id)
    }

    pub fn windows(&self) -> impl Iterator<Item = &W> + '_ {
        self.tiles.iter().map(|t| t.window())
    }

    pub fn windows_mut(&mut self) -> impl Iterator<Item = &mut W> + '_ {
        self.tiles.iter_mut().map(|t| t.window_mut())
    }

    pub fn tiles(&self) -> impl Iterator<Item = &Tile<W>> + '_ {
        self.tiles.iter()
    }

    pub fn tiles_mut(&mut self) -> impl Iterator<Item = &mut Tile<W>> + '_ {
        self.tiles.iter_mut()
    }

    fn idx_of(&self, id: &W::Id) -> Option<usize> {
        self.tiles.iter().position(|t| t.window().id() == id)
    }

    pub fn active_window(&self) -> Option<&W> {
        self.tiles.get(self.active_idx).map(|t| t.window())
    }

    pub fn update_config(
        &mut self,
        view_size: Size<f64, Logical>,
        parent_area: Rectangle<f64, Logical>,
        scale: f64,
        options: Rc<Options>,
        mode: SidebarMode,
    ) {
        self.view_size = view_size;
        self.parent_area = parent_area;
        self.scale = scale;
        self.mode = mode;
        self.options = options.clone();

        // Clamp width to new parent area.
        self.width = self.width.clamp(32., parent_area.size.w * 0.8);
        // Drop in-flight width anim so display matches clamped target.
        self.width_anim = None;
        for tile in &mut self.tiles {
            tile.update_config(view_size, scale, options.clone());
        }
        self.relayout(false);
    }

    pub fn update_output(&mut self, output: &Output) {
        for tile in &mut self.tiles {
            tile.window().output_enter(output);
            tile.window().set_preferred_scale_transform(
                output.current_scale(),
                output.current_transform(),
            );
        }
    }

    pub fn push(&mut self, mut removed: RemovedTile<W>) {
        let prev_positions = self.tile_positions_by_id();

        removed.tile.stop_move_animations();
        removed.tile.window_mut().set_floating(false);
        removed
            .tile
            .update_config(self.view_size, self.scale, self.options.clone());
        self.tiles.push(removed.tile);
        self.active_idx = self.tiles.len() - 1;
        self.reveal_for_user();
        self.relayout(false);
        self.animate_reflow_from(prev_positions);
    }

    /// Window-geometry-relative target rect for popup unconstraining.
    pub fn popup_target_rect(&self, id: &W::Id) -> Option<Rectangle<f64, Logical>> {
        for (tile, pos) in self.tiles_with_positions() {
            if tile.window().id() == id {
                // Horizontally within the window; vertically within the sidebar panel.
                let width = tile.window_size().w;
                let height = self.parent_area.size.h;
                let mut target = Rectangle::from_size(Size::from((width, height)));
                target.loc.y += self.parent_area.loc.y;
                target.loc.y -= pos.y;
                target.loc.y -= tile.window_loc().y;
                return Some(target);
            }
        }
        None
    }

    pub fn remove(&mut self, id: &W::Id) -> Option<RemovedTile<W>> {
        let prev_positions = self.tile_positions_by_id();
        let idx = self.idx_of(id)?;
        let tile = self.tiles.remove(idx);
        if self.tiles.is_empty() {
            self.active_idx = 0;
            self.focused = false;
        } else if self.active_idx >= self.tiles.len() {
            self.active_idx = self.tiles.len() - 1;
        } else if idx < self.active_idx {
            self.active_idx -= 1;
        }
        self.relayout(false);
        self.animate_reflow_from(prev_positions);

        let width = ColumnWidth::Fixed(tile.tile_expected_or_current_size().w);
        Some(RemovedTile {
            tile,
            width,
            is_full_width: false,
            is_floating: false,
        })
    }

    pub fn remove_window(&mut self, id: &W::Id, _transaction: Transaction) -> Option<RemovedTile<W>> {
        self.remove(id)
    }

    pub fn take_all(&mut self) -> Vec<RemovedTile<W>> {
        let tiles = std::mem::take(&mut self.tiles);
        self.active_idx = 0;
        self.focused = false;
        self.width_anim = None;
        self.open_anim = None;
        tiles
            .into_iter()
            .map(|tile| {
                let width = ColumnWidth::Fixed(tile.tile_expected_or_current_size().w);
                RemovedTile {
                    tile,
                    width,
                    is_full_width: false,
                    is_floating: false,
                }
            })
            .collect()
    }

    pub fn focus_up(&mut self) -> bool {
        if self.active_idx > 0 {
            self.active_idx -= 1;
            true
        } else {
            false
        }
    }

    pub fn focus_down(&mut self) -> bool {
        if self.active_idx + 1 < self.tiles.len() {
            self.active_idx += 1;
            true
        } else {
            false
        }
    }

    pub fn move_up(&mut self) -> bool {
        if self.active_idx == 0 || self.tiles.is_empty() {
            return false;
        }

        let prev_positions = self.tile_positions_by_id();
        let new_idx = self.active_idx - 1;
        self.tiles.swap(self.active_idx, new_idx);
        self.active_idx = new_idx;
        self.relayout(false);
        self.animate_reflow_from(prev_positions);
        true
    }

    pub fn move_down(&mut self) -> bool {
        if self.active_idx + 1 >= self.tiles.len() {
            return false;
        }

        let prev_positions = self.tile_positions_by_id();
        let new_idx = self.active_idx + 1;
        self.tiles.swap(self.active_idx, new_idx);
        self.active_idx = new_idx;
        self.relayout(false);
        self.animate_reflow_from(prev_positions);
        true
    }

    /// Geometry of the active window relative to and clamped to the parent area.
    pub fn active_window_visual_rectangle(&self) -> Option<Rectangle<f64, Logical>> {
        let (tile, offset) = self.tiles_with_positions().nth(self.active_idx)?;

        let window_pos = offset + tile.window_loc();
        let window_size = tile.window_size();
        let window_rect = Rectangle::new(window_pos, window_size);

        self.parent_area.intersection(window_rect)
    }

    pub fn activate_window(&mut self, id: &W::Id) -> bool {
        let Some(idx) = self.idx_of(id) else {
            return false;
        };
        self.active_idx = idx;
        self.focused = true;
        self.reveal_for_user();
        true
    }

    pub fn update_window(&mut self, id: &W::Id, serial: Option<Serial>) -> bool {
        let Some(idx) = self.idx_of(id) else {
            return false;
        };
        let tile = &mut self.tiles[idx];
        if let Some(serial) = serial {
            tile.window_mut().on_commit(serial);
        }
        tile.update_window();
        true
    }

    pub fn toggle_width(&mut self, forward: bool) {
        let presets = &self.options.layout.sidebar.preset_widths;
        if presets.is_empty() {
            return;
        }
        let len = presets.len();
        let next = match self.preset_width_idx {
            Some(idx) if forward => (idx + 1) % len,
            Some(idx) => (idx + len - 1) % len,
            None => 0,
        };
        self.preset_width_idx = Some(next);
        let new_width = resolve_preset_width(presets[next], self.parent_area.size.w);
        self.set_width_animated(new_width);
    }

    pub fn set_width(&mut self, change: SizeChange) {
        let current = self.display_width();
        let available = self.parent_area.size.w;
        let new_width = match change {
            SizeChange::SetFixed(v) => v as f64,
            SizeChange::SetProportion(p) => available * (p / 100.),
            SizeChange::AdjustFixed(delta) => current + delta as f64,
            SizeChange::AdjustProportion(delta) => current + available * (delta / 100.),
        };
        let new_width = new_width.clamp(32., available * 0.8);
        self.preset_width_idx = None;
        self.set_width_animated(new_width);
    }

    fn set_width_animated(&mut self, new_width: f64) {
        let from = self.display_width();
        self.width = new_width;
        if (from - new_width).abs() < 1e-6 {
            self.width_anim = None;
            self.relayout(false);
            return;
        }
        self.width_anim = Some(Animation::new(
            self.clock.clone(),
            from,
            new_width,
            0.,
            self.options.animations.window_resize.anim,
        ));
        // Request final tile sizes once so Tile::ResizeAnimation can run; panel/inset
        // keep following display_width() via width_anim without per-frame configures.
        self.relayout(true);
    }

    /// Interactive edge resize: apply width immediately without animation.
    pub fn resize_by_edges(&mut self, edges: ResizeEdge, delta: Point<f64, Logical>) {
        let mut dw = 0.;
        match self.side {
            SidebarSide::Left => {
                if edges.contains(ResizeEdge::RIGHT) {
                    dw = delta.x;
                }
            }
            SidebarSide::Right => {
                if edges.contains(ResizeEdge::LEFT) {
                    dw = -delta.x;
                }
            }
        }
        if dw != 0. {
            let available = self.parent_area.size.w;
            let new_width = (self.display_width() + dw).clamp(32., available * 0.8);
            self.width = new_width;
            self.width_anim = None;
            self.preset_width_idx = None;
            self.relayout(false);
        }
    }

    fn relayout(&mut self, animate_tiles: bool) {
        if self.tiles.is_empty() {
            return;
        }

        let gaps = self.options.layout.gaps;
        let n = self.tiles.len();
        let area_h = self.parent_area.size.h;
        let total_gaps = gaps * (n as f64 + 1.);
        let tile_h = ((area_h - total_gaps) / n as f64).max(1.);
        // Discrete animated resize sizes to the target width; interactive / instant paths
        // follow the current display width.
        let panel_w = if animate_tiles {
            self.width
        } else {
            self.display_width()
        };
        let tile_w = (panel_w - gaps * 2.).max(1.);

        let size = Size::from((tile_w, tile_h));
        for tile in &mut self.tiles {
            tile.request_tile_size(size, animate_tiles, None);
        }
    }

    fn tile_positions_by_id(&self) -> Vec<(W::Id, Point<f64, Logical>)> {
        self.tiles_with_positions()
            .map(|(tile, pos)| (tile.window().id().clone(), pos))
            .collect()
    }

    fn animate_reflow_from(&mut self, prev: Vec<(W::Id, Point<f64, Logical>)>) {
        if prev.is_empty() {
            return;
        }
        let new_positions: Vec<_> = self
            .tiles_with_positions()
            .map(|(tile, pos)| (tile.window().id().clone(), pos))
            .collect();

        for (id, new_pos) in &new_positions {
            let Some((_, prev_pos)) = prev.iter().find(|(prev_id, _)| prev_id == id) else {
                continue;
            };
            let diff = *prev_pos - *new_pos;
            if diff.x * diff.x + diff.y * diff.y <= REFLOW_ANIMATION_THRESHOLD_SQ {
                continue;
            }
            if let Some(tile) = self.tiles.iter_mut().find(|t| t.window().id() == id) {
                tile.animate_move_from(diff);
            }
        }
    }

    /// Geometry of the sidebar panel within the parent/working area coordinates.
    pub fn panel_rect(&self) -> Rectangle<f64, Logical> {
        let width = self.display_width();
        let progress = self.open_progress().clamp(0., 1.);
        let slide = (1. - progress) * width;

        let x = match self.side {
            SidebarSide::Left => self.parent_area.loc.x - slide,
            SidebarSide::Right => {
                self.parent_area.loc.x + self.parent_area.size.w - width + slide
            }
        };
        Rectangle::new(
            Point::from((x, self.parent_area.loc.y)),
            Size::from((width, self.parent_area.size.h)),
        )
    }

    pub fn tiles_with_positions(&self) -> impl Iterator<Item = (&Tile<W>, Point<f64, Logical>)> + '_ {
        let gaps = self.options.layout.gaps;
        let n = self.tiles.len().max(1);
        let area_h = self.parent_area.size.h;
        let total_gaps = gaps * (n as f64 + 1.);
        let tile_h = ((area_h - total_gaps) / n as f64).max(1.);
        let panel = self.panel_rect();
        let scale = self.scale;

        self.tiles.iter().enumerate().map(move |(i, tile)| {
            let x = panel.loc.x + gaps;
            let y = panel.loc.y + gaps + i as f64 * (tile_h + gaps);
            let pos = Point::from((x, y));
            let pos = pos.to_physical_precise_round(scale).to_logical(scale);
            (tile, pos)
        })
    }

    pub fn tiles_with_render_positions(
        &self,
    ) -> impl Iterator<Item = (&Tile<W>, Point<f64, Logical>)> + '_ {
        self.tiles_with_positions()
            .map(|(tile, pos)| (tile, pos + tile.render_offset()))
    }

    pub fn update_render_elements(&mut self, is_active: bool) {
        if !self.is_rendered() {
            return;
        }
        let view_rect = Rectangle::from_size(self.view_size);
        let active_idx = self.active_idx;
        let positions: Vec<_> = self.tiles_with_positions().map(|(_, p)| p).collect();
        for (idx, tile) in self.tiles.iter_mut().enumerate() {
            let mut tile_view_rect = view_rect;
            if let Some(pos) = positions.get(idx) {
                tile_view_rect.loc -= *pos + tile.render_offset();
            }
            tile.update_render_elements(is_active && self.focused && idx == active_idx, tile_view_rect);
        }
    }

    pub fn advance_animations(&mut self) {
        for tile in &mut self.tiles {
            tile.advance_animations();
        }

        if let Some(anim) = &self.open_anim {
            if anim.is_done() {
                self.open_anim = None;
            }
        }

        if let Some(anim) = &self.width_anim {
            if anim.is_done() {
                self.width_anim = None;
                // Final relayout at exact target width (sizes already requested at anim start).
                self.relayout(false);
            }
            // While width_anim runs, do not re-request intermediate tile sizes — that
            // would spam configures and cancel Tile::ResizeAnimation. Panel geometry
            // still tracks display_width() via panel_rect() / reserved_inset().
        }

        self.closing_windows.retain_mut(|closing| {
            closing.advance_animations();
            closing.are_animations_ongoing()
        });
    }

    pub fn are_animations_ongoing(&self) -> bool {
        self.open_anim.is_some()
            || self.width_anim.is_some()
            || !self.closing_windows.is_empty()
            || self.tiles.iter().any(Tile::are_animations_ongoing)
    }

    pub fn start_open_animation(&mut self, id: &W::Id) -> bool {
        let Some(idx) = self.idx_of(id) else {
            return false;
        };
        self.tiles[idx].start_open_animation();
        true
    }

    pub fn store_unmap_snapshot_if_empty(
        &mut self,
        renderer: &mut GlesRenderer,
        xray: Option<&mut Xray>,
        xray_has_blocked_out_layers: bool,
        xray_pos: XrayPos,
        window: &W::Id,
    ) -> bool {
        let view_size = self.view_size;
        let positions: Vec<_> = self
            .tiles_with_render_positions()
            .map(|(tile, pos)| (tile.window().id().clone(), pos))
            .collect();
        let Some((_, tile_pos)) = positions.iter().find(|(id, _)| id == window) else {
            return false;
        };
        let tile_pos = *tile_pos;
        let Some(tile) = self.tiles.iter_mut().find(|t| t.window().id() == window) else {
            return false;
        };

        let view_pos = Point::from((-tile_pos.x, -tile_pos.y));
        let view_rect = Rectangle::new(view_pos, view_size);
        tile.update_render_elements(false, view_rect);
        let xray_pos = xray_pos.offset(tile_pos);
        tile.store_unmap_snapshot_if_empty(renderer, xray, xray_has_blocked_out_layers, xray_pos);
        true
    }

    pub fn start_close_animation_for_window(
        &mut self,
        renderer: &mut GlesRenderer,
        id: &W::Id,
        blocker: TransactionBlocker,
    ) -> bool {
        let Some(idx) = self.idx_of(id) else {
            return false;
        };
        let tile_pos = self
            .tiles_with_render_positions()
            .nth(idx)
            .map(|(_, pos)| pos)
            .unwrap();

        let tile = &mut self.tiles[idx];
        let Some(snapshot) = tile.take_unmap_snapshot() else {
            return false;
        };
        let tile_size = tile.tile_size();
        self.start_close_animation_for_tile(renderer, snapshot, tile_size, tile_pos, blocker);
        true
    }

    pub fn start_close_animation_for_tile(
        &mut self,
        renderer: &mut GlesRenderer,
        snapshot: TileRenderSnapshot,
        tile_size: Size<f64, Logical>,
        tile_pos: Point<f64, Logical>,
        blocker: TransactionBlocker,
    ) {
        let anim = Animation::new(
            self.clock.clone(),
            0.,
            1.,
            0.,
            self.options.animations.window_close.anim,
        );

        let blocker = if self.options.disable_transactions {
            TransactionBlocker::completed()
        } else {
            blocker
        };

        let scale = Scale::from(self.scale);
        match ClosingWindow::new(renderer, snapshot, scale, tile_size, tile_pos, blocker, anim) {
            Ok(closing) => self.closing_windows.push(closing),
            Err(err) => {
                warn!("error creating a closing window animation: {err:?}");
            }
        }
    }

    pub fn window_under(&self, pos: Point<f64, Logical>) -> Option<(&W, HitType)> {
        if !self.is_visible() {
            return None;
        }
        if !self.panel_rect().contains(pos) {
            return None;
        }
        for (tile, tile_pos) in self.tiles_with_positions() {
            if let Some(hit) = HitType::hit_tile(tile, tile_pos, pos) {
                return Some(hit);
            }
        }
        // Click on sidebar chrome activates the active window.
        self.active_window().map(|win| {
            (
                win,
                HitType::Activate {
                    is_tab_indicator: false,
                },
            )
        })
    }

    pub fn resize_edges_under(&self, pos: Point<f64, Logical>) -> Option<ResizeEdge> {
        if !self.is_visible() {
            return None;
        }
        let panel = self.panel_rect();
        if !panel.contains(pos) {
            return None;
        }
        const EDGE: f64 = 8.;
        match self.side {
            SidebarSide::Left => {
                if (panel.loc.x + panel.size.w - pos.x).abs() <= EDGE {
                    return Some(ResizeEdge::RIGHT);
                }
            }
            SidebarSide::Right => {
                if (pos.x - panel.loc.x).abs() <= EDGE {
                    return Some(ResizeEdge::LEFT);
                }
            }
        }
        None
    }

    pub fn render<R: NiriRenderer>(
        &self,
        mut ctx: RenderCtx<R>,
        xray_pos: XrayPos,
        push: &mut dyn FnMut(SidebarRenderElement<R>),
    ) {
        if !self.is_rendered() {
            return;
        }

        let scale = Scale::from(self.scale);
        let view_rect = Rectangle::from_size(self.view_size);
        for closing in self.closing_windows.iter().rev() {
            let elem = closing.render(ctx.as_gles(), view_rect, scale);
            push(elem.into());
        }

        if self.open_progress() <= 0. || self.tiles.is_empty() {
            return;
        }

        // Tiles first (higher in the stack), then chrome behind them.
        let active_idx = self.active_idx;
        for (idx, (tile, tile_pos)) in self.tiles_with_positions().enumerate() {
            let focus_ring = self.focused && idx == active_idx;
            let xray_pos = xray_pos.offset(tile_pos);
            tile.render(ctx.r(), tile_pos, xray_pos, focus_ring, &mut |elem| {
                push(elem.into());
            });
        }

        self.render_chrome(ctx.renderer, push);
    }

    fn render_chrome<R: NiriRenderer>(
        &self,
        renderer: &mut R,
        push: &mut dyn FnMut(SidebarRenderElement<R>),
    ) {
        let sb = &self.options.layout.sidebar;
        let dms = sb
            .use_dms_theme
            .then(|| self.options.dms_sidebar_theme.borrow().clone())
            .flatten();

        let (border_off, border_width, border_color, background, scoop_radius) =
            resolve_chrome(sb, dms.as_ref(), self.focused);

        let has_fill = background.is_some();
        let has_border = !border_off && border_width > 0.;
        if !has_fill && !has_border {
            return;
        }
        if !SidebarChromeRenderElement::has_shader(renderer) {
            return;
        }

        let panel = self.panel_rect();
        let scale = self.scale as f32;
        // Keep chrome fully opaque; open/close motion comes from panel_rect() sliding.
        let alpha = 1.;
        let scoop = scoop_radius as f32;
        let panel_w = panel.size.w as f32;

        // Extend chrome by R into the viewport so wingtips can frame the opening.
        let (chrome_size, chrome_loc) = if scoop > 0. {
            let mut size = panel.size;
            size.w += scoop_radius;
            let mut loc = panel.loc;
            if matches!(self.side, SidebarSide::Right) {
                loc.x -= scoop_radius;
            }
            (size, loc)
        } else {
            (panel.size, panel.loc)
        };

        // Single element when both fill and border are on (one outer AA edge).
        let elem = match (background, has_border) {
            (Some(bg), true) => SidebarChromeRenderElement::fill_and_border(
                chrome_size,
                panel_w,
                bg,
                border_color,
                self.side,
                scoop,
                border_width as f32,
                scale,
                alpha,
            ),
            (Some(bg), false) => SidebarChromeRenderElement::fill(
                chrome_size,
                panel_w,
                bg,
                self.side,
                scoop,
                scale,
                alpha,
            ),
            (None, true) => SidebarChromeRenderElement::border(
                chrome_size,
                panel_w,
                border_color,
                self.side,
                scoop,
                border_width as f32,
                scale,
                alpha,
            ),
            (None, false) => return,
        };
        push(elem.with_location(chrome_loc).into());
    }

    pub fn refresh(&mut self, is_active: bool) {
        let active_idx = self.active_idx;
        let focused = self.focused;
        let visible = self.is_visible();
        for (idx, tile) in self.tiles.iter_mut().enumerate() {
            let win = tile.window_mut();
            win.set_active_in_column(idx == active_idx);
            win.set_floating(false);
            win.set_activated(is_active && focused && visible && idx == active_idx);
            win.set_interactive_resize(None);
            win.set_bounds(self.view_size.to_i32_round());
            win.send_pending_configure();
            win.refresh();
        }
    }
}

fn resolve_preset_width(preset: PresetSize, available: f64) -> f64 {
    match preset {
        PresetSize::Proportion(p) => (available * p).clamp(32., available * 0.8),
        PresetSize::Fixed(px) => (px as f64).clamp(32., available * 0.8),
    }
}

/// Resolve sidebar (and shortcuts dock) chrome: `(border_off, width, border_color, fill, radius)`.
pub(crate) fn resolve_chrome(
    sb: &niri_config::Sidebar,
    dms: Option<&DmsSidebarTheme>,
    focused: bool,
) -> (bool, f64, Color, Option<Color>, f64) {
    let border = &sb.border;
    let (active, inactive, dms_bg, scoop_from_dms) = match dms {
        Some(dms) => (
            dms.active_color,
            dms.inactive_color,
            Some(dms.background_color),
            dms.corner_radius,
        ),
        None => (
            border.active_color,
            border.inactive_color,
            None,
            0.,
        ),
    };

    let border_color = if focused { active } else { inactive };
    let scoop_radius = match sb.corner_radius {
        Some(r) => r,
        None if dms.is_some() => scoop_from_dms,
        None => 0.,
    };

    let background = border
        .background_color
        .or_else(|| {
            // With DMS and an enabled border, fill the strip so scoops read clearly.
            if !border.off {
                dms_bg
            } else {
                None
            }
        });

    (border.off, border.width, border_color, background, scoop_radius)
}
