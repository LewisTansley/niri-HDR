use std::collections::HashMap;
use std::rc::Rc;

use glam::{Mat3, Vec2};
use niri_config::{Color, SidebarSide};
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{GlesError, GlesFrame, GlesRenderer, Uniform};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::gpu_span_location;
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Logical, Physical, Point, Rectangle, Scale, Size, Transform};

use super::renderer::NiriRenderer;
use super::shader_element::ShaderRenderElement;
use super::shaders::{mat3_uniform, ProgramType, Shaders};
use crate::backend::tty::{TtyFrame, TtyRenderer, TtyRendererError};
use crate::render_helpers::renderer::AsGlesFrame as _;

/// Filled and/or stroked sidebar panel chrome with DMS-style viewport wingtips.
#[derive(Debug, Clone)]
pub struct SidebarChromeRenderElement {
    inner: ShaderRenderElement,
    params: Parameters,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Parameters {
    /// Expanded element size (panel + R into the viewport when scoop > 0).
    size: Size<f64, Logical>,
    /// Logical panel body width before the viewport extension.
    panel_width: f32,
    /// Border ring color, or solid color when there is no fill.
    color: Color,
    /// Interior fill; used when `has_fill` is true.
    fill_color: Color,
    has_fill: bool,
    side: SidebarSide,
    scoop_radius: f32,
    border_width: f32,
    scale: f32,
    alpha: f32,
}

impl SidebarChromeRenderElement {
    /// Solid fill only (no border ring).
    pub fn fill(
        size: Size<f64, Logical>,
        panel_width: f32,
        fill_color: Color,
        side: SidebarSide,
        scoop_radius: f32,
        scale: f32,
        alpha: f32,
    ) -> Self {
        Self::new(
            size,
            panel_width,
            Color::default(),
            Some(fill_color),
            side,
            scoop_radius,
            0.,
            scale,
            alpha,
        )
    }

    /// Border ring only (no fill).
    pub fn border(
        size: Size<f64, Logical>,
        panel_width: f32,
        border_color: Color,
        side: SidebarSide,
        scoop_radius: f32,
        border_width: f32,
        scale: f32,
        alpha: f32,
    ) -> Self {
        Self::new(
            size,
            panel_width,
            border_color,
            None,
            side,
            scoop_radius,
            border_width,
            scale,
            alpha,
        )
    }

    /// Single-pass fill + border ring (one outer AA edge).
    pub fn fill_and_border(
        size: Size<f64, Logical>,
        panel_width: f32,
        fill_color: Color,
        border_color: Color,
        side: SidebarSide,
        scoop_radius: f32,
        border_width: f32,
        scale: f32,
        alpha: f32,
    ) -> Self {
        Self::new(
            size,
            panel_width,
            border_color,
            Some(fill_color),
            side,
            scoop_radius,
            border_width,
            scale,
            alpha,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new(
        size: Size<f64, Logical>,
        panel_width: f32,
        color: Color,
        fill: Option<Color>,
        side: SidebarSide,
        scoop_radius: f32,
        border_width: f32,
        scale: f32,
        alpha: f32,
    ) -> Self {
        let inner = ShaderRenderElement::empty(ProgramType::SidebarChrome, Kind::Unspecified);
        let mut rv = Self {
            inner,
            params: Parameters {
                size,
                panel_width,
                color,
                fill_color: fill.unwrap_or_default(),
                has_fill: fill.is_some(),
                side,
                scoop_radius,
                border_width,
                scale,
                alpha,
            },
        };
        rv.update_inner();
        rv
    }

    pub fn empty() -> Self {
        let inner = ShaderRenderElement::empty(ProgramType::SidebarChrome, Kind::Unspecified);
        Self {
            inner,
            params: Parameters {
                size: Default::default(),
                panel_width: 0.,
                color: Default::default(),
                fill_color: Default::default(),
                has_fill: false,
                side: SidebarSide::Left,
                scoop_radius: 0.,
                border_width: 0.,
                scale: 1.,
                alpha: 1.,
            },
        }
    }

    fn update_inner(&mut self) {
        let Parameters {
            size,
            panel_width,
            color,
            fill_color,
            has_fill,
            side,
            scoop_radius,
            border_width,
            scale,
            alpha,
        } = self.params;

        let area_size = Vec2::new(size.w as f32, size.h as f32);
        let geo_size = area_size;
        let input_to_geo = Mat3::from_scale(area_size);

        let side_f = match side {
            SidebarSide::Left => 0.,
            SidebarSide::Right => 1.,
        };

        self.inner.update(
            size,
            None,
            scale,
            alpha,
            Rc::new([
                mat3_uniform("input_to_geo", input_to_geo),
                Uniform::new("geo_size", geo_size.to_array()),
                Uniform::new("color", color.to_array_unpremul()),
                Uniform::new("fill_color", fill_color.to_array_unpremul()),
                Uniform::new("has_fill", if has_fill { 1. } else { 0. }),
                Uniform::new("side", side_f),
                Uniform::new("scoop_radius", scoop_radius),
                Uniform::new("panel_width", panel_width),
                Uniform::new("border_width", border_width),
            ]),
            HashMap::new(),
        );
    }

    pub fn with_location(mut self, location: Point<f64, Logical>) -> Self {
        self.inner = self.inner.with_location(location);
        self
    }

    pub fn has_shader(renderer: &mut impl NiriRenderer) -> bool {
        Shaders::get(renderer)
            .program(ProgramType::SidebarChrome)
            .is_some()
    }
}

impl Default for SidebarChromeRenderElement {
    fn default() -> Self {
        Self::empty()
    }
}

impl Element for SidebarChromeRenderElement {
    fn id(&self) -> &Id {
        self.inner.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }

    fn transform(&self) -> Transform {
        self.inner.transform()
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        self.inner.damage_since(scale, commit)
    }

    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        self.inner.opaque_regions(scale)
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        self.inner.kind()
    }
}

impl RenderElement<GlesRenderer> for SidebarChromeRenderElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        let _span = tracy_client::span!("SidebarChromeRenderElement::draw");
        frame.with_gpu_span(
            gpu_span_location!("SidebarChromeRenderElement::draw"),
            |frame| {
                RenderElement::<GlesRenderer>::draw(
                    &self.inner,
                    frame,
                    src,
                    dst,
                    damage,
                    opaque_regions,
                    cache,
                )
            },
        )
    }

    fn underlying_storage(&self, renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        self.inner.underlying_storage(renderer)
    }
}

impl<'render> RenderElement<TtyRenderer<'render>> for SidebarChromeRenderElement {
    fn draw(
        &self,
        frame: &mut TtyFrame<'_, '_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), TtyRendererError<'render>> {
        let frame = frame.as_gles_frame();
        RenderElement::<GlesRenderer>::draw(self, frame, src, dst, damage, opaque_regions, cache)?;
        Ok(())
    }

    fn underlying_storage(
        &self,
        renderer: &mut TtyRenderer<'render>,
    ) -> Option<UnderlyingStorage<'_>> {
        self.inner.underlying_storage(renderer)
    }
}
