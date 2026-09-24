//! Per-output blend space for mixed SDR/HDR support.
//!
//! An output is either SDR (electrical sRGB, the default) or HDR (the framebuffer holds
//! PQ/BT.2020 electrical values and the connector is signalled accordingly). On HDR outputs:
//! - SDR content is encoded into the blend space at `sdr_brightness` (or `reference_luminance`
//!   when unset)
//! - PQ HDR content is reference-matched then ICtCp-tonemapped against `max_nits`
//! - extended-linear / Windows-scRGB content is scaled to reference white then tonemapped
//!
//! Blending happens directly in PQ-encoded space (same approximation as sRGB-space blending).

use std::cell::Cell;
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{GlesError, GlesFrame, GlesRenderer, Uniform};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::backend::renderer::Color32F;
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Physical, Point, Rectangle, Scale, Transform};
use smithay::wayland::color::management::ImageDescription;

use smithay::backend::renderer::{ImportAll, Renderer};

use super::renderer::AsGlesFrame as _;
use super::shaders::Shaders;
use crate::backend::tty::{TtyFrame, TtyRenderer, TtyRendererError};

/// Default SDR reference white in cd/m² (BT.2408).
pub const DEFAULT_REFERENCE_LUMINANCE: f64 = 203.;

/// PQ reference white in cd/m² (used by unit tests).
#[cfg(test)]
const PQ_REFERENCE_LUMINANCE: f64 = 203.;

/// HDR frame blend luminances in cd/m²: `(reference_luminance, max_nits, sdr_brightness)`.
pub type BlendLuminances = (f64, f64, f64);

/// The blend state of the frame currently being rendered, stored in the renderer's EGL user
/// data (like [`super::shaders::Shaders`]).
#[derive(Debug, Default)]
pub struct FrameBlendState {
    hdr_pq: Cell<bool>,
    ref_lum_scale: Cell<f32>,
    max_nit_scale: Cell<f32>,
    sdr_lum_scale: Cell<f32>,
}

impl FrameBlendState {
    pub fn init(renderer: &mut GlesRenderer) {
        let data = renderer.egl_context().user_data();
        data.insert_if_missing(FrameBlendState::default);
    }

    fn get(renderer: &GlesRenderer) -> &Self {
        renderer
            .egl_context()
            .user_data()
            .get()
            .expect("FrameBlendState::init() must be called when creating the renderer")
    }

    /// Marks frames as HDR with `(reference_luminance, max_nits, sdr_brightness)` in cd/m², or
    /// SDR (`None`).
    pub fn set(renderer: &mut GlesRenderer, luminances: Option<BlendLuminances>) {
        let state = Self::get(renderer);
        match luminances {
            Some((ref_lum, max_nits, sdr_brightness)) => {
                state.hdr_pq.set(true);
                state.ref_lum_scale.set((ref_lum / 10000.) as f32);
                state.max_nit_scale.set((max_nits / 10000.) as f32);
                state.sdr_lum_scale.set((sdr_brightness / 10000.) as f32);
            }
            None => {
                state.hdr_pq.set(false);
                state.ref_lum_scale.set(0.);
                state.max_nit_scale.set(0.);
                state.sdr_lum_scale.set(0.);
            }
        }
    }

    fn values_from_frame(frame: &GlesFrame) -> (bool, f32, f32, f32) {
        let state: &Self = frame
            .egl_context()
            .user_data()
            .get()
            .expect("FrameBlendState::init() must be called when creating the renderer");
        (
            state.hdr_pq.get(),
            state.ref_lum_scale.get(),
            state.max_nit_scale.get(),
            state.sdr_lum_scale.get(),
        )
    }

    /// Uniforms for SDR (or compositor-drawn) content in this frame.
    pub fn uniforms(frame: &GlesFrame) -> [Uniform<'static>; 9] {
        Self::uniforms_for_content(frame, ContentKind::Sdr)
    }

