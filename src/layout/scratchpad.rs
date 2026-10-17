//! i3-like scratchpad: park windows and overlay the most-recent one on the active workspace.

use std::rc::Rc;

use smithay::output::Output;
use smithay::utils::{Logical, Point, Rectangle, Serial, Size};

use super::tile::{Tile, TileRenderElement};
use super::{HitType, LayoutElement, Options, RemovedTile, SizeFrac};
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::xray::XrayPos;
use crate::render_helpers::RenderCtx;
use crate::utils::transaction::Transaction;

/// Per-monitor scratchpad store.
#[derive(Debug)]
pub struct Scratchpad<W: LayoutElement> {
    /// Parked tiles; most recently scratched is last.
    tiles: Vec<Tile<W>>,
    /// Cached floating positions (SizeFrac of working area), parallel to `tiles`.
    positions: Vec<Point<f64, SizeFrac>>,
    /// Which parked window is currently shown as an overlay, if any.
    shown: Option<W::Id>,
    view_size: Size<f64, Logical>,
    working_area: Rectangle<f64, Logical>,
    scale: f64,
    options: Rc<Options>,
}

impl<W: LayoutElement> Scratchpad<W> {
    pub fn new(
        view_size: Size<f64, Logical>,
        working_area: Rectangle<f64, Logical>,
        scale: f64,
        options: Rc<Options>,
    ) -> Self {
        Self {
            tiles: Vec::new(),
            positions: Vec::new(),
            shown: None,
            view_size,
            working_area,
            scale,
            options,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn shown_id(&self) -> Option<&W::Id> {
        self.shown.as_ref()
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

    /// Most recently scratched window id.
    pub fn mru_id(&self) -> Option<&W::Id> {
        self.tiles.last().map(|t| t.window().id())
    }

    pub fn update_config(
        &mut self,
        view_size: Size<f64, Logical>,
        working_area: Rectangle<f64, Logical>,
        scale: f64,
        options: Rc<Options>,
    ) {
        self.view_size = view_size;
        self.working_area = working_area;
        self.scale = scale;
        self.options = options.clone();
        for tile in &mut self.tiles {
            tile.update_config(view_size, scale, options.clone());
        }
        self.relayout_shown();
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

    /// Park a removed tile onto the scratchpad (becomes MRU).
    pub fn push(&mut self, mut removed: RemovedTile<W>) {
        let id = removed.tile.window().id().clone();
        if self.shown.as_ref() == Some(&id) {
            self.shown = None;
        }

        // Prefer stored floating position; otherwise center.
        let pos = removed.tile.floating_pos.unwrap_or_else(|| {
            let size = removed.tile.tile_expected_or_current_size();
            center_size_frac(self.working_area, size)
        });

        removed.tile.stop_move_animations();
        removed.tile.floating_pos = Some(pos);
        // Scratchpad windows behave like floating for sizing.
        removed.tile.window_mut().set_floating(true);
        if let Some(size) = removed.tile.floating_window_size {
            removed.tile.window_mut().request_size_once(size, true);
        }

        self.tiles.push(removed.tile);
        self.positions.push(pos);
    }

    /// Remove a parked window from the scratchpad.
    pub fn remove(&mut self, id: &W::Id) -> Option<RemovedTile<W>> {
        let idx = self.idx_of(id)?;
        let mut tile = self.tiles.remove(idx);
        let pos = self.positions.remove(idx);
        tile.floating_pos = Some(pos);

        if self.shown.as_ref() == Some(id) {
            self.shown = None;
        }

        if let Some(size) = tile.window().expected_size() {
            tile.floating_window_size = Some(size);
        }

        let width = super::scrolling::ColumnWidth::Fixed(tile.tile_expected_or_current_size().w);
        Some(RemovedTile {
            tile,
            width,
            is_full_width: false,
            is_floating: true,
        })
    }

    pub fn remove_window(&mut self, id: &W::Id, _transaction: Transaction) -> Option<RemovedTile<W>> {
        self.remove(id)
    }

    /// Toggle visibility of the MRU scratched window.
    pub fn toggle(&mut self) {
        let Some(mru) = self.mru_id().cloned() else {
            return;
        };
        if self.shown.as_ref() == Some(&mru) {
            self.shown = None;
        } else {
            self.shown = Some(mru);
            self.relayout_shown();
        }
    }

    /// Cycle MRU among parked windows. If something is shown, show the new MRU.
    pub fn cycle(&mut self, forward: bool) {
        if self.tiles.len() < 2 {
            if self.tiles.len() == 1 && self.shown.is_none() {
                self.shown = self.mru_id().cloned();
                self.relayout_shown();
            }
            return;
        }

        let was_shown = self.shown.is_some();
        if forward {
            let tile = self.tiles.remove(0);
            let pos = self.positions.remove(0);
            self.tiles.push(tile);
            self.positions.push(pos);
        } else {
            let tile = self.tiles.pop().unwrap();
            let pos = self.positions.pop().unwrap();
            self.tiles.insert(0, tile);
            self.positions.insert(0, pos);
        }

        if was_shown || self.shown.is_some() {
            self.shown = self.mru_id().cloned();
            self.relayout_shown();
        }
    }

    pub fn hide(&mut self) {
        self.shown = None;
    }

    pub fn show_mru(&mut self) {
        self.shown = self.mru_id().cloned();
        self.relayout_shown();
    }

    fn relayout_shown(&mut self) {
        let Some(id) = self.shown.clone() else {
            return;
        };
        let Some(idx) = self.idx_of(&id) else {
            self.shown = None;
            return;
        };

        let working_area = self.working_area;
        let pos = self.positions[idx];
        let logical = size_frac_to_logical(working_area, pos);
        let tile = &mut self.tiles[idx];

        // Request a reasonable floating size if needed.
        if tile.floating_window_size.is_none() {
            if let Some(size) = tile.window().expected_size() {
                tile.floating_window_size = Some(size);
            }
        }
        if let Some(size) = tile.floating_window_size {
            tile.window_mut().request_size_once(size, true);
        }
        tile.floating_pos = Some(pos);

        let _ = logical; // position used at render time
    }

    pub fn shown_tile_pos(&self) -> Option<(&Tile<W>, Point<f64, Logical>)> {
        let id = self.shown.as_ref()?;
        let idx = self.idx_of(id)?;
        let tile = &self.tiles[idx];
        let logical = size_frac_to_logical(self.working_area, self.positions[idx]);
        let pos = logical
            .to_physical_precise_round(self.scale)
            .to_logical(self.scale);
        Some((tile, pos))
    }

    /// Window-geometry-relative target rect for popup unconstraining.
    pub fn popup_target_rect(&self, id: &W::Id) -> Option<Rectangle<f64, Logical>> {
        let idx = self.idx_of(id)?;
        let tile = &self.tiles[idx];
        let pos = size_frac_to_logical(self.working_area, self.positions[idx]);
        let mut target = self.working_area;
        target.loc -= pos;
        target.loc -= tile.window_loc();
        Some(target)
    }

    pub fn update_render_elements(&mut self, is_active: bool) {
        let view_rect = Rectangle::from_size(self.view_size);
        let shown = self.shown.clone();
        for (idx, tile) in self.tiles.iter_mut().enumerate() {
            let is_shown = shown.as_ref() == Some(tile.window().id());
            let logical = size_frac_to_logical(self.working_area, self.positions[idx]);
            let mut tile_view_rect = view_rect;
            tile_view_rect.loc -= logical + tile.render_offset();
            tile.update_render_elements(is_active && is_shown, tile_view_rect);
        }
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

    pub fn advance_animations(&mut self) {
        for tile in &mut self.tiles {
            tile.advance_animations();
        }
    }

    pub fn are_animations_ongoing(&self) -> bool {
        self.tiles.iter().any(Tile::are_animations_ongoing)
    }

    pub fn window_under(&self, pos: Point<f64, Logical>) -> Option<(&W, HitType)> {
        let (tile, tile_pos) = self.shown_tile_pos()?;
        HitType::hit_tile(tile, tile_pos, pos)
    }

    pub fn activate_shown(&mut self) -> bool {
        self.shown.is_some()
    }

    pub fn render<R: NiriRenderer>(
        &self,
        mut ctx: RenderCtx<R>,
        xray_pos: XrayPos,
        focus_ring: bool,
        push: &mut dyn FnMut(TileRenderElement<R>),
    ) {
        let Some((tile, tile_pos)) = self.shown_tile_pos() else {
            return;
        };
        let xray_pos = xray_pos.offset(tile_pos);
        tile.render(ctx.r(), tile_pos, xray_pos, focus_ring, push);
    }

    pub fn refresh(&mut self, is_active: bool) {
        let shown = self.shown.clone();
        for tile in &mut self.tiles {
            let win = tile.window_mut();
            let is_shown = shown.as_ref() == Some(win.id());
            win.set_active_in_column(true);
            win.set_floating(true);
            win.set_activated(is_active && is_shown);
            win.set_interactive_resize(None);
            win.set_bounds(self.view_size.to_i32_round());
            win.send_pending_configure();
            win.refresh();
        }
    }
}

fn center_size_frac(
    working_area: Rectangle<f64, Logical>,
    size: Size<f64, Logical>,
) -> Point<f64, SizeFrac> {
    let logical = Point::from((
        working_area.loc.x + (working_area.size.w - size.w) / 2.,
        working_area.loc.y + (working_area.size.h - size.h) / 2.,
    ));
    logical_to_size_frac(working_area, logical)
}

fn logical_to_size_frac(
    area: Rectangle<f64, Logical>,
    logical_pos: Point<f64, Logical>,
) -> Point<f64, SizeFrac> {
    let pos = logical_pos - area.loc;
    Point::from((
        pos.x / f64::max(area.size.w, 1.0),
        pos.y / f64::max(area.size.h, 1.0),
    ))
}

fn size_frac_to_logical(
    area: Rectangle<f64, Logical>,
    pos: Point<f64, SizeFrac>,
) -> Point<f64, Logical> {
    Point::from((
        area.loc.x + pos.x * area.size.w,
        area.loc.y + pos.y * area.size.h,
    ))
}