    /// Uniforms for a draw in this frame.
    pub fn uniforms_for_content(frame: &GlesFrame, kind: ContentKind) -> [Uniform<'static>; 9] {
        let (hdr_pq, ref_scale, max_scale, sdr_scale) = Self::values_from_frame(frame);
        let apply = hdr_pq && !matches!(kind, ContentKind::Passthrough);
        let content_hdr = matches!(kind, ContentKind::Hdr { .. });
        let content_peak = match kind {
            ContentKind::Hdr { peak_nits } => (peak_nits / 10000.) as f32,
            ContentKind::Linear { peak_nits, .. } => (peak_nits / 10000.) as f32,
            _ => 0.,
        };
        let linear = matches!(kind, ContentKind::Linear { .. });
        let linear_scale = match kind {
            ContentKind::Linear { reference_nits, .. } => (reference_nits / 10000.) as f32,
            _ => 0.,
        };
        [
            Uniform::new("niri_hdr_pq", if apply { 1.0f32 } else { 0.0 }),
            Uniform::new("niri_ref_lum_scale", ref_scale),
            Uniform::new("niri_max_nit_scale", max_scale),
            Uniform::new("niri_sdr_lum_scale", sdr_scale),
            Uniform::new("niri_content_hdr", if content_hdr { 1.0f32 } else { 0.0 }),
            Uniform::new("niri_content_peak", content_peak),
            Uniform::new("niri_linear", if linear { 1.0f32 } else { 0.0 }),
            Uniform::new("niri_linear_scale", linear_scale),
            Uniform::new("niri_hdr_to_sdr", 0.0f32),
        ]
    }
}

/// Windows-scRGB absolute scale: electrical 1.0 = 80 cd/m² (protocol / DXGI).
///
/// Do not remap 1.0 to paper white (203): that is identical to treating 1.0 as 203 nits and
/// lifts midtones. Games place paper white around ~2–3 (≈160–240 nits) on this scale.
const WINDOWS_LINEAR_REFERENCE_NITS: f64 = 80.;

/// Default peak luminance assumed for Windows HDR / scRGB content without metadata.
const WINDOWS_LINEAR_PEAK_NITS: f64 = 1000.;

/// How a draw's pixels are encoded relative to the output blend space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ContentKind {
    /// Electrical sRGB / untagged — encode into HDR at paper white when the frame is HDR.
    Sdr,
    /// Already PQ/BT.2020 — reference-match and tonemap. `peak_nits` from image description.
    Hdr { peak_nits: f64 },
    /// Extended-linear / Windows-scRGB — scale 1.0 to `reference_nits`, tonemap against `peak_nits`.
    Linear {
        reference_nits: f64,
        peak_nits: f64,
    },
    /// Content already in the output blend space (e.g. a sampled framebuffer effect).
    Passthrough,
}

fn linear_peak_from_description(d: &ImageDescription) -> f64 {
    d.max_cll
        .map(|v| f64::from(v))
        .or_else(|| d.mastering_luminance.map(|(_, max)| f64::from(max)))
        .filter(|v| *v > 0.)
        .unwrap_or(WINDOWS_LINEAR_PEAK_NITS)
}

fn linear_reference_from_description(d: &ImageDescription) -> f64 {
    d.luminances
        .map(|(_, _, reference)| f64::from(reference))
        .filter(|v| *v > 0.)
        .unwrap_or(WINDOWS_LINEAR_REFERENCE_NITS)
}

fn windows_linear_kind(d: &ImageDescription) -> ContentKind {
    ContentKind::Linear {
        reference_nits: linear_reference_from_description(d),
        peak_nits: linear_peak_from_description(d),
    }
}

/// Maps a committed surface image description to the blend-space content kind.
pub fn content_kind_from_description(desc: Option<ImageDescription>) -> ContentKind {
    match desc {
        Some(d) if d.is_windows_scrgb() => windows_linear_kind(&d),
        Some(d)
            if matches!(
                d.transfer,
                smithay::wayland::color::management::TransferFunction::ExtLinear
            ) =>
        {
            windows_linear_kind(&d)
        }
        Some(d) if d.is_pq() => ContentKind::Hdr {
            peak_nits: d
                .max_cll
                .map(|v| f64::from(v))
                .or_else(|| d.mastering_luminance.map(|(_, max)| f64::from(max)))
                .unwrap_or(1000.),
        },
        _ => ContentKind::Sdr,
    }
}

/// True for 16-bit float drm fourccs used by scRGB / EXTENDED_SRGB_LINEAR swapchains.
pub fn is_fp16_fourcc(code: smithay::backend::allocator::Fourcc) -> bool {
    use smithay::backend::allocator::Fourcc;
    matches!(
        code,
        Fourcc::Argb16161616f
            | Fourcc::Abgr16161616f
            | Fourcc::Xrgb16161616f
            | Fourcc::Xbgr16161616f
    )
}

/// Distinct `(fourcc, tagged, kind)` combinations already logged by [`log_content_kind`].
static LOGGED_CONTENT_KINDS: OnceLock<Mutex<HashSet<(Option<u32>, bool, &'static str)>>> =
    OnceLock::new();

/// Logs each distinct buffer classification once; this runs per surface per frame.
fn log_content_kind(
    tagged: bool,
    buffer_fourcc: Option<smithay::backend::allocator::Fourcc>,
    kind: ContentKind,
) {
    let label = match kind {
        ContentKind::Sdr => "sdr",
        ContentKind::Hdr { .. } => "hdr-pq",
        ContentKind::Linear { .. } => "linear-scrgb",
        ContentKind::Passthrough => "passthrough",
    };

    let logged = LOGGED_CONTENT_KINDS.get_or_init(Default::default);
    if !logged
        .lock()
        .unwrap()
        .insert((buffer_fourcc.map(|code| code as u32), tagged, label))
    {
        return;
    }

    debug!("buffer content kind: fourcc={buffer_fourcc:?} tagged={tagged} kind={kind:?}");
}

/// Resolves content kind from the image description, then applies the FP16 heuristic: real
/// HDR10 is 10-bit UNORM, so an FP16 swapchain is extended-linear no matter how (or whether)
/// the client tagged it. Windows/DXVK scRGB buffers often arrive untagged, and reading those
/// as electrical sRGB overflows the PQ encode at highlights.
pub fn content_kind_for_buffer(
    desc: Option<ImageDescription>,
    buffer_fourcc: Option<smithay::backend::allocator::Fourcc>,
) -> ContentKind {
    let kind = content_kind_from_description(desc);
    let kind = if buffer_fourcc.is_some_and(is_fp16_fourcc)
        && !matches!(kind, ContentKind::Linear { .. })
    {
        ContentKind::Linear {
            reference_nits: desc
                .as_ref()
                .map(linear_reference_from_description)
                .unwrap_or(WINDOWS_LINEAR_REFERENCE_NITS),
            peak_nits: desc
                .as_ref()
                .map(linear_peak_from_description)
                .unwrap_or(WINDOWS_LINEAR_PEAK_NITS),
        }
    } else {
        kind
    };

    log_content_kind(desc.is_some(), buffer_fourcc, kind);

    kind
}

/// Configures the renderer for rendering into an SDR capture buffer from an HDR session:
/// HDR surfaces are decoded via the HDR→SDR texture program; SDR content draws normally.
pub fn set_sdr_capture_blend(renderer: &mut GlesRenderer, reference_luminance: f64) {
    FrameBlendState::set(renderer, None);
    let scale = (reference_luminance / 10000.) as f32;
    let program = Shaders::get(renderer).texture_hdr_to_sdr.clone();
    if let Some(program) = program {
        renderer.set_default_tex_program_override(Some((
            program,
            vec![Uniform::new("niri_ref_lum_scale", scale)],
        )));
    } else {
        warn!("HDR-to-SDR texture shader missing; HDR capture will render raw");
        renderer.set_default_tex_program_override(None);
    }
    renderer.set_solid_color_transform(None);
}

/// Configures the renderer for `(reference_luminance, max_nits, sdr_brightness)` HDR blend, or
/// SDR (`None`).
pub fn set_frame_blend(renderer: &mut GlesRenderer, luminances: Option<BlendLuminances>) {
    FrameBlendState::set(renderer, luminances);

    match luminances {
        Some((ref_lum, max_nits, sdr_brightness)) => {
            let ref_scale = (ref_lum / 10000.) as f32;
            let max_scale = (max_nits / 10000.) as f32;
            let sdr_scale = (sdr_brightness / 10000.) as f32;
            let program = Shaders::get(renderer).texture_hdr.clone();
            if let Some(program) = program {
                renderer.set_default_tex_program_override(Some((
                    program,
                    vec![
                        Uniform::new("niri_hdr_pq", 1.0f32),
                        Uniform::new("niri_ref_lum_scale", ref_scale),
                        Uniform::new("niri_max_nit_scale", max_scale),
                        Uniform::new("niri_sdr_lum_scale", sdr_scale),
                        Uniform::new("niri_content_hdr", 0.0f32),
                        Uniform::new("niri_content_peak", 0.0f32),
                        Uniform::new("niri_linear", 0.0f32),
                        Uniform::new("niri_linear_scale", 0.0f32),
                        Uniform::new("niri_hdr_to_sdr", 0.0f32),
                    ],
                )));
            } else {
                warn!("HDR texture shader missing; SDR content will render raw");
            }
            // Solid colors and the default texture override are SDR-only paths.
            renderer
                .set_solid_color_transform(Some(Box::new(move |color| srgb_to_pq(color, sdr_scale))));
        }
        None => {
            renderer.set_default_tex_program_override(None);
            renderer.set_solid_color_transform(None);
        }
    }
}

/// CPU counterpart of the shaders' SDR→PQ path.
pub fn srgb_to_pq(color: Color32F, ref_lum_scale: f32) -> Color32F {
    let a = color.a();
    let unpremul = |c: f32| if a > 0. { c / a } else { c };

    let pq = |lin: f32| {
        const M1: f32 = 0.1593017578125;
        const M2: f32 = 78.84375;
        const C1: f32 = 0.8359375;
        const C2: f32 = 18.8515625;
        const C3: f32 = 18.6875;
        let y = lin.clamp(0., 1.).powf(M1);
        ((C1 + C2 * y) / (1. + C3 * y)).powf(M2)
    };

    let r = unpremul(color.r()).max(0.).powf(2.2);
    let g = unpremul(color.g()).max(0.).powf(2.2);
    let b = unpremul(color.b()).max(0.).powf(2.2);

    let r2020 = 0.627404 * r + 0.329283 * g + 0.043313 * b;
    let g2020 = 0.069097 * r + 0.919540 * g + 0.011362 * b;
    let b2020 = 0.016391 * r + 0.088013 * g + 0.895595 * b;

    Color32F::new(
        pq(r2020 * ref_lum_scale) * a,
        pq(g2020 * ref_lum_scale) * a,
        pq(b2020 * ref_lum_scale) * a,
        a,
    )
}

/// A surface-tree render element that knows how its content is encoded.
#[derive(Debug)]
pub struct BlendSurfaceRenderElement<R: Renderer> {
    inner: WaylandSurfaceRenderElement<R>,
    kind: ContentKind,
}

impl<R: Renderer> BlendSurfaceRenderElement<R> {
    pub fn with_kind(inner: WaylandSurfaceRenderElement<R>, kind: ContentKind) -> Self {
        Self { inner, kind }
    }

    pub fn inner(&self) -> &WaylandSurfaceRenderElement<R> {
        &self.inner
    }

    pub fn into_inner(self) -> WaylandSurfaceRenderElement<R> {
        self.inner
    }

    pub fn content_kind(&self) -> ContentKind {
        self.kind
    }

    pub fn content_hdr(&self) -> bool {
        matches!(self.kind, ContentKind::Hdr { .. } | ContentKind::Linear { .. })
    }
}

impl<R: Renderer + ImportAll> Element for BlendSurfaceRenderElement<R>
where
    R::TextureId: Clone + 'static,
{
    fn id(&self) -> &Id {
        self.inner.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }

    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        self.inner.location(scale)
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }

    fn transform(&self) -> Transform {
        self.inner.transform()
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

impl RenderElement<GlesRenderer> for BlendSurfaceRenderElement<GlesRenderer> {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        // On HDR frames the default tex override encodes SDR→PQ. For PQ/linear surfaces,
        // temporarily swap in the matching content-kind uniforms so hdr.frag converts
        // correctly instead of blind-passthrough. During SDR capture (`set_sdr_capture_blend`)
        // the override converts HDR→SDR and must be kept as-is.
        let saved = match self.kind {
            ContentKind::Hdr { .. } | ContentKind::Linear { .. } => {
                let (hdr_pq, _, _, _) = FrameBlendState::values_from_frame(frame);
                if hdr_pq {
                    let saved = frame.take_tex_program_override();
                    if let Some((program, _)) = saved.clone() {
                        let uniforms =
                            FrameBlendState::uniforms_for_content(frame, self.kind).to_vec();
                        frame.override_default_tex_program(program, uniforms);
                    }
                    saved
                } else {
                    None
                }
            }
            ContentKind::Sdr | ContentKind::Passthrough => None,
        };
        let res = RenderElement::<GlesRenderer>::draw(
            &self.inner,
            frame,
            src,
            dst,
            damage,
            opaque_regions,
            cache,
        );
        if saved.is_some() {
            frame.set_tex_program_override(saved);
        }
        res
    }

    fn underlying_storage(&self, renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        // scRGB / extended-linear must not hit a PQ-signalled plane.
        if matches!(self.kind, ContentKind::Linear { .. }) {
            return None;
        }
        self.inner.underlying_storage(renderer)
    }
}

impl<'render> RenderElement<TtyRenderer<'render>>
    for BlendSurfaceRenderElement<TtyRenderer<'render>>
{
    fn draw(
        &self,
        frame: &mut TtyFrame<'render, '_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), TtyRendererError<'render>> {
        let gles_frame = frame.as_gles_frame();
        let saved = match self.kind {
            ContentKind::Hdr { .. } | ContentKind::Linear { .. } => {
                let (hdr_pq, _, _, _) = FrameBlendState::values_from_frame(gles_frame);
                if hdr_pq {
                    let saved = gles_frame.take_tex_program_override();
                    if let Some((program, _)) = saved.clone() {
                        let uniforms =
                            FrameBlendState::uniforms_for_content(gles_frame, self.kind).to_vec();
                        gles_frame.override_default_tex_program(program, uniforms);
                    }
                    saved
                } else {
                    None
                }
            }
            ContentKind::Sdr | ContentKind::Passthrough => None,
        };
        let res = RenderElement::draw(&self.inner, frame, src, dst, damage, opaque_regions, cache);
        if saved.is_some() {
            frame.as_gles_frame().set_tex_program_override(saved);
        }
        res
    }

    fn underlying_storage(
        &self,
        renderer: &mut TtyRenderer<'render>,
    ) -> Option<UnderlyingStorage<'_>> {
        if matches!(self.kind, ContentKind::Linear { .. }) {
            return None;
        }
        self.inner.underlying_storage(renderer)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_to_pq_reference_values() {
        let scale = (PQ_REFERENCE_LUMINANCE / 10000.) as f32;

        let white = srgb_to_pq(Color32F::new(1., 1., 1., 1.), scale);
        assert!((white.r() - 0.5806).abs() < 0.002, "got {}", white.r());
        assert!((white.r() - white.g()).abs() < 0.0005);
        assert!((white.g() - white.b()).abs() < 0.0005);

        let black = srgb_to_pq(Color32F::new(0., 0., 0., 0.5), scale);
        assert!(black.r() < 1e-6, "got {}", black.r());
        assert_eq!(black.a(), 0.5);

        let half = srgb_to_pq(Color32F::new(0.5, 0.5, 0.5, 0.5), scale);
        assert!((half.r() - white.r() * 0.5).abs() < 0.0005);
    }

    #[test]
    fn srgb_to_pq_uses_provided_sdr_scale() {
        let ref_scale = (PQ_REFERENCE_LUMINANCE / 10000.) as f32;
        let sdr_scale = (400. / 10000.) as f32;

        let at_ref = srgb_to_pq(Color32F::new(1., 1., 1., 1.), ref_scale);
        let at_sdr = srgb_to_pq(Color32F::new(1., 1., 1., 1.), sdr_scale);
        // Higher absolute nits encode to a higher PQ code value.
        assert!(
            at_sdr.r() > at_ref.r() + 0.02,
            "sdr 400 nits ({}) should be brighter than ref 203 ({})",
            at_sdr.r(),
            at_ref.r()
        );
    }

    #[test]
    fn content_kind_from_description_mapping() {
        use smithay::wayland::color::management::{Primaries, TransferFunction};

        assert_eq!(content_kind_from_description(None), ContentKind::Sdr);
        assert_eq!(
            content_kind_from_description(Some(ImageDescription::SRGB)),
            ContentKind::Sdr
        );
        assert_eq!(
            content_kind_from_description(Some(ImageDescription::WINDOWS_SCRGB)),
            ContentKind::Linear {
                reference_nits: 80.,
                peak_nits: 1000.,
            }
        );

        let pq = ImageDescription {
            transfer: TransferFunction::St2084Pq,
            primaries: Primaries::Bt2020,
            max_cll: Some(650),
            max_fall: None,
            mastering_luminance: Some((50, 1000)),
            luminances: None,
            windows_scrgb: false,
        };
        assert_eq!(
            content_kind_from_description(Some(pq)),
            ContentKind::Hdr { peak_nits: 650. }
        );

        let pq_mastering_only = ImageDescription {
            max_cll: None,
            ..pq
        };
        assert_eq!(
            content_kind_from_description(Some(pq_mastering_only)),
            ContentKind::Hdr { peak_nits: 1000. }
        );

        let ext_linear = ImageDescription {
            transfer: TransferFunction::ExtLinear,
            primaries: Primaries::Srgb,
            max_cll: None,
            max_fall: None,
            mastering_luminance: None,
            luminances: None,
            windows_scrgb: false,
        };
        assert_eq!(
            content_kind_from_description(Some(ext_linear)),
            ContentKind::Linear {
                reference_nits: 80.,
                peak_nits: 1000.,
            }
        );
        assert!(ext_linear.is_hdr());
        assert!(!ext_linear.is_pq());

        let ext_linear_ref = ImageDescription {
            luminances: Some((2, 80, 100)),
            max_cll: Some(800),
            ..ext_linear
        };
        assert_eq!(
            content_kind_from_description(Some(ext_linear_ref)),
            ContentKind::Linear {
                reference_nits: 100.,
                peak_nits: 800.,
            }
        );
    }

    #[test]
    fn content_kind_fp16_heuristic() {
        use smithay::backend::allocator::Fourcc;
        use smithay::wayland::color::management::{Primaries, TransferFunction};

        let pq = ImageDescription {
            transfer: TransferFunction::St2084Pq,
            primaries: Primaries::Bt2020,
            max_cll: Some(1000),
            max_fall: None,
            mastering_luminance: None,
            luminances: None,
            windows_scrgb: false,
        };
        assert_eq!(
            content_kind_for_buffer(Some(pq), Some(Fourcc::Abgr16161616f)),
            ContentKind::Linear {
                reference_nits: 80.,
                peak_nits: 1000.,
            }
        );
        assert_eq!(
            content_kind_for_buffer(Some(pq), Some(Fourcc::Abgr2101010)),
            ContentKind::Hdr { peak_nits: 1000. }
        );

        // Untagged and sRGB-tagged FP16 swapchains are scRGB too; reading them as electrical
        // sRGB overflows the PQ encode and breaks hue at highlights.
        let scrgb = ContentKind::Linear {
            reference_nits: 80.,
            peak_nits: 1000.,
        };
        assert_eq!(
            content_kind_for_buffer(None, Some(Fourcc::Abgr16161616f)),
            scrgb
        );
        assert_eq!(
            content_kind_for_buffer(Some(ImageDescription::SRGB), Some(Fourcc::Xrgb16161616f)),
            scrgb
        );

        // 8-bit buffers keep their tagged (or untagged) meaning.
        assert_eq!(content_kind_for_buffer(None, Some(Fourcc::Xrgb8888)), ContentKind::Sdr);

        assert!(is_fp16_fourcc(Fourcc::Argb16161616f));
        assert!(!is_fp16_fourcc(Fourcc::Abgr2101010));
    }
}
