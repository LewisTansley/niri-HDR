use std::cell::{Cell, OnceCell, RefCell};

use niri_config::utils::Flag;
use niri_config::workspace::WorkspaceName;
use niri_config::{
    CenterFocusedColumn, CenterVisibleColumns, FloatOrInt, OutputName, Struts,
    TabIndicatorLength, TabIndicatorPosition, WorkspaceReference,
};
use proptest::prelude::*;
use proptest_derive::Arbitrary;
use smithay::output::{Mode, PhysicalProperties, Subpixel};
use smithay::utils::Rectangle;

use super::*;

mod animations;
mod fullscreen;

impl<W: LayoutElement> Default for Layout<W> {
    fn default() -> Self {
        Self::with_options(Clock::with_time(Duration::ZERO), Default::default())
    }
}

#[derive(Debug)]
struct TestWindowInner {
    id: usize,
    parent_id: Cell<Option<usize>>,
    bbox: Cell<Rectangle<i32, Logical>>,
    initial_bbox: Rectangle<i32, Logical>,
    requested_size: Cell<Option<Size<i32, Logical>>>,
    // Emulates the window ignoring the compositor-provided size.
    forced_size: Cell<Option<Size<i32, Logical>>>,
    min_size: Size<i32, Logical>,
    max_size: Size<i32, Logical>,
    pending_sizing_mode: Cell<SizingMode>,
    pending_activated: Cell<bool>,
    sizing_mode: Cell<SizingMode>,
    is_windowed_fullscreen: Cell<bool>,
    is_pending_windowed_fullscreen: Cell<bool>,
    animate_next_configure: Cell<bool>,
    animation_snapshot: RefCell<Option<LayoutElementRenderSnapshot>>,
    rules: ResolvedWindowRules,
}

#[derive(Debug, Clone)]
struct TestWindow(Rc<TestWindowInner>);

#[derive(Debug, Clone, Arbitrary)]
struct TestWindowParams {
    #[proptest(strategy = "1..=5usize")]
    id: usize,
    #[proptest(strategy = "arbitrary_parent_id()")]
    parent_id: Option<usize>,
    is_floating: bool,
    #[proptest(strategy = "arbitrary_bbox()")]
    bbox: Rectangle<i32, Logical>,
    #[proptest(strategy = "arbitrary_min_max_size()")]
    min_max_size: (Size<i32, Logical>, Size<i32, Logical>),
    #[proptest(strategy = "prop::option::of(arbitrary_rules())")]
    rules: Option<ResolvedWindowRules>,
}

impl TestWindowParams {
    pub fn new(id: usize) -> Self {
        Self {
            id,
            parent_id: None,
            is_floating: false,
            bbox: Rectangle::from_size(Size::from((100, 200))),
            min_max_size: Default::default(),
            rules: None,
        }
    }
}

impl TestWindow {
    fn new(params: TestWindowParams) -> Self {
        Self(Rc::new(TestWindowInner {
            id: params.id,
            parent_id: Cell::new(params.parent_id),
            bbox: Cell::new(params.bbox),
            initial_bbox: params.bbox,
            requested_size: Cell::new(None),
            forced_size: Cell::new(None),
            min_size: params.min_max_size.0,
            max_size: params.min_max_size.1,
            pending_sizing_mode: Cell::new(SizingMode::Normal),
            pending_activated: Cell::new(false),
            sizing_mode: Cell::new(SizingMode::Normal),
            is_windowed_fullscreen: Cell::new(false),
            is_pending_windowed_fullscreen: Cell::new(false),
            animate_next_configure: Cell::new(false),
            animation_snapshot: RefCell::new(None),
            rules: params.rules.unwrap_or_default(),
        }))
    }

    fn communicate(&self) -> bool {
        let mut changed = false;

        let size = self.0.forced_size.get().or(self.0.requested_size.get());
        if let Some(size) = size {
            assert!(size.w >= 0);
            assert!(size.h >= 0);

            let mut new_bbox = self.0.initial_bbox;
            if size.w != 0 {
                new_bbox.size.w = size.w;
            }
            if size.h != 0 {
                new_bbox.size.h = size.h;
            }

            if self.0.bbox.get() != new_bbox {
                if self.0.animate_next_configure.get() {
                    self.0.animation_snapshot.replace(Some(RenderSnapshot {
                        contents: Vec::new(),
                        contents_with_blocked_out_bg: None,
                        blocked_out_contents: Vec::new(),
                        block_out_from: None,
                        size: self.0.bbox.get().size.to_f64(),
                        texture: OnceCell::new(),
                        texture_with_blocked_out_bg: Default::default(),
                        blocked_out_texture: OnceCell::new(),
                    }));
                }

                self.0.bbox.set(new_bbox);
                changed = true;
            }
        }

        self.0.animate_next_configure.set(false);

        if self.0.sizing_mode.get() != self.0.pending_sizing_mode.get() {
            self.0.sizing_mode.set(self.0.pending_sizing_mode.get());
            changed = true;
        }

        if self.0.is_windowed_fullscreen.get() != self.0.is_pending_windowed_fullscreen.get() {
            self.0
                .is_windowed_fullscreen
                .set(self.0.is_pending_windowed_fullscreen.get());
            changed = true;
        }

        changed
    }
}

impl LayoutElement for TestWindow {
    type Id = usize;

    fn id(&self) -> &Self::Id {
        &self.0.id
    }

    fn size(&self) -> Size<i32, Logical> {
        self.0.bbox.get().size
    }

    fn buf_loc(&self) -> Point<i32, Logical> {
        (0, 0).into()
    }

    fn is_in_input_region(&self, _point: Point<f64, Logical>) -> bool {
        false
    }

    fn request_size(
        &mut self,
        size: Size<i32, Logical>,
        mode: SizingMode,
        animate: bool,
        _transaction: Option<Transaction>,
    ) {
        if self.0.requested_size.get() != Some(size) {
            self.0.requested_size.set(Some(size));
            if animate {
                self.0.animate_next_configure.set(true);
            }
        }

        self.0.pending_sizing_mode.set(mode);

        if mode.is_fullscreen() {
            self.0.is_pending_windowed_fullscreen.set(false);
        }
    }

    fn min_size(&self) -> Size<i32, Logical> {
        self.0.min_size
    }

    fn max_size(&self) -> Size<i32, Logical> {
        self.0.max_size
    }

    fn is_wl_surface(&self, _wl_surface: &WlSurface) -> bool {
        false
    }

    fn set_preferred_scale_transform(&self, _scale: output::Scale, _transform: Transform) {}

    fn has_ssd(&self) -> bool {
        false
    }

    fn output_enter(&self, _output: &Output) {}

    fn output_leave(&self, _output: &Output) {}

    fn set_offscreen_data(&self, _data: Option<OffscreenData>) {}

    fn set_activated(&mut self, active: bool) {
        self.0.pending_activated.set(active);
    }

    fn set_bounds(&self, _bounds: Size<i32, Logical>) {}

    fn is_ignoring_opacity_window_rule(&self) -> bool {
        false
    }

    fn configure_intent(&self) -> ConfigureIntent {
        ConfigureIntent::CanSend
    }

    fn send_pending_configure(&mut self) {}

    fn set_active_in_column(&mut self, _active: bool) {}

    fn set_floating(&mut self, _floating: bool) {}

    fn sizing_mode(&self) -> SizingMode {
        self.0.sizing_mode.get()
    }

    fn pending_sizing_mode(&self) -> SizingMode {
        self.0.pending_sizing_mode.get()
    }

    fn requested_size(&self) -> Option<Size<i32, Logical>> {
        self.0.requested_size.get()
    }

    fn is_windowed_fullscreen(&self) -> bool {
        self.0.is_windowed_fullscreen.get()
    }

    fn is_pending_windowed_fullscreen(&self) -> bool {
        self.0.is_pending_windowed_fullscreen.get()
    }

    fn request_windowed_fullscreen(&mut self, value: bool) {
        self.0.is_pending_windowed_fullscreen.set(value);
    }

    fn is_child_of(&self, parent: &Self) -> bool {
        self.0.parent_id.get() == Some(parent.0.id)
    }

    fn refresh(&self) {}

    fn rules(&self) -> &ResolvedWindowRules {
        &self.0.rules
    }

    fn take_animation_snapshot(&mut self) -> Option<LayoutElementRenderSnapshot> {
        self.0.animation_snapshot.take()
    }

    fn set_interactive_resize(&mut self, _data: Option<InteractiveResizeData>) {}

    fn cancel_interactive_resize(&mut self) {}

    fn on_commit(&mut self, _serial: Serial) {}

    fn interactive_resize_data(&self) -> Option<InteractiveResizeData> {
        None
    }

    fn is_urgent(&self) -> bool {
        false
    }
}

fn arbitrary_size() -> impl Strategy<Value = Size<i32, Logical>> {
    any::<(u16, u16)>().prop_map(|(w, h)| Size::from((w.max(1).into(), h.max(1).into())))
}

fn arbitrary_bbox() -> impl Strategy<Value = Rectangle<i32, Logical>> {
    any::<(i16, i16, u16, u16)>().prop_map(|(x, y, w, h)| {
        let loc: Point<i32, _> = Point::from((x.into(), y.into()));
        let size: Size<i32, _> = Size::from((w.max(1).into(), h.max(1).into()));
        Rectangle::new(loc, size)
    })
}

fn arbitrary_size_change() -> impl Strategy<Value = SizeChange> {
    prop_oneof![
        (0..).prop_map(SizeChange::SetFixed),
        (0f64..).prop_map(SizeChange::SetProportion),
        any::<i32>().prop_map(SizeChange::AdjustFixed),
        any::<f64>().prop_map(SizeChange::AdjustProportion),
        // Interactive resize can have negative values here.
        Just(SizeChange::SetFixed(-100)),
    ]
}

fn arbitrary_position_change() -> impl Strategy<Value = PositionChange> {
    prop_oneof![
        (-1000f64..1000f64).prop_map(PositionChange::SetFixed),
        any::<f64>().prop_map(PositionChange::SetProportion),
        (-1000f64..1000f64).prop_map(PositionChange::AdjustFixed),
        any::<f64>().prop_map(PositionChange::AdjustProportion),
        any::<f64>().prop_map(PositionChange::SetFixed),
        any::<f64>().prop_map(PositionChange::AdjustFixed),
    ]
}

fn arbitrary_min_max() -> impl Strategy<Value = (i32, i32)> {
    prop_oneof![
        Just((0, 0)),
        (1..65536).prop_map(|n| (n, n)),
        (1..65536).prop_map(|min| (min, 0)),
        (1..).prop_map(|max| (0, max)),
        (1..65536, 1..).prop_map(|(min, max): (i32, i32)| (min, max.max(min))),
    ]
}

fn arbitrary_min_max_size() -> impl Strategy<Value = (Size<i32, Logical>, Size<i32, Logical>)> {
    prop_oneof![
        5 => (arbitrary_min_max(), arbitrary_min_max()).prop_map(
            |((min_w, max_w), (min_h, max_h))| {
                let min_size = Size::from((min_w, min_h));
                let max_size = Size::from((max_w, max_h));
                (min_size, max_size)
            },
        ),
        1 => arbitrary_min_max().prop_map(|(w, h)| {
            let size = Size::from((w, h));
            (size, size)
        }),
    ]
}

prop_compose! {
    fn arbitrary_rules()(
        focus_ring in arbitrary_focus_ring(),
        border in arbitrary_border(),
    ) -> ResolvedWindowRules {
        ResolvedWindowRules {
            focus_ring,
            border,
            ..ResolvedWindowRules::default()
        }
    }
}

fn arbitrary_view_offset_gesture_delta() -> impl Strategy<Value = f64> {
    prop_oneof![(-10f64..10f64), (-50000f64..50000f64),]
}

fn arbitrary_resize_edge() -> impl Strategy<Value = ResizeEdge> {
    prop_oneof![
        Just(ResizeEdge::RIGHT),
        Just(ResizeEdge::BOTTOM),
        Just(ResizeEdge::LEFT),
        Just(ResizeEdge::TOP),
        Just(ResizeEdge::BOTTOM_RIGHT),
        Just(ResizeEdge::BOTTOM_LEFT),
        Just(ResizeEdge::TOP_RIGHT),
        Just(ResizeEdge::TOP_LEFT),
        Just(ResizeEdge::empty()),
    ]
}

fn arbitrary_scale() -> impl Strategy<Value = f64> {
    prop_oneof![Just(1.), Just(1.5), Just(2.),]
}

fn arbitrary_msec_delta() -> impl Strategy<Value = i32> {
    prop_oneof![
        1 => Just(-1000),
        2 => Just(-10),
        1 => Just(0),
        2 => Just(10),
        6 => Just(1000),
    ]
}

fn arbitrary_parent_id() -> impl Strategy<Value = Option<usize>> {
    prop_oneof![
        5 => Just(None),
        1 => prop::option::of(1..=5usize),
    ]
}

fn arbitrary_scroll_direction() -> impl Strategy<Value = ScrollDirection> {
    prop_oneof![Just(ScrollDirection::Left), Just(ScrollDirection::Right)]
}

fn arbitrary_column_display() -> impl Strategy<Value = ColumnDisplay> {
    prop_oneof![Just(ColumnDisplay::Normal), Just(ColumnDisplay::Tabbed)]
}

#[derive(Debug, Clone, Arbitrary)]
enum Op {
    AddOutput(#[proptest(strategy = "1..=5usize")] usize),
    AddScaledOutput {
        #[proptest(strategy = "1..=5usize")]
        id: usize,
        #[proptest(strategy = "arbitrary_scale()")]
        scale: f64,
        #[proptest(strategy = "prop::option::of(arbitrary_layout_part().prop_map(Box::new))")]
        layout_config: Option<Box<niri_config::LayoutPart>>,
    },
    RemoveOutput(#[proptest(strategy = "1..=5usize")] usize),
    FocusOutput(#[proptest(strategy = "1..=5usize")] usize),
    UpdateOutputLayoutConfig {
        #[proptest(strategy = "1..=5usize")]
        id: usize,
        #[proptest(strategy = "prop::option::of(arbitrary_layout_part().prop_map(Box::new))")]
        layout_config: Option<Box<niri_config::LayoutPart>>,
    },
    AddNamedWorkspace {
        #[proptest(strategy = "1..=5usize")]
        ws_name: usize,
        #[proptest(strategy = "prop::option::of(1..=5usize)")]
        output_name: Option<usize>,
        #[proptest(strategy = "prop::option::of(arbitrary_layout_part().prop_map(Box::new))")]
        layout_config: Option<Box<niri_config::LayoutPart>>,
    },
    UnnameWorkspace {
        #[proptest(strategy = "1..=5usize")]
        ws_name: usize,
    },
    UpdateWorkspaceLayoutConfig {
        #[proptest(strategy = "1..=5usize")]
        ws_name: usize,
        #[proptest(strategy = "prop::option::of(arbitrary_layout_part().prop_map(Box::new))")]
        layout_config: Option<Box<niri_config::LayoutPart>>,
    },
    AddWindow {
        params: TestWindowParams,
    },
    AddWindowNextTo {
        params: TestWindowParams,
        #[proptest(strategy = "1..=5usize")]
        next_to_id: usize,
    },
    AddWindowToNamedWorkspace {
        params: TestWindowParams,
        #[proptest(strategy = "1..=5usize")]
        ws_name: usize,
    },
    CloseWindow(#[proptest(strategy = "1..=5usize")] usize),
    FullscreenWindow(#[proptest(strategy = "1..=5usize")] usize),
    SetFullscreenWindow {
        #[proptest(strategy = "1..=5usize")]
        window: usize,
        is_fullscreen: bool,
    },
    ToggleWindowedFullscreen(#[proptest(strategy = "1..=5usize")] usize),
    FocusColumnLeft,
    FocusColumnRight,
    FocusColumnFirst,
    FocusColumnLast,
    FocusColumnRightOrFirst,
    FocusColumnLeftOrLast,
    FocusColumn(#[proptest(strategy = "1..=5usize")] usize),
    FocusWindowOrMonitorUp(#[proptest(strategy = "1..=2u8")] u8),
    FocusWindowOrMonitorDown(#[proptest(strategy = "1..=2u8")] u8),
    FocusColumnOrMonitorLeft(#[proptest(strategy = "1..=2u8")] u8),
    FocusColumnOrMonitorRight(#[proptest(strategy = "1..=2u8")] u8),
    FocusWindowDown,
    FocusWindowUp,
    FocusWindowDownOrColumnLeft,
    FocusWindowDownOrColumnRight,
    FocusWindowUpOrColumnLeft,
    FocusWindowUpOrColumnRight,
    FocusWindowOrWorkspaceDown,
    FocusWindowOrWorkspaceUp,
    FocusWindow(#[proptest(strategy = "1..=5usize")] usize),
    FocusWindowInColumn(#[proptest(strategy = "1..=5u8")] u8),
    FocusWindowTop,
    FocusWindowBottom,
    FocusWindowDownOrTop,
    FocusWindowUpOrBottom,
    MoveColumnLeft,
    MoveColumnRight,
    MoveColumnToFirst,
    MoveColumnToLast,
    MoveColumnLeftOrToMonitorLeft(#[proptest(strategy = "1..=2u8")] u8),
    MoveColumnRightOrToMonitorRight(#[proptest(strategy = "1..=2u8")] u8),
    MoveColumnToIndex(#[proptest(strategy = "1..=5usize")] usize),
    MoveWindowDown,
    MoveWindowUp,
    MoveWindowDownOrToWorkspaceDown,
    MoveWindowUpOrToWorkspaceUp,
    ConsumeOrExpelWindowLeft {
        #[proptest(strategy = "proptest::option::of(1..=5usize)")]
        id: Option<usize>,
    },
    ConsumeOrExpelWindowRight {
        #[proptest(strategy = "proptest::option::of(1..=5usize)")]
        id: Option<usize>,
    },
    ConsumeWindowIntoColumn,
    ExpelWindowFromColumn,
    SwapWindowInDirection(#[proptest(strategy = "arbitrary_scroll_direction()")] ScrollDirection),
    ToggleColumnTabbedDisplay,
    SetColumnDisplay(#[proptest(strategy = "arbitrary_column_display()")] ColumnDisplay),
    CenterColumn,
    CenterWindow {
        #[proptest(strategy = "proptest::option::of(1..=5usize)")]
        id: Option<usize>,
    },
    CenterVisibleColumns,
    FocusWorkspaceDown,
    FocusWorkspaceUp,
    FocusWorkspace(#[proptest(strategy = "0..=4usize")] usize),
    FocusWorkspaceAutoBackAndForth(#[proptest(strategy = "0..=4usize")] usize),
    FocusWorkspacePrevious,
    MoveWindowToWorkspaceDown(bool),
    MoveWindowToWorkspaceUp(bool),
    MoveWindowToWorkspace {
        #[proptest(strategy = "proptest::option::of(1..=5usize)")]
        window_id: Option<usize>,
        #[proptest(strategy = "0..=4usize")]
        workspace_idx: usize,
    },
    MoveColumnToWorkspaceDown(bool),
    MoveColumnToWorkspaceUp(bool),
    MoveColumnToWorkspace(#[proptest(strategy = "0..=4usize")] usize, bool),
    MoveWorkspaceDown,
    MoveWorkspaceUp,
    MoveWorkspaceToIndex {
        #[proptest(strategy = "proptest::option::of(1..=5usize)")]
        ws_name: Option<usize>,
        #[proptest(strategy = "0..=4usize")]
        target_idx: usize,
    },
    MoveWorkspaceToMonitor {
        #[proptest(strategy = "proptest::option::of(1..=5usize)")]
        ws_name: Option<usize>,
        #[proptest(strategy = "0..=5usize")]
        output_id: usize,
    },
    SetWorkspaceName {
        #[proptest(strategy = "1..=5usize")]
        new_ws_name: usize,
        #[proptest(strategy = "proptest::option::of(1..=5usize)")]
        ws_name: Option<usize>,
    },
    UnsetWorkspaceName {
        #[proptest(strategy = "proptest::option::of(1..=5usize)")]
        ws_name: Option<usize>,
    },
    MoveWindowToOutput {
        #[proptest(strategy = "proptest::option::of(1..=5usize)")]
        window_id: Option<usize>,
        #[proptest(strategy = "1..=5usize")]
        output_id: usize,
        #[proptest(strategy = "proptest::option::of(0..=4usize)")]
        target_ws_idx: Option<usize>,
    },
    MoveColumnToOutput {
        #[proptest(strategy = "1..=5usize")]
        output_id: usize,
        #[proptest(strategy = "proptest::option::of(0..=4usize)")]
        target_ws_idx: Option<usize>,
        activate: bool,
    },
    SwitchPresetColumnWidth,
    SwitchPresetColumnWidthBack,
    SwitchPresetWindowWidth {
        #[proptest(strategy = "proptest::option::of(1..=5usize)")]
        id: Option<usize>,
    },
    SwitchPresetWindowWidthBack {
        #[proptest(strategy = "proptest::option::of(1..=5usize)")]
        id: Option<usize>,
    },
    SwitchPresetWindowHeight {
        #[proptest(strategy = "proptest::option::of(1..=5usize)")]
        id: Option<usize>,
    },
    SwitchPresetWindowHeightBack {
        #[proptest(strategy = "proptest::option::of(1..=5usize)")]
        id: Option<usize>,
    },
    MaximizeColumn,
    MaximizeWindowToEdges {
        #[proptest(strategy = "proptest::option::of(1..=5usize)")]
        id: Option<usize>,
    },
    SetColumnWidth(#[proptest(strategy = "arbitrary_size_change()")] SizeChange),
    SetWindowWidth {
        #[proptest(strategy = "proptest::option::of(1..=5usize)")]
        id: Option<usize>,
        #[proptest(strategy = "arbitrary_size_change()")]
        change: SizeChange,
    },
    SetWindowHeight {
        #[proptest(strategy = "proptest::option::of(1..=5usize)")]
        id: Option<usize>,
        #[proptest(strategy = "arbitrary_size_change()")]
        change: SizeChange,
    },
    ResetWindowHeight {
        #[proptest(strategy = "proptest::option::of(1..=5usize)")]
        id: Option<usize>,
    },
    ExpandColumnToAvailableWidth,
    ToggleWindowFloating {
        #[proptest(strategy = "proptest::option::of(1..=5usize)")]
        id: Option<usize>,
    },
    SetWindowFloating {
        #[proptest(strategy = "proptest::option::of(1..=5usize)")]
        id: Option<usize>,
        floating: bool,
    },
    FocusFloating,
    FocusTiling,
    SwitchFocusFloatingTiling,
    MoveFloatingWindow {
        #[proptest(strategy = "proptest::option::of(1..=5usize)")]
        id: Option<usize>,
        #[proptest(strategy = "arbitrary_position_change()")]
        x: PositionChange,
        #[proptest(strategy = "arbitrary_position_change()")]
        y: PositionChange,
        animate: bool,
    },
    SetParent {
        #[proptest(strategy = "1..=5usize")]
        id: usize,
        #[proptest(strategy = "prop::option::of(1..=5usize)")]
        new_parent_id: Option<usize>,
    },
    SetForcedSize {
        #[proptest(strategy = "1..=5usize")]
        id: usize,
        #[proptest(strategy = "proptest::option::of(arbitrary_size())")]
        size: Option<Size<i32, Logical>>,
    },
    Communicate(#[proptest(strategy = "1..=5usize")] usize),
    Refresh {
        is_active: bool,
    },
    AdvanceAnimations {
        #[proptest(strategy = "arbitrary_msec_delta()")]
        msec_delta: i32,
    },
    CompleteAnimations,
    MoveWorkspaceToOutput(#[proptest(strategy = "1..=5usize")] usize),
    ViewOffsetGestureBegin {
        #[proptest(strategy = "1..=5usize")]
        output_idx: usize,
        #[proptest(strategy = "proptest::option::of(0..=4usize)")]
        workspace_idx: Option<usize>,
        is_touchpad: bool,
    },
    ViewOffsetGestureUpdate {
        #[proptest(strategy = "arbitrary_view_offset_gesture_delta()")]
        delta: f64,
        timestamp: Duration,
        is_touchpad: bool,
    },
    ViewOffsetGestureEnd {
        is_touchpad: Option<bool>,
    },
    WorkspaceSwitchGestureBegin {
        #[proptest(strategy = "1..=5usize")]
        output_idx: usize,
        is_touchpad: bool,
    },
    WorkspaceSwitchGestureUpdate {
        #[proptest(strategy = "-400f64..400f64")]
        delta: f64,
        timestamp: Duration,
        is_touchpad: bool,
    },
    WorkspaceSwitchGestureEnd {
        is_touchpad: Option<bool>,
    },
    OverviewGestureBegin,
    OverviewGestureUpdate {
        #[proptest(strategy = "-400f64..400f64")]
        delta: f64,
        timestamp: Duration,
    },
    OverviewGestureEnd,
    InteractiveMoveBegin {
        #[proptest(strategy = "1..=5usize")]
        window: usize,
        #[proptest(strategy = "1..=5usize")]
        output_idx: usize,
        #[proptest(strategy = "-20000f64..20000f64")]
        px: f64,
        #[proptest(strategy = "-20000f64..20000f64")]
        py: f64,
    },
    InteractiveMoveUpdate {
        #[proptest(strategy = "1..=5usize")]
        window: usize,
        #[proptest(strategy = "-20000f64..20000f64")]
        dx: f64,
        #[proptest(strategy = "-20000f64..20000f64")]
        dy: f64,
        #[proptest(strategy = "1..=5usize")]
        output_idx: usize,
        #[proptest(strategy = "-20000f64..20000f64")]
        px: f64,
        #[proptest(strategy = "-20000f64..20000f64")]
        py: f64,
    },
    InteractiveMoveEnd {
        #[proptest(strategy = "1..=5usize")]
        window: usize,
    },
    DndUpdate {
        #[proptest(strategy = "1..=5usize")]
        output_idx: usize,
        #[proptest(strategy = "-20000f64..20000f64")]
        px: f64,
        #[proptest(strategy = "-20000f64..20000f64")]
        py: f64,
    },
    DndEnd,
    InteractiveResizeBegin {
        #[proptest(strategy = "1..=5usize")]
        window: usize,
        #[proptest(strategy = "arbitrary_resize_edge()")]
        edges: ResizeEdge,
    },
    InteractiveResizeUpdate {
        #[proptest(strategy = "1..=5usize")]
        window: usize,
        #[proptest(strategy = "-20000f64..20000f64")]
        dx: f64,
        #[proptest(strategy = "-20000f64..20000f64")]
        dy: f64,
    },
    InteractiveResizeEnd {
        #[proptest(strategy = "1..=5usize")]
        window: usize,
    },
    ToggleOverview,
    UpdateConfig {
        #[proptest(strategy = "arbitrary_layout_part().prop_map(Box::new)")]
        layout_config: Box<niri_config::LayoutPart>,
    },
}

impl Op {
    fn apply(self, layout: &mut Layout<TestWindow>) {
        match self {
            Op::AddOutput(id) => {
                let name = format!("output{id}");
                if layout.outputs().any(|o| o.name() == name) {
                    return;
                }

                let output = Output::new(
                    name.clone(),
                    PhysicalProperties {
                        size: Size::from((1280, 720)),
                        subpixel: Subpixel::Unknown,
                        make: String::new(),
                        model: String::new(),
                        serial_number: String::new(),
                    },
                );
                output.change_current_state(
                    Some(Mode {
                        size: Size::from((1280, 720)),
                        refresh: 60000,
                    }),
                    None,
                    None,
                    None,
                );
                output.user_data().insert_if_missing(|| OutputName {
                    connector: name,
                    make: None,
                    model: None,
                    serial: None,
                });
                layout.add_output(output.clone(), None);
            }
            Op::AddScaledOutput {
                id,
                scale,
                layout_config,
            } => {
                let name = format!("output{id}");
                if layout.outputs().any(|o| o.name() == name) {
                    return;
                }

                let output = Output::new(
                    name.clone(),
                    PhysicalProperties {
                        size: Size::from((1280, 720)),
                        subpixel: Subpixel::Unknown,
                        make: String::new(),
                        model: String::new(),
                        serial_number: String::new(),
                    },
                );
                output.change_current_state(
                    Some(Mode {
                        size: Size::from((1280, 720)),
                        refresh: 60000,
                    }),
                    None,
                    Some(smithay::output::Scale::Fractional(scale)),
                    None,
                );
                output.user_data().insert_if_missing(|| OutputName {
                    connector: name,
                    make: None,
                    model: None,
                    serial: None,
                });
                layout.add_output(output.clone(), layout_config.map(|x| *x));
            }
            Op::RemoveOutput(id) => {
                let name = format!("output{id}");
                let Some(output) = layout.outputs().find(|o| o.name() == name).cloned() else {
                    return;
                };

                layout.remove_output(&output);
            }
            Op::FocusOutput(id) => {
                let name = format!("output{id}");
                let Some(output) = layout.outputs().find(|o| o.name() == name).cloned() else {
                    return;
                };

                layout.focus_output(&output);
            }
            Op::UpdateOutputLayoutConfig { id, layout_config } => {
                let name = format!("output{id}");
                let Some(mon) = layout.monitors_mut().find(|m| m.output_name() == &name) else {
                    return;
                };

                mon.update_layout_config(layout_config.map(|x| *x));
            }
            Op::AddNamedWorkspace {
                ws_name,
                output_name,
                layout_config,
            } => {
                layout.ensure_named_workspace(&WorkspaceConfig {
                    name: WorkspaceName(format!("ws{ws_name}")),
                    open_on_output: output_name.map(|name| format!("output{name}")),
                    layout: layout_config.map(|x| niri_config::WorkspaceLayoutPart(*x)),
                });
            }
            Op::UnnameWorkspace { ws_name } => {
                layout.unname_workspace(&format!("ws{ws_name}"));
            }
            Op::UpdateWorkspaceLayoutConfig {
                ws_name,
                layout_config,
            } => {
                let ws_name = format!("ws{ws_name}");
                let Some(ws) = layout
                    .workspaces_mut()
                    .find(|ws| ws.name() == Some(&ws_name))
                else {
                    return;
                };

                ws.update_layout_config(layout_config.map(|x| *x));
            }
            Op::SetWorkspaceName {
                new_ws_name,
                ws_name,
            } => {
                let ws_ref =
                    ws_name.map(|ws_name| WorkspaceReference::Name(format!("ws{ws_name}")));
                layout.set_workspace_name(format!("ws{new_ws_name}"), ws_ref);
            }
            Op::UnsetWorkspaceName { ws_name } => {
                let ws_ref =
                    ws_name.map(|ws_name| WorkspaceReference::Name(format!("ws{ws_name}")));
                layout.unset_workspace_name(ws_ref);
            }
            Op::AddWindow { mut params } => {
                if layout.has_window(&params.id) {
                    return;
                }
                if let Some(parent_id) = params.parent_id {
                    if parent_id_causes_loop(layout, params.id, parent_id) {
                        params.parent_id = None;
                    }
                }

                let is_floating = params.is_floating;
                let win = TestWindow::new(params);
                // Match the real compositor: resolve layout/sidebar default column width.
                let width = layout
                    .active_workspace()
                    .map(|ws| ws.resolve_default_width(None, is_floating))
                    .flatten();
                layout.add_window(
                    win,
                    AddWindowTarget::Auto,
                    width,
                    None,
                    false,
                    is_floating,
                    ActivateWindow::default(),
                );
            }
            Op::AddWindowNextTo {
                mut params,
                next_to_id,
            } => {
                let mut found_next_to = false;

                if let Some(InteractiveMoveState::Moving(move_)) = &layout.interactive_move {
                    let win_id = move_.tile.window().0.id;
                    if win_id == params.id {
                        return;
                    }
                    if win_id == next_to_id {
                        found_next_to = true;
                    }
                }

                match &mut layout.monitor_set {
                    MonitorSet::Normal { monitors, .. } => {
                        for mon in monitors {
                            for ws in &mut mon.workspaces {
                                for win in ws.windows() {
                                    if win.0.id == params.id {
                                        return;
                                    }

                                    if win.0.id == next_to_id {
                                        found_next_to = true;
                                    }
                                }
                            }
                        }
                    }
                    MonitorSet::NoOutputs { workspaces, .. } => {
                        for ws in workspaces {
                            for win in ws.windows() {
                                if win.0.id == params.id {
                                    return;
                                }

                                if win.0.id == next_to_id {
                                    found_next_to = true;
                                }
                            }
                        }
                    }
                }

                if !found_next_to {
                    return;
                }

                if let Some(parent_id) = params.parent_id {
                    if parent_id_causes_loop(layout, params.id, parent_id) {
                        params.parent_id = None;
                    }
                }

                let is_floating = params.is_floating;
                let win = TestWindow::new(params);
                layout.add_window(
                    win,
                    AddWindowTarget::NextTo(&next_to_id),
                    None,
                    None,
                    false,
                    is_floating,
                    ActivateWindow::default(),
                );
            }
            Op::AddWindowToNamedWorkspace {
                mut params,
                ws_name,
            } => {
                let ws_name = format!("ws{ws_name}");
                let mut ws_id = None;

                if let Some(InteractiveMoveState::Moving(move_)) = &layout.interactive_move {
                    if move_.tile.window().0.id == params.id {
                        return;
                    }
                }

                match &mut layout.monitor_set {
                    MonitorSet::Normal { monitors, .. } => {
                        for mon in monitors {
                            for ws in &mut mon.workspaces {
                                for win in ws.windows() {
                                    if win.0.id == params.id {
                                        return;
                                    }
                                }

                                if ws
                                    .name
                                    .as_ref()
                                    .is_some_and(|name| name.eq_ignore_ascii_case(&ws_name))
                                {
                                    ws_id = Some(ws.id());
                                }
                            }
                        }
                    }
                    MonitorSet::NoOutputs { workspaces, .. } => {
                        for ws in workspaces {
                            for win in ws.windows() {
                                if win.0.id == params.id {
                                    return;
                                }
                            }

                            if ws
                                .name
                                .as_ref()
                                .is_some_and(|name| name.eq_ignore_ascii_case(&ws_name))
                            {
                                ws_id = Some(ws.id());
                            }
                        }
                    }
                }

                let Some(ws_id) = ws_id else {
                    return;
                };

                if let Some(parent_id) = params.parent_id {
                    if parent_id_causes_loop(layout, params.id, parent_id) {
                        params.parent_id = None;
                    }
                }

                let is_floating = params.is_floating;
                let win = TestWindow::new(params);
                layout.add_window(
                    win,
                    AddWindowTarget::Workspace(ws_id),
                    None,
                    None,
                    false,
                    is_floating,
                    ActivateWindow::default(),
                );
            }
            Op::CloseWindow(id) => {
                layout.remove_window(&id, Transaction::new());
            }
            Op::FullscreenWindow(id) => {
                if !layout.has_window(&id) {
                    return;
                }
                layout.toggle_fullscreen(&id);
            }
            Op::SetFullscreenWindow {
                window,
                is_fullscreen,
            } => {
                if !layout.has_window(&window) {
                    return;
                }
                layout.set_fullscreen(&window, is_fullscreen);
            }
            Op::ToggleWindowedFullscreen(id) => {
                if !layout.has_window(&id) {
                    return;
                }
                layout.toggle_windowed_fullscreen(&id);
            }
            Op::FocusColumnLeft => layout.focus_left(),
            Op::FocusColumnRight => layout.focus_right(),
            Op::FocusColumnFirst => layout.focus_column_first(),
            Op::FocusColumnLast => layout.focus_column_last(),
            Op::FocusColumnRightOrFirst => layout.focus_column_right_or_first(),
            Op::FocusColumnLeftOrLast => layout.focus_column_left_or_last(),
            Op::FocusColumn(index) => layout.focus_column(index),
            Op::FocusWindowOrMonitorUp(id) => {
                let name = format!("output{id}");
                let Some(output) = layout.outputs().find(|o| o.name() == name).cloned() else {
                    return;
                };

                layout.focus_window_up_or_output(&output);
            }
            Op::FocusWindowOrMonitorDown(id) => {
                let name = format!("output{id}");
                let Some(output) = layout.outputs().find(|o| o.name() == name).cloned() else {
                    return;
                };

                layout.focus_window_down_or_output(&output);
            }
            Op::FocusColumnOrMonitorLeft(id) => {
                let name = format!("output{id}");
                let Some(output) = layout.outputs().find(|o| o.name() == name).cloned() else {
                    return;
                };

                layout.focus_column_left_or_output(&output);
            }
            Op::FocusColumnOrMonitorRight(id) => {
                let name = format!("output{id}");
                let Some(output) = layout.outputs().find(|o| o.name() == name).cloned() else {
                    return;
                };

                layout.focus_column_right_or_output(&output);
            }
            Op::FocusWindowDown => {
                layout.focus_down();
            }
            Op::FocusWindowUp => {
                layout.focus_up();
            }
            Op::FocusWindowDownOrColumnLeft => layout.focus_down_or_left(),
            Op::FocusWindowDownOrColumnRight => layout.focus_down_or_right(),
            Op::FocusWindowUpOrColumnLeft => layout.focus_up_or_left(),
            Op::FocusWindowUpOrColumnRight => layout.focus_up_or_right(),
            Op::FocusWindowOrWorkspaceDown => layout.focus_window_or_workspace_down(),
            Op::FocusWindowOrWorkspaceUp => layout.focus_window_or_workspace_up(),
            Op::FocusWindow(id) => layout.activate_window(&id),
            Op::FocusWindowInColumn(index) => layout.focus_window_in_column(index),
            Op::FocusWindowTop => layout.focus_window_top(),
            Op::FocusWindowBottom => layout.focus_window_bottom(),
            Op::FocusWindowDownOrTop => layout.focus_window_down_or_top(),
            Op::FocusWindowUpOrBottom => layout.focus_window_up_or_bottom(),
            Op::MoveColumnLeft => layout.move_left(),
            Op::MoveColumnRight => layout.move_right(),
            Op::MoveColumnToFirst => layout.move_column_to_first(),
            Op::MoveColumnToLast => layout.move_column_to_last(),
            Op::MoveColumnLeftOrToMonitorLeft(id) => {
                let name = format!("output{id}");
                let Some(output) = layout.outputs().find(|o| o.name() == name).cloned() else {
                    return;
                };

                layout.move_column_left_or_to_output(&output);
            }
            Op::MoveColumnRightOrToMonitorRight(id) => {
                let name = format!("output{id}");
                let Some(output) = layout.outputs().find(|o| o.name() == name).cloned() else {
                    return;
                };

                layout.move_column_right_or_to_output(&output);
            }
            Op::MoveColumnToIndex(index) => layout.move_column_to_index(index),
            Op::MoveWindowDown => layout.move_down(),
            Op::MoveWindowUp => layout.move_up(),
            Op::MoveWindowDownOrToWorkspaceDown => layout.move_down_or_to_workspace_down(),
            Op::MoveWindowUpOrToWorkspaceUp => layout.move_up_or_to_workspace_up(),
            Op::ConsumeOrExpelWindowLeft { id } => {
                let id = id.filter(|id| layout.has_window(id));
                layout.consume_or_expel_window_left(id.as_ref());
            }
            Op::ConsumeOrExpelWindowRight { id } => {
                let id = id.filter(|id| layout.has_window(id));
                layout.consume_or_expel_window_right(id.as_ref());
            }
            Op::ConsumeWindowIntoColumn => layout.consume_into_column(),
            Op::ExpelWindowFromColumn => layout.expel_from_column(),
            Op::SwapWindowInDirection(direction) => layout.swap_window_in_direction(direction),
            Op::ToggleColumnTabbedDisplay => layout.toggle_column_tabbed_display(),
            Op::SetColumnDisplay(display) => layout.set_column_display(display),
            Op::CenterColumn => layout.center_column(),
            Op::CenterWindow { id } => {
                let id = id.filter(|id| layout.has_window(id));
                layout.center_window(id.as_ref());
            }
            Op::CenterVisibleColumns => layout.center_visible_columns(),
            Op::FocusWorkspaceDown => layout.switch_workspace_down(),
            Op::FocusWorkspaceUp => layout.switch_workspace_up(),
            Op::FocusWorkspace(idx) => layout.switch_workspace(idx),
            Op::FocusWorkspaceAutoBackAndForth(idx) => {
                layout.switch_workspace_auto_back_and_forth(idx)
            }
            Op::FocusWorkspacePrevious => layout.switch_workspace_previous(),
            Op::MoveWindowToWorkspaceDown(focus) => layout.move_to_workspace_down(focus),
            Op::MoveWindowToWorkspaceUp(focus) => layout.move_to_workspace_up(focus),
            Op::MoveWindowToWorkspace {
                window_id,
                workspace_idx,
            } => {
                let window_id = window_id.filter(|id| layout.has_window(id));
                layout.move_to_workspace(window_id.as_ref(), workspace_idx, ActivateWindow::Smart);
            }
            Op::MoveColumnToWorkspaceDown(focus) => layout.move_column_to_workspace_down(focus),
            Op::MoveColumnToWorkspaceUp(focus) => layout.move_column_to_workspace_up(focus),
            Op::MoveColumnToWorkspace(idx, focus) => layout.move_column_to_workspace(idx, focus),
            Op::MoveWindowToOutput {
                window_id,
                output_id: id,
                target_ws_idx,
            } => {
                let name = format!("output{id}");
                let Some(output) = layout.outputs().find(|o| o.name() == name).cloned() else {
                    return;
                };
                let mon = layout.monitor_for_output(&output).unwrap();

                let window_id = window_id.filter(|id| layout.has_window(id));
                let target_ws_idx = target_ws_idx.filter(|idx| mon.workspaces.len() > *idx);
                layout.move_to_output(
                    window_id.as_ref(),
                    &output,
                    target_ws_idx,
                    ActivateWindow::Smart,
                );
            }
            Op::MoveColumnToOutput {
                output_id: id,
                target_ws_idx,
                activate,
            } => {
                let name = format!("output{id}");
                let Some(output) = layout.outputs().find(|o| o.name() == name).cloned() else {
                    return;
                };

                layout.move_column_to_output(&output, target_ws_idx, activate);
            }
            Op::MoveWorkspaceDown => layout.move_workspace_down(),
            Op::MoveWorkspaceUp => layout.move_workspace_up(),
            Op::MoveWorkspaceToIndex {
                ws_name: Some(ws_name),
                target_idx,
            } => {
                let MonitorSet::Normal { monitors, .. } = &mut layout.monitor_set else {
                    return;
                };

                let Some((old_idx, old_output)) = monitors.iter().find_map(|monitor| {
                    monitor
                        .workspaces
                        .iter()
                        .enumerate()
                        .find_map(|(i, ws)| {
                            if ws.name == Some(format!("ws{ws_name}")) {
                                Some(i)
                            } else {
                                None
                            }
                        })
                        .map(|i| (i, monitor.output.clone()))
                }) else {
                    return;
                };

                layout.move_workspace_to_idx(Some((Some(old_output), old_idx)), target_idx)
            }
            Op::MoveWorkspaceToIndex {
                ws_name: None,
                target_idx,
            } => layout.move_workspace_to_idx(None, target_idx),
            Op::MoveWorkspaceToMonitor {
                ws_name: None,
                output_id: id,
            } => {
                let name = format!("output{id}");
                let Some(output) = layout.outputs().find(|o| o.name() == name).cloned() else {
                    return;
                };
                layout.move_workspace_to_output(&output);
            }
            Op::MoveWorkspaceToMonitor {
                ws_name: Some(ws_name),
                output_id: id,
            } => {
                let name = format!("output{id}");
                let Some(output) = layout.outputs().find(|o| o.name() == name).cloned() else {
                    return;
                };
                let MonitorSet::Normal { monitors, .. } = &mut layout.monitor_set else {
                    return;
                };

                let Some((old_idx, old_output)) = monitors.iter().find_map(|monitor| {
                    monitor
                        .workspaces
                        .iter()
                        .enumerate()
                        .find_map(|(i, ws)| {
                            if ws.name == Some(format!("ws{ws_name}")) {
                                Some(i)
                            } else {
                                None
                            }
                        })
                        .map(|i| (i, monitor.output.clone()))
                }) else {
                    return;
                };

                layout.move_workspace_to_output_by_id(old_idx, Some(old_output), &output);
            }
            Op::SwitchPresetColumnWidth => layout.toggle_width(true),
            Op::SwitchPresetColumnWidthBack => layout.toggle_width(false),
            Op::SwitchPresetWindowWidth { id } => {
                let id = id.filter(|id| layout.has_window(id));
                layout.toggle_window_width(id.as_ref(), true);
            }
            Op::SwitchPresetWindowWidthBack { id } => {
                let id = id.filter(|id| layout.has_window(id));
                layout.toggle_window_width(id.as_ref(), false);
            }
            Op::SwitchPresetWindowHeight { id } => {
                let id = id.filter(|id| layout.has_window(id));
                layout.toggle_window_height(id.as_ref(), true);
            }
            Op::SwitchPresetWindowHeightBack { id } => {
                let id = id.filter(|id| layout.has_window(id));
                layout.toggle_window_height(id.as_ref(), false);
            }
            Op::MaximizeColumn => layout.toggle_full_width(),
            Op::MaximizeWindowToEdges { id } => {
                let id = id.or_else(|| layout.focus().map(|win| *win.id()));
                let Some(id) = id else {
                    return;
                };
                if !layout.has_window(&id) {
                    return;
                }
                layout.toggle_maximized(&id);
            }
            Op::SetColumnWidth(change) => layout.set_column_width(change),
            Op::SetWindowWidth { id, change } => {
                let id = id.filter(|id| layout.has_window(id));
                layout.set_window_width(id.as_ref(), change);
            }
            Op::SetWindowHeight { id, change } => {
                let id = id.filter(|id| layout.has_window(id));
                layout.set_window_height(id.as_ref(), change);
            }
            Op::ResetWindowHeight { id } => {
                let id = id.filter(|id| layout.has_window(id));
                layout.reset_window_height(id.as_ref());
            }
            Op::ExpandColumnToAvailableWidth => layout.expand_column_to_available_width(),
            Op::ToggleWindowFloating { id } => {
                let id = id.filter(|id| layout.has_window(id));
                layout.toggle_window_floating(id.as_ref());
            }
            Op::SetWindowFloating { id, floating } => {
                let id = id.filter(|id| layout.has_window(id));
                layout.set_window_floating(id.as_ref(), floating);
            }
            Op::FocusFloating => {
                layout.focus_floating();
            }
            Op::FocusTiling => {
                layout.focus_tiling();
            }
            Op::SwitchFocusFloatingTiling => {
                layout.switch_focus_floating_tiling();
            }
            Op::MoveFloatingWindow { id, x, y, animate } => {
                let id = id.filter(|id| layout.has_window(id));
                layout.move_floating_window(id.as_ref(), x, y, animate);
            }
            Op::SetParent {
                id,
                mut new_parent_id,
            } => {
                if !layout.has_window(&id) {
                    return;
                }

                if let Some(parent_id) = new_parent_id {
                    if parent_id_causes_loop(layout, id, parent_id) {
                        new_parent_id = None;
                    }
                }

                let mut update = false;

                if let Some(InteractiveMoveState::Moving(move_)) = &layout.interactive_move {
                    if move_.tile.window().0.id == id {
                        move_.tile.window().0.parent_id.set(new_parent_id);
                        update = true;
                    }
                }

                match &mut layout.monitor_set {
                    MonitorSet::Normal { monitors, .. } => {
                        'outer: for mon in monitors {
                            for ws in &mut mon.workspaces {
                                for win in ws.windows() {
                                    if win.0.id == id {
                                        win.0.parent_id.set(new_parent_id);
                                        update = true;
                                        break 'outer;
                                    }
                                }
                            }
                        }
                    }
                    MonitorSet::NoOutputs { workspaces, .. } => {
                        'outer: for ws in workspaces {
                            for win in ws.windows() {
                                if win.0.id == id {
                                    win.0.parent_id.set(new_parent_id);
                                    update = true;
                                    break 'outer;
                                }
                            }
                        }
                    }
                }

                if update {
                    if let Some(new_parent_id) = new_parent_id {
                        layout.descendants_added(&new_parent_id);
                    }
                }
            }
            Op::SetForcedSize { id, size } => {
                for (_mon, win) in layout.windows() {
                    if win.0.id == id {
                        win.0.forced_size.set(size);
                        return;
                    }
                }
            }
            Op::Communicate(id) => {
                let mut update = false;

                if let Some(InteractiveMoveState::Moving(move_)) = &layout.interactive_move {
                    if move_.tile.window().0.id == id {
                        if move_.tile.window().communicate() {
                            update = true;
                        }

                        if update {
                            // FIXME: serial.
                            layout.update_window(&id, None);
                        }
                        return;
                    }
                }

                match &mut layout.monitor_set {
                    MonitorSet::Normal { monitors, .. } => {
                        'outer: for mon in monitors {
                            for ws in &mut mon.workspaces {
                                for win in ws.windows() {
                                    if win.0.id == id {
                                        if win.communicate() {
                                            update = true;
                                        }
                                        break 'outer;
                                    }
                                }
                            }
                        }
                    }
                    MonitorSet::NoOutputs { workspaces, .. } => {
                        'outer: for ws in workspaces {
                            for win in ws.windows() {
                                if win.0.id == id {
                                    if win.communicate() {
                                        update = true;
                                    }
                                    break 'outer;
                                }
                            }
                        }
                    }
                }

                if update {
                    // FIXME: serial.
                    layout.update_window(&id, None);
                }
            }
            Op::Refresh { is_active } => {
                layout.refresh(is_active);
            }
            Op::AdvanceAnimations { msec_delta } => {
                let mut now = layout.clock.now_unadjusted();
                if msec_delta >= 0 {
                    now = now.saturating_add(Duration::from_millis(msec_delta as u64));
                } else {
                    now = now.saturating_sub(Duration::from_millis(-msec_delta as u64));
                }
                layout.clock.set_unadjusted(now);
                layout.advance_animations();
            }
            Op::CompleteAnimations => {
                layout.clock.set_complete_instantly(true);
                layout.advance_animations();
                layout.clock.set_complete_instantly(false);
            }
            Op::MoveWorkspaceToOutput(id) => {
                let name = format!("output{id}");
                let Some(output) = layout.outputs().find(|o| o.name() == name).cloned() else {
                    return;
                };

                layout.move_workspace_to_output(&output);
            }
            Op::ViewOffsetGestureBegin {
                output_idx: id,
                workspace_idx,
                is_touchpad: normalize,
            } => {
                let name = format!("output{id}");
                let Some(output) = layout.outputs().find(|o| o.name() == name).cloned() else {
                    return;
                };

                layout.view_offset_gesture_begin(&output, workspace_idx, normalize);
            }
            Op::ViewOffsetGestureUpdate {
                delta,
                timestamp,
                is_touchpad,
            } => {
                layout.view_offset_gesture_update(delta, timestamp, is_touchpad);
            }
            Op::ViewOffsetGestureEnd { is_touchpad } => {
                layout.view_offset_gesture_end(is_touchpad);
            }
            Op::WorkspaceSwitchGestureBegin {
                output_idx: id,
                is_touchpad,
            } => {
                let name = format!("output{id}");
                let Some(output) = layout.outputs().find(|o| o.name() == name).cloned() else {
                    return;
                };

                layout.workspace_switch_gesture_begin(&output, is_touchpad);
            }
            Op::WorkspaceSwitchGestureUpdate {
                delta,
                timestamp,
                is_touchpad,
            } => {
                layout.workspace_switch_gesture_update(delta, timestamp, is_touchpad);
            }
            Op::WorkspaceSwitchGestureEnd { is_touchpad } => {
                layout.workspace_switch_gesture_end(is_touchpad);
            }
            Op::OverviewGestureBegin => {
                layout.overview_gesture_begin();
            }
            Op::OverviewGestureUpdate { delta, timestamp } => {
                layout.overview_gesture_update(delta, timestamp);
            }
            Op::OverviewGestureEnd => {
                layout.overview_gesture_end();
            }
            Op::InteractiveMoveBegin {
                window,
                output_idx,
                px,
                py,
            } => {
                let name = format!("output{output_idx}");
                let Some(output) = layout.outputs().find(|o| o.name() == name).cloned() else {
                    return;
                };
                layout.interactive_move_begin(window, &output, Point::from((px, py)));
            }
            Op::InteractiveMoveUpdate {
                window,
                dx,
                dy,
                output_idx,
                px,
                py,
            } => {
                let name = format!("output{output_idx}");
                let Some(output) = layout.outputs().find(|o| o.name() == name).cloned() else {
                    return;
                };
                layout.interactive_move_update(
                    &window,
                    Point::from((dx, dy)),
                    output,
                    Point::from((px, py)),
                );
            }
            Op::InteractiveMoveEnd { window } => {
                layout.interactive_move_end(&window);
            }
            Op::DndUpdate { output_idx, px, py } => {
                let name = format!("output{output_idx}");
                let Some(output) = layout.outputs().find(|o| o.name() == name).cloned() else {
                    return;
                };
                layout.dnd_update(output, Point::from((px, py)));
            }
            Op::DndEnd => {
                layout.dnd_end();
            }
            Op::InteractiveResizeBegin { window, edges } => {
                layout.interactive_resize_begin(window, edges);
            }
            Op::InteractiveResizeUpdate { window, dx, dy } => {
                layout.interactive_resize_update(&window, Point::from((dx, dy)));
            }
            Op::InteractiveResizeEnd { window } => {
                layout.interactive_resize_end(&window);
            }
            Op::ToggleOverview => {
                layout.toggle_overview();
            }
            Op::UpdateConfig { layout_config } => {
                let options = Options {
                    layout: niri_config::Layout::from_part(&layout_config),
                    ..Default::default()
                };

                layout.update_options(options);
            }
        }
    }
}

#[track_caller]
fn check_ops_on_layout(layout: &mut Layout<TestWindow>, ops: impl IntoIterator<Item = Op>) {
    for op in ops {
        op.apply(layout);
        layout.verify_invariants();
    }
}

#[track_caller]
fn check_ops(ops: impl IntoIterator<Item = Op>) -> Layout<TestWindow> {
    let mut layout = Layout::default();
    check_ops_on_layout(&mut layout, ops);
    layout
}

#[track_caller]
fn check_ops_with_options(
    options: Options,
    ops: impl IntoIterator<Item = Op>,
) -> Layout<TestWindow> {
    let mut layout = Layout::with_options(Clock::with_time(Duration::ZERO), options);
    check_ops_on_layout(&mut layout, ops);
    layout
}

#[test]
fn operations_dont_panic() {
    if std::env::var_os("RUN_SLOW_TESTS").is_none() {
        eprintln!("ignoring slow test");
        return;
    }

    let every_op = [
        Op::AddOutput(0),
        Op::AddOutput(1),
        Op::AddOutput(2),
        Op::RemoveOutput(0),
        Op::RemoveOutput(1),
        Op::RemoveOutput(2),
        Op::FocusOutput(0),
        Op::FocusOutput(1),
        Op::FocusOutput(2),
        Op::AddNamedWorkspace {
            ws_name: 1,
            output_name: Some(1),
            layout_config: None,
        },
        Op::UnnameWorkspace { ws_name: 1 },
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindowNextTo {
            params: TestWindowParams::new(2),
            next_to_id: 1,
        },
        Op::AddWindowToNamedWorkspace {
            params: TestWindowParams::new(3),
            ws_name: 1,
        },
        Op::CloseWindow(0),
        Op::CloseWindow(1),
        Op::CloseWindow(2),
        Op::FullscreenWindow(1),
        Op::FullscreenWindow(2),
        Op::FullscreenWindow(3),
        Op::MaximizeWindowToEdges { id: Some(1) },
        Op::MaximizeWindowToEdges { id: Some(2) },
        Op::MaximizeWindowToEdges { id: Some(3) },
        Op::FocusColumnLeft,
        Op::FocusColumnRight,
        Op::FocusColumnRightOrFirst,
        Op::FocusColumnLeftOrLast,
        Op::FocusWindowOrMonitorUp(0),
        Op::FocusWindowOrMonitorDown(1),
        Op::FocusColumnOrMonitorLeft(0),
        Op::FocusColumnOrMonitorRight(1),
        Op::FocusWindowUp,
        Op::FocusWindowUpOrColumnLeft,
        Op::FocusWindowUpOrColumnRight,
        Op::FocusWindowOrWorkspaceUp,
        Op::FocusWindowDown,
        Op::FocusWindowDownOrColumnLeft,
        Op::FocusWindowDownOrColumnRight,
        Op::FocusWindowOrWorkspaceDown,
        Op::MoveColumnLeft,
        Op::MoveColumnRight,
        Op::MoveColumnLeftOrToMonitorLeft(0),
        Op::MoveColumnRightOrToMonitorRight(1),
        Op::ConsumeWindowIntoColumn,
        Op::ExpelWindowFromColumn,
        Op::CenterColumn,
        Op::FocusWorkspaceDown,
        Op::FocusWorkspaceUp,
        Op::FocusWorkspace(1),
        Op::FocusWorkspace(2),
        Op::MoveWindowToWorkspaceDown(true),
        Op::MoveWindowToWorkspaceUp(true),
        Op::MoveWindowToWorkspace {
            window_id: None,
            workspace_idx: 1,
        },
        Op::MoveWindowToWorkspace {
            window_id: None,
            workspace_idx: 2,
        },
        Op::MoveColumnToWorkspaceDown(true),
        Op::MoveColumnToWorkspaceUp(true),
        Op::MoveColumnToWorkspace(1, true),
        Op::MoveColumnToWorkspace(2, true),
        Op::MoveWindowDown,
        Op::MoveWindowDownOrToWorkspaceDown,
        Op::MoveWindowUp,
        Op::MoveWindowUpOrToWorkspaceUp,
        Op::ConsumeOrExpelWindowLeft { id: None },
        Op::ConsumeOrExpelWindowRight { id: None },
        Op::MoveWorkspaceToOutput(1),
        Op::ToggleColumnTabbedDisplay,
    ];

    for third in &every_op {
        for second in &every_op {
            for first in &every_op {
                // eprintln!("{first:?}, {second:?}, {third:?}");

                let mut layout = Layout::default();
                first.clone().apply(&mut layout);
                layout.verify_invariants();
                second.clone().apply(&mut layout);
                layout.verify_invariants();
                third.clone().apply(&mut layout);
                layout.verify_invariants();
            }
        }
    }
}

#[test]
fn operations_from_starting_state_dont_panic() {
    if std::env::var_os("RUN_SLOW_TESTS").is_none() {
        eprintln!("ignoring slow test");
        return;
    }

    // Running every op from an empty state doesn't get us to all the interesting states. So,
    // also run it from a manually-created starting state with more things going on to exercise
    // more code paths.
    let setup_ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::MoveWindowToWorkspaceDown(true),
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::AddWindow {
            params: TestWindowParams::new(3),
        },
        Op::FocusColumnLeft,
        Op::ConsumeWindowIntoColumn,
        Op::AddWindow {
            params: TestWindowParams::new(4),
        },
        Op::AddOutput(2),
        Op::AddWindow {
            params: TestWindowParams::new(5),
        },
        Op::MoveWindowToOutput {
            window_id: None,
            output_id: 2,
            target_ws_idx: None,
        },
        Op::FocusOutput(1),
        Op::Communicate(1),
        Op::Communicate(2),
        Op::Communicate(3),
        Op::Communicate(4),
        Op::Communicate(5),
    ];

    let every_op = [
        Op::AddOutput(0),
        Op::AddOutput(1),
        Op::AddOutput(2),
        Op::RemoveOutput(0),
        Op::RemoveOutput(1),
        Op::RemoveOutput(2),
        Op::FocusOutput(0),
        Op::FocusOutput(1),
        Op::FocusOutput(2),
        Op::AddNamedWorkspace {
            ws_name: 1,
            output_name: Some(1),
            layout_config: None,
        },
        Op::UnnameWorkspace { ws_name: 1 },
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::AddWindowNextTo {
            params: TestWindowParams::new(6),
            next_to_id: 0,
        },
        Op::AddWindowNextTo {
            params: TestWindowParams::new(7),
            next_to_id: 1,
        },
        Op::AddWindowToNamedWorkspace {
            params: TestWindowParams::new(5),
            ws_name: 1,
        },
        Op::CloseWindow(0),
        Op::CloseWindow(1),
        Op::CloseWindow(2),
        Op::FullscreenWindow(1),
        Op::FullscreenWindow(2),
        Op::FullscreenWindow(3),
        Op::MaximizeWindowToEdges { id: Some(1) },
        Op::MaximizeWindowToEdges { id: Some(2) },
        Op::MaximizeWindowToEdges { id: Some(3) },
        Op::SetFullscreenWindow {
            window: 1,
            is_fullscreen: false,
        },
        Op::SetFullscreenWindow {
            window: 1,
            is_fullscreen: true,
        },
        Op::SetFullscreenWindow {
            window: 2,
            is_fullscreen: false,
        },
        Op::SetFullscreenWindow {
            window: 2,
            is_fullscreen: true,
        },
        Op::FocusColumnLeft,
        Op::FocusColumnRight,
        Op::FocusColumnRightOrFirst,
        Op::FocusColumnLeftOrLast,
        Op::FocusWindowOrMonitorUp(0),
        Op::FocusWindowOrMonitorDown(1),
        Op::FocusColumnOrMonitorLeft(0),
        Op::FocusColumnOrMonitorRight(1),
        Op::FocusWindowUp,
        Op::FocusWindowUpOrColumnLeft,
        Op::FocusWindowUpOrColumnRight,
        Op::FocusWindowOrWorkspaceUp,
        Op::FocusWindowDown,
        Op::FocusWindowDownOrColumnLeft,
        Op::FocusWindowDownOrColumnRight,
        Op::FocusWindowOrWorkspaceDown,
        Op::MoveColumnLeft,
        Op::MoveColumnRight,
        Op::MoveColumnLeftOrToMonitorLeft(0),
        Op::MoveColumnRightOrToMonitorRight(1),
        Op::ConsumeWindowIntoColumn,
        Op::ExpelWindowFromColumn,
        Op::CenterColumn,
        Op::FocusWorkspaceDown,
        Op::FocusWorkspaceUp,
        Op::FocusWorkspace(1),
        Op::FocusWorkspace(2),
        Op::FocusWorkspace(3),
        Op::MoveWindowToWorkspaceDown(true),
        Op::MoveWindowToWorkspaceUp(true),
        Op::MoveWindowToWorkspace {
            window_id: None,
            workspace_idx: 1,
        },
        Op::MoveWindowToWorkspace {
            window_id: None,
            workspace_idx: 2,
        },
        Op::MoveWindowToWorkspace {
            window_id: None,
            workspace_idx: 3,
        },
        Op::MoveColumnToWorkspaceDown(true),
        Op::MoveColumnToWorkspaceUp(true),
        Op::MoveColumnToWorkspace(1, true),
        Op::MoveColumnToWorkspace(2, true),
        Op::MoveColumnToWorkspace(3, true),
        Op::MoveWindowDown,
        Op::MoveWindowDownOrToWorkspaceDown,
        Op::MoveWindowUp,
        Op::MoveWindowUpOrToWorkspaceUp,
        Op::ConsumeOrExpelWindowLeft { id: None },
        Op::ConsumeOrExpelWindowRight { id: None },
        Op::ToggleColumnTabbedDisplay,
    ];

    for third in &every_op {
        for second in &every_op {
            for first in &every_op {
                // eprintln!("{first:?}, {second:?}, {third:?}");

                let mut layout = Layout::default();
                for op in &setup_ops {
                    op.clone().apply(&mut layout);
                }

                let mut layout = Layout::default();
                first.clone().apply(&mut layout);
                layout.verify_invariants();
                second.clone().apply(&mut layout);
                layout.verify_invariants();
                third.clone().apply(&mut layout);
                layout.verify_invariants();
            }
        }
    }
}

#[test]
fn primary_active_workspace_idx_not_updated_on_output_add() {
    let ops = [
        Op::AddOutput(1),
        Op::AddOutput(2),
        Op::FocusOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::FocusOutput(2),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::RemoveOutput(2),
        Op::FocusWorkspace(3),
        Op::AddOutput(2),
    ];

    check_ops(ops);
}

#[test]
fn window_closed_on_previous_workspace() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::FocusWorkspaceDown,
        Op::CloseWindow(0),
    ];

    check_ops(ops);
}

#[test]
fn removing_output_must_keep_empty_focus_on_primary() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::AddOutput(2),
        Op::RemoveOutput(1),
    ];

    let layout = check_ops(ops);

    let MonitorSet::Normal { monitors, .. } = layout.monitor_set else {
        unreachable!()
    };

    // The workspace from the removed output was inserted at position 0, so the active workspace
    // must change to 1 to keep the focus on the empty workspace.
    assert_eq!(monitors[0].active_workspace_idx, 1);
}

#[test]
fn move_to_workspace_by_idx_does_not_leave_empty_workspaces() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::AddOutput(2),
        Op::FocusOutput(2),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::RemoveOutput(1),
        Op::MoveWindowToWorkspace {
            window_id: Some(0),
            workspace_idx: 2,
        },
    ];

    let layout = check_ops(ops);

    let MonitorSet::Normal { monitors, .. } = layout.monitor_set else {
        unreachable!()
    };

    assert!(monitors[0].workspaces[1].has_windows());
}

#[test]
fn empty_workspaces_dont_move_back_to_original_output() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::FocusWorkspaceDown,
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::AddOutput(2),
        Op::RemoveOutput(1),
        Op::FocusWorkspace(1),
        Op::CloseWindow(1),
        Op::AddOutput(1),
    ];

    check_ops(ops);
}

#[test]
fn named_workspaces_dont_update_original_output_on_adding_window() {
    let ops = [
        Op::AddOutput(1),
        Op::SetWorkspaceName {
            new_ws_name: 1,
            ws_name: None,
        },
        Op::AddOutput(2),
        Op::RemoveOutput(1),
        Op::FocusWorkspaceUp,
        // Adding a window updates the original output for unnamed workspaces.
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        // Connecting the previous output should move the named workspace back since its
        // original output wasn't updated.
        Op::AddOutput(1),
    ];

    let layout = check_ops(ops);
    let (mon, _, ws) = layout
        .workspaces()
        .find(|(_, _, ws)| ws.name().is_some())
        .unwrap();
    assert!(ws.name().is_some()); // Sanity check.
    let mon = mon.unwrap();
    assert_eq!(mon.output_name(), "output1");
}

#[test]
fn workspaces_update_original_output_on_moving_to_same_output() {
    let ops = [
        Op::AddOutput(1),
        Op::SetWorkspaceName {
            new_ws_name: 1,
            ws_name: None,
        },
        Op::AddOutput(2),
        Op::RemoveOutput(1),
        Op::FocusWorkspaceUp,
        Op::MoveWorkspaceToOutput(2),
        Op::AddOutput(1),
    ];

    let layout = check_ops(ops);
    let (mon, _, ws) = layout
        .workspaces()
        .find(|(_, _, ws)| ws.name().is_some())
        .unwrap();
    assert!(ws.name().is_some()); // Sanity check.
    let mon = mon.unwrap();
    assert_eq!(mon.output_name(), "output2");
}

#[test]
fn workspaces_update_original_output_on_moving_to_same_monitor() {
    let ops = [
        Op::AddOutput(1),
        Op::SetWorkspaceName {
            new_ws_name: 1,
            ws_name: None,
        },
        Op::AddOutput(2),
        Op::RemoveOutput(1),
        Op::FocusWorkspaceUp,
        Op::MoveWorkspaceToMonitor {
            ws_name: Some(1),
            output_id: 2,
        },
        Op::AddOutput(1),
    ];

    let layout = check_ops(ops);
    let (mon, _, ws) = layout
        .workspaces()
        .find(|(_, _, ws)| ws.name().is_some())
        .unwrap();
    assert!(ws.name().is_some()); // Sanity check.
    let mon = mon.unwrap();
    assert_eq!(mon.output_name(), "output2");
}

#[test]
fn large_negative_height_change() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::AdjustProportion(-1e129),
        },
    ];

    let mut options = Options::default();
    options.layout.border.off = false;
    options.layout.border.width = 1.;

    check_ops_with_options(options, ops);
}

#[test]
fn large_max_size() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams {
                min_max_size: (Size::from((0, 0)), Size::from((i32::MAX, i32::MAX))),
                ..TestWindowParams::new(1)
            },
        },
    ];

    let mut options = Options::default();
    options.layout.border.off = false;
    options.layout.border.width = 1.;

    check_ops_with_options(options, ops);
}

#[test]
fn workspace_cleanup_during_switch() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::FocusWorkspaceDown,
        Op::CloseWindow(1),
    ];

    check_ops(ops);
}

#[test]
fn workspace_transfer_during_switch() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddOutput(2),
        Op::FocusOutput(2),
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::RemoveOutput(1),
        Op::FocusWorkspaceDown,
        Op::FocusWorkspaceDown,
        Op::AddOutput(1),
    ];

    check_ops(ops);
}

#[test]
fn workspace_transfer_during_switch_from_last() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddOutput(2),
        Op::RemoveOutput(1),
        Op::FocusWorkspaceUp,
        Op::AddOutput(1),
    ];

    check_ops(ops);
}

#[test]
fn workspace_transfer_during_switch_gets_cleaned_up() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::RemoveOutput(1),
        Op::AddOutput(2),
        Op::MoveColumnToWorkspaceDown(true),
        Op::MoveColumnToWorkspaceDown(true),
        Op::AddOutput(1),
    ];

    check_ops(ops);
}

#[test]
fn move_workspace_to_output() {
    let ops = [
        Op::AddOutput(1),
        Op::AddOutput(2),
        Op::FocusOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::MoveWorkspaceToOutput(2),
    ];

    let layout = check_ops(ops);

    let MonitorSet::Normal {
        monitors,
        active_monitor_idx,
        ..
    } = layout.monitor_set
    else {
        unreachable!()
    };

    assert_eq!(active_monitor_idx, 1);
    assert_eq!(monitors[0].workspaces.len(), 1);
    assert!(!monitors[0].workspaces[0].has_windows());
    assert_eq!(monitors[1].active_workspace_idx, 0);
    assert_eq!(monitors[1].workspaces.len(), 2);
    assert!(monitors[1].workspaces[0].has_windows());
}

#[test]
fn open_right_of_on_different_workspace() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::FocusWorkspaceDown,
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::AddWindowNextTo {
            params: TestWindowParams::new(3),
            next_to_id: 1,
        },
    ];

    let layout = check_ops(ops);

    let MonitorSet::Normal { monitors, .. } = layout.monitor_set else {
        unreachable!()
    };

    let mon = monitors.into_iter().next().unwrap();
    assert_eq!(
        mon.active_workspace_idx, 1,
        "the second workspace must remain active"
    );
    assert_eq!(
        mon.workspaces[0].scrolling().active_column_idx(),
        1,
        "the new window must become active"
    );
}

#[test]
// empty_workspace_above_first = true
fn open_right_of_on_different_workspace_ewaf() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::FocusWorkspaceDown,
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::AddWindowNextTo {
            params: TestWindowParams::new(3),
            next_to_id: 1,
        },
    ];

    let options = Options {
        layout: niri_config::Layout {
            empty_workspace_above_first: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let layout = check_ops_with_options(options, ops);

    let MonitorSet::Normal { monitors, .. } = layout.monitor_set else {
        unreachable!()
    };

    let mon = monitors.into_iter().next().unwrap();
    assert_eq!(
        mon.active_workspace_idx, 2,
        "the second workspace must remain active"
    );
    assert_eq!(
        mon.workspaces[1].scrolling().active_column_idx(),
        1,
        "the new window must become active"
    );
}

#[test]
fn removing_all_outputs_preserves_empty_named_workspaces() {
    let ops = [
        Op::AddOutput(1),
        Op::AddNamedWorkspace {
            ws_name: 1,
            output_name: None,
            layout_config: None,
        },
        Op::AddNamedWorkspace {
            ws_name: 2,
            output_name: None,
            layout_config: None,
        },
        Op::RemoveOutput(1),
    ];

    let layout = check_ops(ops);

    let MonitorSet::NoOutputs { workspaces } = layout.monitor_set else {
        unreachable!()
    };

    assert_eq!(workspaces.len(), 2);
}

#[test]
fn config_change_updates_cached_sizes() {
    let mut config = Config::default();
    let border = &mut config.layout.border;
    border.off = false;
    border.width = 2.;

    let mut layout = Layout::new(Clock::default(), &config);

    Op::AddWindow {
        params: TestWindowParams {
            bbox: Rectangle::from_size(Size::from((1280, 200))),
            ..TestWindowParams::new(1)
        },
    }
    .apply(&mut layout);

    config.layout.border.width = 4.;
    layout.update_config(&config);

    layout.verify_invariants();
}

#[test]
fn preset_height_change_removes_preset() {
    let mut config = Config::default();
    config.layout.preset_window_heights = vec![PresetSize::Fixed(1), PresetSize::Fixed(2)];

    let mut layout = Layout::new(Clock::default(), &config);

    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::ConsumeOrExpelWindowLeft { id: None },
        Op::SwitchPresetWindowHeight { id: None },
        Op::SwitchPresetWindowHeight { id: None },
    ];
    for op in ops {
        op.apply(&mut layout);
    }

    // Leave only one.
    config.layout.preset_window_heights = vec![PresetSize::Fixed(1)];

    layout.update_config(&config);

    layout.verify_invariants();
}

#[test]
fn set_window_height_recomputes_to_auto() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::ConsumeOrExpelWindowLeft { id: None },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::ConsumeOrExpelWindowLeft { id: None },
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(100),
        },
        Op::FocusWindowUp,
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(200),
        },
    ];

    check_ops(ops);
}

#[test]
fn one_window_in_column_becomes_weight_1() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::ConsumeOrExpelWindowLeft { id: None },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::ConsumeOrExpelWindowLeft { id: None },
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(100),
        },
        Op::Communicate(2),
        Op::FocusWindowUp,
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(200),
        },
        Op::Communicate(1),
        Op::CloseWindow(0),
        Op::CloseWindow(1),
    ];

    check_ops(ops);
}

#[test]
fn fixed_height_takes_max_non_auto_into_account() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::SetWindowHeight {
            id: Some(0),
            change: SizeChange::SetFixed(704),
        },
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::ConsumeOrExpelWindowLeft { id: None },
    ];

    let options = Options {
        layout: niri_config::Layout {
            border: niri_config::Border {
                off: false,
                width: 4.,
                ..Default::default()
            },
            gaps: 0.,
            ..Default::default()
        },
        ..Default::default()
    };
    check_ops_with_options(options, ops);
}

#[test]
fn start_interactive_move_then_remove_window() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::InteractiveMoveBegin {
            window: 0,
            output_idx: 1,
            px: 0.,
            py: 0.,
        },
        Op::CloseWindow(0),
    ];

    check_ops(ops);
}

#[test]
fn interactive_move_onto_empty_output() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::InteractiveMoveBegin {
            window: 0,
            output_idx: 1,
            px: 0.,
            py: 0.,
        },
        Op::AddOutput(2),
        Op::InteractiveMoveUpdate {
            window: 0,
            dx: 1000.,
            dy: 0.,
            output_idx: 2,
            px: 0.,
            py: 0.,
        },
        Op::InteractiveMoveEnd { window: 0 },
    ];

    check_ops(ops);
}

#[test]
fn interactive_move_onto_empty_output_ewaf() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::InteractiveMoveBegin {
            window: 0,
            output_idx: 1,
            px: 0.,
            py: 0.,
        },
        Op::AddOutput(2),
        Op::InteractiveMoveUpdate {
            window: 0,
            dx: 1000.,
            dy: 0.,
            output_idx: 2,
            px: 0.,
            py: 0.,
        },
        Op::InteractiveMoveEnd { window: 0 },
    ];

    let options = Options {
        layout: niri_config::Layout {
            empty_workspace_above_first: true,
            ..Default::default()
        },
        ..Default::default()
    };
    check_ops_with_options(options, ops);
}

#[test]
fn interactive_move_onto_last_workspace() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::InteractiveMoveBegin {
            window: 0,
            output_idx: 1,
            px: 0.,
            py: 0.,
        },
        Op::InteractiveMoveUpdate {
            window: 0,
            dx: 1000.,
            dy: 0.,
            output_idx: 1,
            px: 0.,
            py: 0.,
        },
        Op::FocusWorkspaceDown,
        Op::AdvanceAnimations { msec_delta: 1000 },
        Op::InteractiveMoveEnd { window: 0 },
    ];

    check_ops(ops);
}

#[test]
fn interactive_move_onto_first_empty_workspace() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::InteractiveMoveBegin {
            window: 1,
            output_idx: 1,
            px: 0.,
            py: 0.,
        },
        Op::InteractiveMoveUpdate {
            window: 1,
            dx: 1000.,
            dy: 0.,
            output_idx: 1,
            px: 0.,
            py: 0.,
        },
        Op::FocusWorkspaceUp,
        Op::AdvanceAnimations { msec_delta: 1000 },
        Op::InteractiveMoveEnd { window: 1 },
    ];
    let options = Options {
        layout: niri_config::Layout {
            empty_workspace_above_first: true,
            ..Default::default()
        },
        ..Default::default()
    };
    check_ops_with_options(options, ops);
}

#[test]
fn output_active_workspace_is_preserved() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::FocusWorkspaceDown,
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::RemoveOutput(1),
        Op::AddOutput(1),
    ];

    let layout = check_ops(ops);

    let MonitorSet::Normal { monitors, .. } = layout.monitor_set else {
        unreachable!()
    };

    assert_eq!(monitors[0].active_workspace_idx, 1);
}

#[test]
fn output_active_workspace_is_preserved_with_other_outputs() {
    let ops = [
        Op::AddOutput(1),
        Op::AddOutput(2),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::FocusWorkspaceDown,
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::RemoveOutput(1),
        Op::AddOutput(1),
    ];

    let layout = check_ops(ops);

    let MonitorSet::Normal { monitors, .. } = layout.monitor_set else {
        unreachable!()
    };

    assert_eq!(monitors[1].active_workspace_idx, 1);
}

#[test]
fn named_workspace_to_output() {
    let ops = [
        Op::AddNamedWorkspace {
            ws_name: 1,
            output_name: None,
            layout_config: None,
        },
        Op::AddOutput(1),
        Op::MoveWorkspaceToOutput(1),
        Op::FocusWorkspaceUp,
    ];
    check_ops(ops);
}

#[test]
// empty_workspace_above_first = true
fn named_workspace_to_output_ewaf() {
    let ops = [
        Op::AddNamedWorkspace {
            ws_name: 1,
            output_name: Some(2),
            layout_config: None,
        },
        Op::AddOutput(1),
        Op::AddOutput(2),
    ];
    let options = Options {
        layout: niri_config::Layout {
            empty_workspace_above_first: true,
            ..Default::default()
        },
        ..Default::default()
    };
    check_ops_with_options(options, ops);
}

#[test]
fn move_window_to_empty_workspace_above_first() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::MoveWorkspaceUp,
        Op::MoveWorkspaceDown,
        Op::FocusWorkspaceUp,
        Op::MoveWorkspaceDown,
    ];
    let options = Options {
        layout: niri_config::Layout {
            empty_workspace_above_first: true,
            ..Default::default()
        },
        ..Default::default()
    };
    check_ops_with_options(options, ops);
}

#[test]
fn move_window_to_different_output() {
    let ops = [
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddOutput(1),
        Op::AddOutput(2),
        Op::MoveWorkspaceToOutput(2),
    ];
    let options = Options {
        layout: niri_config::Layout {
            empty_workspace_above_first: true,
            ..Default::default()
        },
        ..Default::default()
    };
    check_ops_with_options(options, ops);
}

#[test]
fn close_window_empty_ws_above_first() {
    let ops = [
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddOutput(1),
        Op::CloseWindow(1),
    ];
    let options = Options {
        layout: niri_config::Layout {
            empty_workspace_above_first: true,
            ..Default::default()
        },
        ..Default::default()
    };
    check_ops_with_options(options, ops);
}

#[test]
fn add_and_remove_output() {
    let ops = [
        Op::AddOutput(2),
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::RemoveOutput(2),
    ];
    let options = Options {
        layout: niri_config::Layout {
            empty_workspace_above_first: true,
            ..Default::default()
        },
        ..Default::default()
    };
    check_ops_with_options(options, ops);
}

#[test]
fn switch_ewaf_on() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
    ];

    let mut layout = check_ops(ops);
    layout.update_options(Options {
        layout: niri_config::Layout {
            empty_workspace_above_first: true,
            ..Default::default()
        },
        ..Default::default()
    });
    layout.verify_invariants();
}

#[test]
fn switch_ewaf_off() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
    ];

    let options = Options {
        layout: niri_config::Layout {
            empty_workspace_above_first: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut layout = check_ops_with_options(options, ops);
    layout.update_options(Options::default());
    layout.verify_invariants();
}

#[test]
fn interactive_move_drop_on_other_output_during_animation() {
    let ops = [
        Op::AddOutput(3),
        Op::AddWindow {
            params: TestWindowParams::new(3),
        },
        Op::InteractiveMoveBegin {
            window: 3,
            output_idx: 3,
            px: 0.0,
            py: 0.0,
        },
        Op::FocusWorkspaceDown,
        Op::AddOutput(4),
        Op::InteractiveMoveUpdate {
            window: 3,
            dx: 0.0,
            dy: 8300.68619826683,
            output_idx: 4,
            px: 0.0,
            py: 0.0,
        },
        Op::RemoveOutput(4),
        Op::InteractiveMoveEnd { window: 3 },
    ];
    check_ops(ops);
}

#[test]
fn add_window_next_to_only_interactively_moved_without_outputs() {
    let ops = [
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::AddOutput(1),
        Op::InteractiveMoveBegin {
            window: 2,
            output_idx: 1,
            px: 0.0,
            py: 0.0,
        },
        Op::InteractiveMoveUpdate {
            window: 2,
            dx: 0.0,
            dy: 3586.692842955048,
            output_idx: 1,
            px: 0.0,
            py: 0.0,
        },
        Op::RemoveOutput(1),
        // We have no outputs, and the only existing window is interactively moved, meaning there
        // are no workspaces either.
        Op::AddWindowNextTo {
            params: TestWindowParams::new(3),
            next_to_id: 2,
        },
    ];

    check_ops(ops);
}

#[test]
fn interactive_move_toggle_floating_ends_dnd_gesture() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::InteractiveMoveBegin {
            window: 2,
            output_idx: 1,
            px: 0.0,
            py: 0.0,
        },
        Op::InteractiveMoveUpdate {
            window: 2,
            dx: 0.0,
            dy: 3586.692842955048,
            output_idx: 1,
            px: 0.0,
            py: 0.0,
        },
        Op::Refresh { is_active: false },
        Op::ToggleWindowFloating { id: None },
        Op::InteractiveMoveEnd { window: 2 },
    ];

    check_ops(ops);
}

#[test]
fn interactive_move_from_workspace_with_layout_config() {
    let ops = [
        Op::AddNamedWorkspace {
            ws_name: 1,
            output_name: Some(2),
            layout_config: Some(Box::new(niri_config::LayoutPart {
                border: Some(niri_config::BorderRule {
                    on: true,
                    ..Default::default()
                }),
                ..Default::default()
            })),
        },
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::InteractiveMoveBegin {
            window: 2,
            output_idx: 1,
            px: 0.0,
            py: 0.0,
        },
        Op::InteractiveMoveUpdate {
            window: 2,
            dx: 0.0,
            dy: 3586.692842955048,
            output_idx: 1,
            px: 0.0,
            py: 0.0,
        },
        // Now remove and add the output. It will have the same workspace.
        Op::RemoveOutput(1),
        Op::AddOutput(1),
        Op::InteractiveMoveUpdate {
            window: 2,
            dx: 0.0,
            dy: 0.0,
            output_idx: 1,
            px: 0.0,
            py: 0.0,
        },
        // Now move onto a different workspace.
        Op::FocusWorkspaceDown,
        Op::CompleteAnimations,
        Op::InteractiveMoveUpdate {
            window: 2,
            dx: 0.0,
            dy: 0.0,
            output_idx: 1,
            px: 0.0,
            py: 0.0,
        },
    ];

    check_ops(ops);
}

#[test]
fn set_width_fixed_negative() {
    let ops = [
        Op::AddOutput(3),
        Op::AddWindow {
            params: TestWindowParams::new(3),
        },
        Op::ToggleWindowFloating { id: Some(3) },
        Op::SetColumnWidth(SizeChange::SetFixed(-100)),
    ];
    check_ops(ops);
}

#[test]
fn set_height_fixed_negative() {
    let ops = [
        Op::AddOutput(3),
        Op::AddWindow {
            params: TestWindowParams::new(3),
        },
        Op::ToggleWindowFloating { id: Some(3) },
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(-100),
        },
    ];
    check_ops(ops);
}

#[test]
fn interactive_resize_to_negative() {
    let ops = [
        Op::AddOutput(3),
        Op::AddWindow {
            params: TestWindowParams::new(3),
        },
        Op::ToggleWindowFloating { id: Some(3) },
        Op::InteractiveResizeBegin {
            window: 3,
            edges: ResizeEdge::BOTTOM_RIGHT,
        },
        Op::InteractiveResizeUpdate {
            window: 3,
            dx: -10000.,
            dy: -10000.,
        },
    ];
    check_ops(ops);
}

#[test]
fn windows_on_other_workspaces_remain_activated() {
    let ops = [
        Op::AddOutput(3),
        Op::AddWindow {
            params: TestWindowParams::new(3),
        },
        Op::FocusWorkspaceDown,
        Op::Refresh { is_active: true },
    ];

    let layout = check_ops(ops);
    let (_, win) = layout.windows().next().unwrap();
    assert!(win.0.pending_activated.get());
}

#[test]
fn stacking_add_parent_brings_up_child() {
    let ops = [
        Op::AddOutput(0),
        Op::AddWindow {
            params: TestWindowParams {
                is_floating: true,
                parent_id: Some(1),
                ..TestWindowParams::new(0)
            },
        },
        Op::AddWindow {
            params: TestWindowParams {
                is_floating: true,
                ..TestWindowParams::new(1)
            },
        },
    ];

    check_ops(ops);
}

#[test]
fn stacking_add_parent_brings_up_descendants() {
    let ops = [
        Op::AddOutput(0),
        Op::AddWindow {
            params: TestWindowParams {
                is_floating: true,
                parent_id: Some(2),
                ..TestWindowParams::new(0)
            },
        },
        Op::AddWindow {
            params: TestWindowParams {
                is_floating: true,
                parent_id: Some(0),
                ..TestWindowParams::new(1)
            },
        },
        Op::AddWindow {
            params: TestWindowParams {
                is_floating: true,
                ..TestWindowParams::new(2)
            },
        },
    ];

    check_ops(ops);
}

#[test]
fn stacking_activate_brings_up_descendants() {
    let ops = [
        Op::AddOutput(0),
        Op::AddWindow {
            params: TestWindowParams {
                is_floating: true,
                ..TestWindowParams::new(0)
            },
        },
        Op::AddWindow {
            params: TestWindowParams {
                is_floating: true,
                parent_id: Some(0),
                ..TestWindowParams::new(1)
            },
        },
        Op::AddWindow {
            params: TestWindowParams {
                is_floating: true,
                parent_id: Some(1),
                ..TestWindowParams::new(2)
            },
        },
        Op::AddWindow {
            params: TestWindowParams {
                is_floating: true,
                ..TestWindowParams::new(3)
            },
        },
        Op::FocusWindow(0),
    ];

    check_ops(ops);
}

#[test]
fn stacking_set_parent_brings_up_child() {
    let ops = [
        Op::AddOutput(0),
        Op::AddWindow {
            params: TestWindowParams {
                is_floating: true,
                ..TestWindowParams::new(0)
            },
        },
        Op::AddWindow {
            params: TestWindowParams {
                is_floating: true,
                ..TestWindowParams::new(1)
            },
        },
        Op::SetParent {
            id: 0,
            new_parent_id: Some(1),
        },
    ];

    check_ops(ops);
}

#[test]
fn move_window_to_workspace_with_different_active_output() {
    let ops = [
        Op::AddOutput(0),
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::FocusOutput(1),
        Op::MoveWindowToWorkspace {
            window_id: Some(0),
            workspace_idx: 2,
        },
    ];

    check_ops(ops);
}

#[test]
fn set_first_workspace_name() {
    let ops = [
        Op::AddOutput(0),
        Op::SetWorkspaceName {
            new_ws_name: 0,
            ws_name: None,
        },
    ];

    check_ops(ops);
}

#[test]
fn set_first_workspace_name_ewaf() {
    let ops = [
        Op::AddOutput(0),
        Op::SetWorkspaceName {
            new_ws_name: 0,
            ws_name: None,
        },
    ];

    let options = Options {
        layout: niri_config::Layout {
            empty_workspace_above_first: true,
            ..Default::default()
        },
        ..Default::default()
    };
    check_ops_with_options(options, ops);
}

#[test]
fn set_last_workspace_name() {
    let ops = [
        Op::AddOutput(0),
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::FocusWorkspaceDown,
        Op::SetWorkspaceName {
            new_ws_name: 0,
            ws_name: None,
        },
    ];

    check_ops(ops);
}

#[test]
fn move_workspace_to_same_monitor_doesnt_reorder() {
    let ops = [
        Op::AddOutput(0),
        Op::SetWorkspaceName {
            new_ws_name: 0,
            ws_name: None,
        },
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::FocusWorkspaceDown,
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::MoveWorkspaceToMonitor {
            ws_name: Some(0),
            output_id: 0,
        },
    ];

    let layout = check_ops(ops);
    let counts: Vec<_> = layout
        .workspaces()
        .map(|(_, _, ws)| ws.windows().count())
        .collect();
    assert_eq!(counts, &[1, 2, 0]);
}

#[test]
fn removing_window_above_preserves_focused_window() {
    let ops = [
        Op::AddOutput(0),
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::FocusColumnFirst,
        Op::ConsumeWindowIntoColumn,
        Op::ConsumeWindowIntoColumn,
        Op::FocusWindowDown,
        Op::CloseWindow(0),
    ];

    let layout = check_ops(ops);
    let win = layout.focus().unwrap();
    assert_eq!(win.0.id, 1);
}

#[test]
fn preset_column_width_fixed_correct_with_border() {
    let ops = [
        Op::AddOutput(0),
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::SwitchPresetColumnWidth,
    ];

    let options = Options {
        layout: niri_config::Layout {
            preset_column_widths: vec![PresetSize::Fixed(500)],
            ..Default::default()
        },
        ..Default::default()
    };
    let mut layout = check_ops_with_options(options, ops);

    let win = layout.windows().next().unwrap().1;
    assert_eq!(win.requested_size().unwrap().w, 500);

    // Add border.
    let options = Options {
        layout: niri_config::Layout {
            preset_column_widths: vec![PresetSize::Fixed(500)],
            border: niri_config::Border {
                off: false,
                width: 5.,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    layout.update_options(options);

    // With border, the window gets less size.
    let win = layout.windows().next().unwrap().1;
    assert_eq!(win.requested_size().unwrap().w, 490);

    // However, preset fixed width will still work correctly.
    layout.toggle_width(true);
    let win = layout.windows().next().unwrap().1;
    assert_eq!(win.requested_size().unwrap().w, 500);
}

#[test]
fn preset_column_width_reset_after_set_width() {
    let ops = [
        Op::AddOutput(0),
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::SwitchPresetColumnWidth,
        Op::SetWindowWidth {
            id: None,
            change: SizeChange::AdjustFixed(-10),
        },
        Op::SwitchPresetColumnWidth,
    ];

    let options = Options {
        layout: niri_config::Layout {
            preset_column_widths: vec![PresetSize::Fixed(500), PresetSize::Fixed(1000)],
            ..Default::default()
        },
        ..Default::default()
    };
    let layout = check_ops_with_options(options, ops);
    let win = layout.windows().next().unwrap().1;
    assert_eq!(win.requested_size().unwrap().w, 500);
}

#[test]
fn move_column_to_workspace_unfocused_with_multiple_monitors() {
    let ops = [
        Op::AddOutput(1),
        Op::SetWorkspaceName {
            new_ws_name: 101,
            ws_name: None,
        },
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::FocusWorkspaceDown,
        Op::SetWorkspaceName {
            new_ws_name: 102,
            ws_name: None,
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::AddOutput(2),
        Op::FocusOutput(2),
        Op::SetWorkspaceName {
            new_ws_name: 201,
            ws_name: None,
        },
        Op::AddWindow {
            params: TestWindowParams::new(3),
        },
        Op::AddWindow {
            params: TestWindowParams::new(4),
        },
        Op::MoveColumnToOutput {
            output_id: 1,
            target_ws_idx: Some(0),
            activate: false,
        },
        Op::FocusOutput(1),
    ];

    let layout = check_ops(ops);

    assert_eq!(layout.active_workspace().unwrap().name().unwrap(), "ws102");

    for (mon, win) in layout.windows() {
        let mon = mon.unwrap();
        let ws = mon
            .workspaces
            .iter()
            .find(|w| w.has_window(win.id()))
            .unwrap();

        assert_eq!(
            ws.name().unwrap(),
            match win.id() {
                1 | 4 => "ws101",
                2 => "ws102",
                3 => "ws201",
                _ => unreachable!(),
            }
        );
    }
}

#[test]
fn scratchpad_toggle_and_cycle() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
    ]);

    layout.move_window_to_scratchpad(Some(&1));
    layout.verify_invariants();
    assert!(layout.active_monitor_ref().unwrap().scratchpad.contains(&1));
    assert!(!layout.active_monitor_ref().unwrap().scratchpad.contains(&2));

    layout.toggle_scratchpad();
    layout.verify_invariants();
    assert_eq!(
        layout.active_monitor_ref().unwrap().scratchpad.shown_id(),
        Some(&1)
    );

    layout.move_window_to_scratchpad(Some(&2));
    layout.verify_invariants();
    layout.cycle_scratchpad(true);
    layout.verify_invariants();
    assert_eq!(
        layout.active_monitor_ref().unwrap().scratchpad.shown_id(),
        Some(&1)
    );

    layout.toggle_scratchpad();
    layout.verify_invariants();
    assert!(layout
        .active_monitor_ref()
        .unwrap()
        .scratchpad
        .shown_id()
        .is_none());
}

#[test]
fn sidebar_move_and_reserve_insets() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
    ]);

    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&1));
    layout.verify_invariants();
    {
        let mon = layout.active_monitor_ref().unwrap();
        let sb = mon.left_sidebar.as_ref().unwrap();
        assert!(sb.contains(&1));
        assert!(sb.reserved_inset() > 0.);
        let ws = mon.active_workspace_ref();
        assert!(ws.area_with_sidebar_insets().size.w < ws.view_size().w);
    }

    // Regression: output-scoped sidebar windows must not panic FFM / popup lookup.
    assert!(layout.should_trigger_focus_follows_mouse_on(&1));
    let popup = layout.popup_target_rect(&1);
    assert!(popup.size.w > 0. && popup.size.h > 0.);

    layout.toggle_sidebar(niri_config::SidebarSide::Left);
    layout.verify_invariants();
    assert!(!layout
        .active_monitor_ref()
        .unwrap()
        .left_sidebar
        .as_ref()
        .unwrap()
        .is_visible());

    layout.toggle_sidebar(niri_config::SidebarSide::Left);
    layout.focus_sidebar(niri_config::SidebarSide::Left);
    layout.toggle_width(true);
    layout.verify_invariants();

    layout.move_window_out_of_sidebar(Some(&1));
    layout.verify_invariants();
    assert!(layout
        .active_monitor_ref()
        .unwrap()
        .left_sidebar
        .is_none());
}

#[test]
fn sidebar_hides_when_maximized_or_fullscreen() {
    let mut options = Options::default();
    options.layout.sidebar.hide_when_maximized = true;
    options.layout.sidebar.hide_when_fullscreen = true;

    let mut layout = check_ops_with_options(
        options,
        [
            Op::AddOutput(1),
            Op::AddWindow {
                params: TestWindowParams::new(1),
            },
            Op::AddWindow {
                params: TestWindowParams::new(2),
            },
        ],
    );

    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&1));
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    let full_inset = layout
        .active_monitor_ref()
        .unwrap()
        .left_sidebar
        .as_ref()
        .unwrap()
        .reserved_inset();
    assert!(full_inset > 0.);

    layout.toggle_maximized(&2);
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    {
        let sb = layout
            .active_monitor_ref()
            .unwrap()
            .left_sidebar
            .as_ref()
            .unwrap();
        assert!(!sb.is_visible());
        assert_eq!(sb.reserved_inset(), 0.);
    }

    layout.toggle_maximized(&2);
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    {
        let sb = layout
            .active_monitor_ref()
            .unwrap()
            .left_sidebar
            .as_ref()
            .unwrap();
        assert!(sb.is_visible());
        assert!((sb.reserved_inset() - full_inset).abs() < 1e-6);
    }

    layout.toggle_fullscreen(&2);
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    {
        let sb = layout
            .active_monitor_ref()
            .unwrap()
            .left_sidebar
            .as_ref()
            .unwrap();
        assert!(!sb.is_visible());
        assert_eq!(sb.reserved_inset(), 0.);
    }

    layout.toggle_fullscreen(&2);
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    {
        let sb = layout
            .active_monitor_ref()
            .unwrap()
            .left_sidebar
            .as_ref()
            .unwrap();
        assert!(sb.is_visible());
        assert!((sb.reserved_inset() - full_inset).abs() < 1e-6);
    }
}

#[test]
fn sidebar_does_not_auto_hide_without_flags() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
    ]);

    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&1));
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    let full_inset = layout
        .active_monitor_ref()
        .unwrap()
        .left_sidebar
        .as_ref()
        .unwrap()
        .reserved_inset();
    assert!(full_inset > 0.);

    layout.toggle_maximized(&2);
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    let sb = layout
        .active_monitor_ref()
        .unwrap()
        .left_sidebar
        .as_ref()
        .unwrap();
    assert!(sb.is_visible());
    assert!((sb.reserved_inset() - full_inset).abs() < 1e-6);
}

#[test]
fn sidebar_open_close_animates_reserved_inset() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
    ]);

    // Window 1 stays tiled; window 2 goes into the sidebar.
    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&2));
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    let full_inset = layout
        .active_monitor_ref()
        .unwrap()
        .left_sidebar
        .as_ref()
        .unwrap()
        .reserved_inset();
    assert!(full_inset > 0.);
    let tiling_size_before = layout
        .windows()
        .find(|(_, w)| *w.id() == 1)
        .unwrap()
        .1
        .requested_size();

    layout.toggle_sidebar(niri_config::SidebarSide::Left);
    // Mid-close: still rendering and inset between 0 and full.
    {
        let sb = layout
            .active_monitor_ref()
            .unwrap()
            .left_sidebar
            .as_ref()
            .unwrap();
        assert!(!sb.is_visible());
        assert!(sb.is_rendered());
        assert!(sb.are_animations_ongoing());
        let inset = sb.reserved_inset();
        assert!(inset > 0. && inset <= full_inset);
    }

    // Clear leftover animate flags from earlier ops, then sync mid-close.
    layout
        .windows()
        .find(|(_, w)| *w.id() == 1)
        .unwrap()
        .1
        .0
        .animate_next_configure
        .set(false);
    let now = layout.clock.now_unadjusted() + Duration::from_millis(50);
    layout.clock.set_unadjusted(now);
    layout.advance_animations();
    {
        let tiling_win = layout
            .windows()
            .find(|(_, w)| *w.id() == 1)
            .unwrap()
            .1;
        assert_ne!(tiling_size_before, tiling_win.requested_size());
        assert!(
            !tiling_win.0.animate_next_configure.get(),
            "tiling window must not arm animate_next_configure during sidebar open/close inset sync"
        );
    }

    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    {
        let sb = layout
            .active_monitor_ref()
            .unwrap()
            .left_sidebar
            .as_ref()
            .unwrap();
        assert!(!sb.is_visible());
        assert!(!sb.is_rendered());
        assert_eq!(sb.reserved_inset(), 0.);
        assert!(!sb.are_animations_ongoing());
    }
    let ws = layout.active_monitor_ref().unwrap().active_workspace_ref();
    assert_eq!(ws.area_with_sidebar_insets().size.w, ws.view_size().w);

    layout.toggle_sidebar(niri_config::SidebarSide::Left);
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    let sb = layout
        .active_monitor_ref()
        .unwrap()
        .left_sidebar
        .as_ref()
        .unwrap();
    assert!(sb.is_visible());
    assert!((sb.reserved_inset() - full_inset).abs() < 1e-6);
}

fn test_tiling_window_width(layout: &Layout<TestWindow>, id: usize) -> i32 {
    layout
        .windows()
        .find(|(_, w)| *w.id() == id)
        .unwrap()
        .1
        .requested_size()
        .unwrap()
        .w
}

#[test]
fn sidebar_swaps_default_matching_column_on_open() {
    let mut options = Options::default();
    options.layout.default_column_width = Some(PresetSize::Proportion(0.5));
    options.layout.sidebar.default_column_width_when_open =
        Some(PresetSize::Proportion(0.35));

    let mut layout = check_ops_with_options(
        options.clone(),
        [
            Op::AddOutput(1),
            Op::AddWindow {
                params: TestWindowParams::new(1),
            },
            Op::AddWindow {
                params: TestWindowParams::new(2),
            },
        ],
    );

    let width_at_default_closed = test_tiling_window_width(&layout, 2);
    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&1));
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);

    let ws = layout.active_workspace().unwrap();
    let working_w = ws.scrolling().working_area().size.w;
    let gaps = options.layout.gaps;
    let expected_open_default =
        ((working_w - gaps) * 0.35 - gaps).round() as i32;
    let width_after_open = test_tiling_window_width(&layout, 2);
    assert!(
        (width_after_open - expected_open_default).abs() <= 2,
        "expected default-matching column to swap to open default (got {width_after_open}, expected ~{expected_open_default})"
    );
    assert!(
        width_after_open < width_at_default_closed,
        "open default should be narrower than closed default on a smaller strip"
    );
}

#[test]
fn sidebar_column_width_pairs_swap_on_open_and_close() {
    let mut options = Options::default();
    options.layout.default_column_width = Some(PresetSize::Proportion(0.5));
    options.layout.sidebar.column_width_pairs = vec![
        niri_config::ColumnWidthPair {
            closed: Some(PresetSize::Proportion(0.5)),
            open: Some(PresetSize::Proportion(1.0)),
        },
        niri_config::ColumnWidthPair {
            closed: Some(PresetSize::Proportion(1. / 3.)),
            open: Some(PresetSize::Proportion(0.5)),
        },
    ];
    // Should be ignored while pairs are set.
    options.layout.sidebar.default_column_width_when_open =
        Some(PresetSize::Proportion(0.35));

    let mut layout = check_ops_with_options(
        options.clone(),
        [
            Op::AddOutput(1),
            Op::AddWindow {
                params: TestWindowParams::new(1),
            },
            Op::AddWindow {
                params: TestWindowParams::new(2),
            },
            Op::FocusColumn(2),
            Op::SetColumnWidth(SizeChange::SetProportion(100. / 3.)),
            Op::AddWindow {
                params: TestWindowParams::new(3),
            },
        ],
    );

    // Window 2 is pair[1] closed (1/3); window 3 is pair[0] closed (0.5).
    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&1));
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);

    let ws = layout.active_workspace().unwrap();
    let working_w = ws.scrolling().working_area().size.w;
    let gaps = options.layout.gaps;
    let expected_pair0_open = ((working_w - gaps) * 1.0 - gaps).round() as i32;
    let expected_pair1_open = ((working_w - gaps) * 0.5 - gaps).round() as i32;

    let width3 = test_tiling_window_width(&layout, 3);
    let width2 = test_tiling_window_width(&layout, 2);
    assert!(
        (width3 - expected_pair0_open).abs() <= 2,
        "pair[0] column should swap to open side (got {width3}, expected ~{expected_pair0_open})"
    );
    assert!(
        (width2 - expected_pair1_open).abs() <= 2,
        "pair[1] column should swap to its own open side (got {width2}, expected ~{expected_pair1_open})"
    );

    layout.toggle_sidebar(niri_config::SidebarSide::Left);
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);

    let ws = layout.active_workspace().unwrap();
    let working_w = ws.scrolling().working_area().size.w;
    let expected_pair0_closed = ((working_w - gaps) * 0.5 - gaps).round() as i32;
    let expected_pair1_closed = ((working_w - gaps) * (1. / 3.) - gaps).round() as i32;

    let width3 = test_tiling_window_width(&layout, 3);
    let width2 = test_tiling_window_width(&layout, 2);
    assert!(
        (width3 - expected_pair0_closed).abs() <= 2,
        "pair[0] column should swap back to closed side (got {width3}, expected ~{expected_pair0_closed})"
    );
    assert!(
        (width2 - expected_pair1_closed).abs() <= 2,
        "pair[1] column should swap back to closed side (got {width2}, expected ~{expected_pair1_closed})"
    );
}

#[test]
fn sidebar_column_width_pairs_mod_r_uses_open_side() {
    let mut options = Options::default();
    options.layout.default_column_width = Some(PresetSize::Proportion(0.5));
    options.layout.preset_column_widths = vec![
        PresetSize::Proportion(0.25),
        PresetSize::Proportion(0.75),
    ];
    options.layout.sidebar.column_width_pairs = vec![
        niri_config::ColumnWidthPair {
            closed: Some(PresetSize::Proportion(0.5)),
            open: Some(PresetSize::Proportion(1.0)),
        },
        niri_config::ColumnWidthPair {
            closed: Some(PresetSize::Proportion(1. / 3.)),
            open: Some(PresetSize::Proportion(0.5)),
        },
    ];

    let mut layout = check_ops_with_options(
        options.clone(),
        [
            Op::AddOutput(1),
            Op::AddWindow {
                params: TestWindowParams::new(1),
            },
            Op::AddWindow {
                params: TestWindowParams::new(2),
            },
        ],
    );

    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&1));
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);

    let ws = layout.active_workspace().unwrap();
    let working_w = ws.scrolling().working_area().size.w;
    let gaps = options.layout.gaps;
    let open_half = ((working_w - gaps) * 0.5 - gaps).round() as i32;
    let open_full = ((working_w - gaps) * 1.0 - gaps).round() as i32;

    // After open, window 2 is on pair[0] open (1.0, largest). Forwards wraps to the
    // smallest open side (pair[1] = 0.5), not the next list index.
    layout.toggle_width(true);
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    let width = test_tiling_window_width(&layout, 2);
    assert!(
        (width - open_half).abs() <= 2,
        "Mod+R from largest open side should wrap to smallest (got {width}, expected ~{open_half})"
    );

    // Next forwards climbs ascending: 0.5 → 1.0.
    layout.toggle_width(true);
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    let width = test_tiling_window_width(&layout, 2);
    assert!(
        (width - open_full).abs() <= 2,
        "Mod+R should climb ascending open sizes (got {width}, expected ~{open_full})"
    );

    // One more wrap to smallest again, then close to verify pair linkage.
    layout.toggle_width(true);
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    layout.toggle_sidebar(niri_config::SidebarSide::Left);
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);

    let ws = layout.active_workspace().unwrap();
    let working_w = ws.scrolling().working_area().size.w;
    let expected_closed = ((working_w - gaps) * (1. / 3.) - gaps).round() as i32;
    let width = test_tiling_window_width(&layout, 2);
    assert!(
        (width - expected_closed).abs() <= 2,
        "after Mod+R then close, column should use pair closed side (got {width}, expected ~{expected_closed})"
    );
}

#[test]
fn sidebar_column_width_pairs_skip_full_width() {
    let mut options = Options::default();
    options.layout.default_column_width = Some(PresetSize::Proportion(0.5));
    options.layout.sidebar.column_width_pairs = vec![niri_config::ColumnWidthPair {
        closed: Some(PresetSize::Proportion(0.5)),
        open: Some(PresetSize::Proportion(0.35)),
    }];

    let mut layout = check_ops_with_options(
        options.clone(),
        [
            Op::AddOutput(1),
            Op::AddWindow {
                params: TestWindowParams::new(1),
            },
            Op::AddWindow {
                params: TestWindowParams::new(2),
            },
            Op::FocusColumn(2),
            Op::MaximizeColumn,
        ],
    );

    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&1));
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);

    let ws = layout.active_workspace().unwrap();
    let working_w = ws.scrolling().working_area().size.w;
    let gaps = options.layout.gaps;
    let expected_full = ((working_w - gaps) * 1.0 - gaps).round() as i32;
    let width = test_tiling_window_width(&layout, 2);
    assert!(
        (width - expected_full).abs() <= 2,
        "full-width column should not be pair-swapped (got {width}, expected ~{expected_full})"
    );
}

#[test]
fn sidebar_clamps_max_column_proportion_when_closed() {
    let mut options = Options::default();
    options.layout.sidebar.max_column_proportion_when_closed = Some(0.5);

    let mut layout = check_ops_with_options(
        options.clone(),
        [
            Op::AddOutput(1),
            Op::AddWindow {
                params: TestWindowParams::new(1),
            },
            Op::AddWindow {
                params: TestWindowParams::new(2),
            },
            Op::FocusColumn(2),
            Op::SetColumnWidth(SizeChange::SetProportion(80.)),
        ],
    );

    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&1));
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);

    layout.toggle_sidebar(niri_config::SidebarSide::Left);
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);

    let ws = layout.active_workspace().unwrap();
    let working_w = ws.scrolling().working_area().size.w;
    let gaps = options.layout.gaps;
    let expected = ((working_w - gaps) * 0.5 - gaps).round() as i32;
    let width = test_tiling_window_width(&layout, 2);
    assert!(
        (width - expected).abs() <= 2,
        "expected column clamped to max 0.5 when closed (got {width}, expected ~{expected})"
    );
}

#[test]
fn sidebar_clamps_min_column_proportion_when_open() {
    let mut options = Options::default();
    options.layout.sidebar.min_column_proportion_when_open = Some(0.5);

    let mut layout = check_ops_with_options(
        options.clone(),
        [
            Op::AddOutput(1),
            Op::AddWindow {
                params: TestWindowParams::new(1),
            },
            Op::AddWindow {
                params: TestWindowParams::new(2),
            },
            Op::FocusColumn(2),
            Op::SetColumnWidth(SizeChange::SetProportion(30.)),
        ],
    );

    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&1));
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);

    let ws = layout.active_workspace().unwrap();
    let working_w = ws.scrolling().working_area().size.w;
    let gaps = options.layout.gaps;
    let expected = ((working_w - gaps) * 0.5 - gaps).round() as i32;
    let width = test_tiling_window_width(&layout, 2);
    assert!(
        (width - expected).abs() <= 2,
        "expected column clamped to min 0.5 when open (got {width}, expected ~{expected})"
    );
}

#[test]
fn sidebar_keep_proportions_preserves_non_default_column_pixels() {
    let mut options = Options::default();
    options.layout.default_column_width = Some(PresetSize::Proportion(0.5));
    options.layout.sidebar.keep_proportions_on_sidebar_inset = true;

    let mut layout = check_ops_with_options(
        options,
        [
            Op::AddOutput(1),
            Op::AddWindow {
                params: TestWindowParams::new(1),
            },
            Op::AddWindow {
                params: TestWindowParams::new(2),
            },
            Op::FocusColumn(2),
            Op::SetColumnWidth(SizeChange::SetProportion(100. / 3.)),
            Op::AddWindow {
                params: TestWindowParams::new(3),
            },
        ],
    );

    let default_col_width = test_tiling_window_width(&layout, 1);
    let third_col_width = test_tiling_window_width(&layout, 2);

    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&3));
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);

    let third_after = test_tiling_window_width(&layout, 2);
    let default_after = test_tiling_window_width(&layout, 1);
    assert!(
        (third_after - third_col_width).abs() <= 2,
        "non-default proportional column should keep its pixel width (was {third_col_width}, now {third_after})"
    );
    assert!(
        default_after < default_col_width,
        "default-matching column should still reflow smaller when the strip shrinks"
    );
}

fn assert_active_column_within_working_area(scrolling: &scrolling::ScrollingSpace<TestWindow>) {
    let area = scrolling.working_area();
    let active = scrolling.active_column_idx();
    let view_pos = scrolling.column_x_at(active) + scrolling.view_offset().current();
    let screen_x = scrolling.column_x_at(active) - view_pos;
    let col_w = scrolling.active_column_width();
    assert!(
        screen_x + 1e-3 >= area.loc.x,
        "active column left edge {screen_x} is left of working area {}",
        area.loc.x
    );
    assert!(
        screen_x + col_w <= area.loc.x + area.size.w + 1e-3,
        "active column right edge {} exceeds working area {}",
        screen_x + col_w,
        area.loc.x + area.size.w
    );
}

fn assert_all_columns_within_working_area(scrolling: &scrolling::ScrollingSpace<TestWindow>) {
    let area = scrolling.working_area();
    let active = scrolling.active_column_idx();
    let view_pos = scrolling.column_x_at(active) + scrolling.view_offset().current();
    for idx in 0..scrolling.column_count() {
        let screen_x = scrolling.column_x_at(idx) - view_pos;
        let col_w = scrolling.column_width_at(idx);
        assert!(
            screen_x + 1e-3 >= area.loc.x,
            "column {idx} left edge {screen_x} is left of working area {}",
            area.loc.x
        );
        assert!(
            screen_x + col_w <= area.loc.x + area.size.w + 1e-3,
            "column {idx} right edge {} exceeds working area {}",
            screen_x + col_w,
            area.loc.x + area.size.w
        );
    }
}

fn assert_column_group_centered_in_working_area(scrolling: &scrolling::ScrollingSpace<TestWindow>) {
    let n = scrolling.column_count();
    assert!(n >= 1);
    let area = scrolling.working_area();
    let active = scrolling.active_column_idx();
    let view_pos = scrolling.column_x_at(active) + scrolling.view_offset().current();
    let left = scrolling.column_x_at(0) - view_pos;
    let right = scrolling.column_x_at(n - 1) + scrolling.column_width_at(n - 1) - view_pos;
    let left_margin = left - area.loc.x;
    let right_margin = area.loc.x + area.size.w - right;
    assert!(
        (left_margin - right_margin).abs() < 1e-3,
        "column group should be centered in working area (left margin {left_margin} vs right {right_margin})"
    );
    assert_all_columns_within_working_area(scrolling);
}

#[test]
fn sidebar_inset_keeps_view_offset_in_sync() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
    ]);

    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&1));
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);

    let inset_open = {
        let ws = layout.active_workspace().unwrap();
        let scrolling = ws.scrolling();
        let area = scrolling.working_area();
        assert!(area.loc.x > 0.);
        assert!(
            !scrolling.view_offset().is_animation_ongoing(),
            "settled open sidebar must not leave a lagging view animation"
        );
        assert_all_columns_within_working_area(scrolling);
        assert_column_group_centered_in_working_area(scrolling);
        area.loc.x
    };

    layout.toggle_sidebar(niri_config::SidebarSide::Left);
    // Advance into the open/close spring so the inset has moved, then check that the
    // tiling camera tracks without a lagging horizontal view animation and keeps the
    // column group on screen / centered when it fits.
    let mut saw_mid_close = false;
    for _ in 0..60 {
        let mut now = layout.clock.now_unadjusted();
        now += Duration::from_millis(16);
        layout.clock.set_unadjusted(now);
        layout.advance_animations();

        let mon = layout.active_monitor_ref().unwrap();
        let sb = mon.left_sidebar.as_ref().unwrap();
        let animating = sb.are_animations_ongoing();
        let ws = mon.active_workspace_ref();
        let scrolling = ws.scrolling();
        let area = scrolling.working_area();
        if area.loc.x <= 0. || (area.loc.x - inset_open).abs() < 1e-3 {
            if !animating {
                break;
            }
            continue;
        }

        assert!(area.loc.x > 0. && area.loc.x < inset_open);
        assert!(
            !scrolling.view_offset().is_animation_ongoing(),
            "sidebar inset sync must not restart horizontal view movement"
        );
        assert_all_columns_within_working_area(scrolling);
        saw_mid_close = true;
        break;
    }
    assert!(
        saw_mid_close,
        "expected a mid-close inset sample while the sidebar open/close anim runs"
    );

    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    {
        let sb = layout
            .active_monitor_ref()
            .unwrap()
            .left_sidebar
            .as_ref()
            .unwrap();
        assert_eq!(sb.reserved_inset(), 0.);
        let ws = layout.active_workspace().unwrap();
        let scrolling = ws.scrolling();
        assert_eq!(scrolling.working_area().loc.x, 0.);
        assert!(!scrolling.view_offset().is_animation_ongoing());
        assert_column_group_centered_in_working_area(scrolling);
    }
}

#[test]
fn sidebar_left_inset_keeps_right_aligned_column_on_screen() {
    let mut options = Options::default();
    options.layout.default_column_width = Some(PresetSize::Proportion(0.5));
    options.layout.gaps = 0.;

    let mut layout = check_ops_with_options(
        options,
        [
            Op::AddOutput(1),
            Op::AddWindow {
                params: TestWindowParams::new(1),
            },
            Op::AddWindow {
                params: TestWindowParams::new(2),
            },
            Op::AddWindow {
                params: TestWindowParams::new(3),
            },
            // Scroll so the last column is brought into view (typically right-aligned by fit).
            Op::FocusColumnLast,
            Op::CompleteAnimations,
        ],
    );

    // Put window 1 in the left sidebar; tiled focus stays on a later column.
    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&1));
    check_ops_on_layout(
        &mut layout,
        [Op::FocusWindow(3), Op::CompleteAnimations],
    );

    {
        let scrolling = layout.active_workspace().unwrap().scrolling();
        assert_active_column_within_working_area(scrolling);
        assert!(!scrolling.view_offset().is_animation_ongoing());
    }

    // Mid-close and settled: column that fits must remain inside the working area.
    layout.toggle_sidebar(niri_config::SidebarSide::Left);
    let mut saw_mid = false;
    for _ in 0..60 {
        let mut now = layout.clock.now_unadjusted();
        now += Duration::from_millis(16);
        layout.clock.set_unadjusted(now);
        layout.advance_animations();

        let mon = layout.active_monitor_ref().unwrap();
        let Some(sb) = mon.left_sidebar.as_ref() else {
            break;
        };
        let inset = sb.reserved_inset();
        if inset <= 0. {
            if !sb.are_animations_ongoing() {
                break;
            }
            continue;
        }
        let scrolling = mon.active_workspace_ref().scrolling();
        assert!(!scrolling.view_offset().is_animation_ongoing());
        assert_active_column_within_working_area(scrolling);
        saw_mid = true;
        break;
    }
    assert!(saw_mid, "expected a mid-close inset sample");

    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    let scrolling = layout.active_workspace().unwrap().scrolling();
    assert_active_column_within_working_area(scrolling);
}

#[test]
fn sidebar_right_inset_keeps_column_on_screen() {
    let mut options = Options::default();
    options.layout.default_column_width = Some(PresetSize::Proportion(0.5));
    options.layout.gaps = 0.;

    let mut layout = check_ops_with_options(
        options,
        [
            Op::AddOutput(1),
            Op::AddWindow {
                params: TestWindowParams::new(1),
            },
            Op::AddWindow {
                params: TestWindowParams::new(2),
            },
            Op::AddWindow {
                params: TestWindowParams::new(3),
            },
            Op::FocusColumnLast,
            Op::CompleteAnimations,
        ],
    );

    layout.move_window_to_sidebar(niri_config::SidebarSide::Right, Some(&1));
    check_ops_on_layout(
        &mut layout,
        [Op::FocusWindow(3), Op::CompleteAnimations],
    );

    {
        let scrolling = layout.active_workspace().unwrap().scrolling();
        // Right inset has no delta_left; fit/center must still keep the column on screen.
        assert!(scrolling.working_area().loc.x == 0.);
        assert!(scrolling.working_area().size.w < 1280.);
        assert_active_column_within_working_area(scrolling);
        assert!(!scrolling.view_offset().is_animation_ongoing());
    }

    layout.toggle_sidebar(niri_config::SidebarSide::Right);
    for _ in 0..60 {
        let mut now = layout.clock.now_unadjusted();
        now += Duration::from_millis(16);
        layout.clock.set_unadjusted(now);
        layout.advance_animations();

        let mon = layout.active_monitor_ref().unwrap();
        let Some(sb) = mon.right_sidebar.as_ref() else {
            break;
        };
        if sb.reserved_inset() <= 0. {
            if !sb.are_animations_ongoing() {
                break;
            }
            continue;
        }
        let scrolling = mon.active_workspace_ref().scrolling();
        assert!(!scrolling.view_offset().is_animation_ongoing());
        assert_active_column_within_working_area(scrolling);
        break;
    }

    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    let scrolling = layout.active_workspace().unwrap().scrolling();
    assert_active_column_within_working_area(scrolling);
}

#[test]
fn sidebar_centers_column_group_that_fits() {
    let mut options = Options::default();
    // Two 0.3 columns fit easily in the strip with or without a sidebar.
    options.layout.default_column_width = Some(PresetSize::Proportion(0.3));
    options.layout.gaps = 0.;

    let mut layout = check_ops_with_options(
        options,
        [
            Op::AddOutput(1),
            Op::AddWindow {
                params: TestWindowParams::new(1),
            },
            Op::AddWindow {
                params: TestWindowParams::new(2),
            },
            Op::AddWindow {
                params: TestWindowParams::new(3),
            },
            Op::CompleteAnimations,
        ],
    );

    // Move one window into the left sidebar; remaining two still fit and must be
    // centered as a group in the reduced strip (not only the active column).
    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&1));
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);

    {
        let scrolling = layout.active_workspace().unwrap().scrolling();
        assert!(scrolling.working_area().loc.x > 0.);
        assert_eq!(scrolling.column_count(), 2);
        assert_column_group_centered_in_working_area(scrolling);
        assert!(!scrolling.view_offset().is_animation_ongoing());
    }

    layout.toggle_sidebar(niri_config::SidebarSide::Left);
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);

    let scrolling = layout.active_workspace().unwrap().scrolling();
    assert_eq!(scrolling.working_area().loc.x, 0.);
    assert_eq!(scrolling.column_count(), 2);
    assert_column_group_centered_in_working_area(scrolling);
    assert!(!scrolling.view_offset().is_animation_ongoing());
}

#[test]
fn center_visible_columns_always_centers_group_that_fits() {
    let mut options = Options::default();
    options.layout.gaps = 0.;
    options.layout.center_visible_columns = CenterVisibleColumns::Always;

    let mut layout = check_ops_with_options(
        options,
        [
            Op::AddOutput(1),
            Op::AddWindow {
                params: TestWindowParams::new(1),
            },
            Op::SetColumnWidth(SizeChange::SetFixed(300)),
            Op::Communicate(1),
            Op::AddWindow {
                params: TestWindowParams::new(2),
            },
            Op::SetColumnWidth(SizeChange::SetFixed(300)),
            Op::Communicate(2),
            Op::AddWindow {
                params: TestWindowParams::new(3),
            },
            Op::SetColumnWidth(SizeChange::SetFixed(300)),
            Op::Communicate(3),
            Op::CompleteAnimations,
        ],
    );

    {
        let scrolling = layout.active_workspace().unwrap().scrolling();
        assert_eq!(scrolling.column_count(), 3);
        assert_column_group_centered_in_working_area(scrolling);
    }

    check_ops_on_layout(
        &mut layout,
        [Op::FocusColumnLeft, Op::CompleteAnimations],
    );
    let scrolling = layout.active_workspace().unwrap().scrolling();
    assert_column_group_centered_in_working_area(scrolling);
}

#[test]
fn center_visible_columns_always_falls_through_when_strip_overflows() {
    let mut options = Options::default();
    // Three 600px columns on a 1280px output cannot all fit.
    options.layout.gaps = 0.;
    options.layout.center_visible_columns = CenterVisibleColumns::Always;

    let layout = check_ops_with_options(
        options,
        [
            Op::AddOutput(1),
            Op::AddWindow {
                params: TestWindowParams::new(1),
            },
            Op::SetColumnWidth(SizeChange::SetFixed(600)),
            Op::Communicate(1),
            Op::AddWindow {
                params: TestWindowParams::new(2),
            },
            Op::SetColumnWidth(SizeChange::SetFixed(600)),
            Op::Communicate(2),
            Op::AddWindow {
                params: TestWindowParams::new(3),
            },
            Op::SetColumnWidth(SizeChange::SetFixed(600)),
            Op::Communicate(3),
            Op::CompleteAnimations,
        ],
    );

    let scrolling = layout.active_workspace().unwrap().scrolling();
    assert_eq!(scrolling.column_count(), 3);
    assert_active_column_within_working_area(scrolling);
    // At least one column should be off-screen (strip does not fit).
    let area = scrolling.working_area();
    let active = scrolling.active_column_idx();
    let view_pos = scrolling.column_x_at(active) + scrolling.view_offset().current();
    let any_off = (0..scrolling.column_count()).any(|idx| {
        let screen_x = scrolling.column_x_at(idx) - view_pos;
        let col_w = scrolling.column_width_at(idx);
        screen_x + 1e-3 < area.loc.x
            || screen_x + col_w > area.loc.x + area.size.w + 1e-3
    });
    assert!(any_off, "expected the overflowing strip to leave some column off-screen");
}

#[test]
fn center_visible_columns_always_yields_to_center_focused_column() {
    let mut options = Options::default();
    options.layout.gaps = 0.;
    options.layout.center_visible_columns = CenterVisibleColumns::Always;
    options.layout.center_focused_column = CenterFocusedColumn::Always;

    let layout = check_ops_with_options(
        options,
        [
            Op::AddOutput(1),
            Op::AddWindow {
                params: TestWindowParams::new(1),
            },
            Op::SetColumnWidth(SizeChange::SetFixed(300)),
            Op::Communicate(1),
            Op::AddWindow {
                params: TestWindowParams::new(2),
            },
            Op::SetColumnWidth(SizeChange::SetFixed(300)),
            Op::Communicate(2),
            Op::AddWindow {
                params: TestWindowParams::new(3),
            },
            Op::SetColumnWidth(SizeChange::SetFixed(300)),
            Op::Communicate(3),
            Op::CompleteAnimations,
        ],
    );

    let scrolling = layout.active_workspace().unwrap().scrolling();
    assert!(scrolling.is_centering_focused_column());
    assert!(!scrolling.is_centering_visible_columns());
    assert_active_column_within_working_area(scrolling);

    // Active column alone is centered; the group as a whole is not.
    let area = scrolling.working_area();
    let active = scrolling.active_column_idx();
    let view_pos = scrolling.column_x_at(active) + scrolling.view_offset().current();
    let screen_x = scrolling.column_x_at(active) - view_pos;
    let col_w = scrolling.column_width_at(active);
    let left_margin = screen_x - area.loc.x;
    let right_margin = area.loc.x + area.size.w - (screen_x + col_w);
    assert!(
        (left_margin - right_margin).abs() < 1e-3,
        "active column should be centered when center-focused-column Always wins"
    );

    let n = scrolling.column_count();
    let left = scrolling.column_x_at(0) - view_pos;
    let right = scrolling.column_x_at(n - 1) + scrolling.column_width_at(n - 1) - view_pos;
    let group_left = left - area.loc.x;
    let group_right = area.loc.x + area.size.w - right;
    assert!(
        (group_left - group_right).abs() > 1.,
        "group should not be centered when focused-column centering takes precedence"
    );
}

#[test]
fn sidebar_discrete_width_animates_then_settles() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
    ]);

    // Window 1 stays in the tiling strip; window 2 goes into the sidebar.
    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&2));
    layout.focus_sidebar(niri_config::SidebarSide::Left);
    let before = layout
        .active_monitor_ref()
        .unwrap()
        .left_sidebar
        .as_ref()
        .unwrap()
        .width();
    let tiling_size_before = layout
        .windows()
        .find(|(_, w)| *w.id() == 1)
        .unwrap()
        .1
        .requested_size();

    layout.set_sidebar_width(niri_ipc::SizeChange::SetProportion(40.));
    {
        let sb = layout
            .active_monitor_ref()
            .unwrap()
            .left_sidebar
            .as_ref()
            .unwrap();
        assert!(
            sb.are_animations_ongoing(),
            "discrete width change must start width_anim"
        );
        assert_ne!(sb.width(), before);
        // Sidebar tile was sized to the target width once with animate=true.
        let sidebar_win = sb.windows().next().unwrap();
        assert!(
            sidebar_win.0.animate_next_configure.get(),
            "sidebar tile must request animated size for discrete width"
        );
    }

    // Clear leftover animate flags from earlier ops, then sync mid-width-anim.
    layout
        .windows()
        .find(|(_, w)| *w.id() == 1)
        .unwrap()
        .1
        .0
        .animate_next_configure
        .set(false);
    let now = layout.clock.now_unadjusted() + Duration::from_millis(50);
    layout.clock.set_unadjusted(now);
    layout.advance_animations();
    let tiling_win = layout
        .windows()
        .find(|(_, w)| *w.id() == 1)
        .unwrap()
        .1;
    let tiling_size_mid = tiling_win.requested_size();
    // Inset animation should drive a new size request on the tiling window.
    assert_ne!(tiling_size_before, tiling_size_mid);
    // Main-strip sizes lockstep without window-resize (avoids fighting the moving inset).
    assert!(
        !tiling_win.0.animate_next_configure.get(),
        "tiling window must not arm animate_next_configure during sidebar width inset sync"
    );

    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    {
        let sb = layout
            .active_monitor_ref()
            .unwrap()
            .left_sidebar
            .as_ref()
            .unwrap();
        assert!(!sb.are_animations_ongoing() || sb.open_progress() == 1.);
        assert!((sb.display_width() - sb.width()).abs() < 1e-6);
        assert!((sb.reserved_inset() - sb.width()).abs() < 1e-6);
    }

    // Interactive edge resize applies immediately (no width anim).
    let w0 = layout
        .active_monitor_ref()
        .unwrap()
        .left_sidebar
        .as_ref()
        .unwrap()
        .width();
    let sidebar_req_before = layout
        .windows()
        .find(|(_, w)| *w.id() == 2)
        .unwrap()
        .1
        .requested_size();
    // Clear animate flag from the discrete path so we can observe the interactive request.
    layout
        .windows()
        .find(|(_, w)| *w.id() == 2)
        .unwrap()
        .1
        .0
        .animate_next_configure
        .set(false);
    layout
        .active_monitor()
        .unwrap()
        .left_sidebar
        .as_mut()
        .unwrap()
        .resize_by_edges(ResizeEdge::RIGHT, Point::from((20., 0.)));
    {
        let sb = layout
            .active_monitor_ref()
            .unwrap()
            .left_sidebar
            .as_ref()
            .unwrap();
        assert!(
            !sb.are_animations_ongoing() || sb.open_progress() == 1.,
            "interactive resize must not leave a width_anim"
        );
        assert!((sb.display_width() - sb.width()).abs() < 1e-6);
        assert!((sb.width() - (w0 + 20.)).abs() < 1e-6);
    }
    let sidebar_win = layout
        .windows()
        .find(|(_, w)| *w.id() == 2)
        .unwrap()
        .1;
    assert_ne!(sidebar_req_before, sidebar_win.requested_size());
    assert!(
        !sidebar_win.0.animate_next_configure.get(),
        "interactive edge resize must request size without animation"
    );
}

#[test]
fn sidebar_open_animation_and_close_window() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
    ]);

    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&1));
    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&2));
    layout.start_open_animation_for_window(&1);
    layout.verify_invariants();

    // Closing one window reflows the stack; sidebar remains for the other.
    check_ops_on_layout(&mut layout, [Op::CloseWindow(1)]);
    layout.verify_invariants();
    let sb = layout
        .active_monitor_ref()
        .unwrap()
        .left_sidebar
        .as_ref()
        .unwrap();
    assert!(!sb.contains(&1));
    assert!(sb.contains(&2));
}

#[test]
fn sidebar_edge_drop_shows_insert_hint() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::InteractiveMoveBegin {
            window: 1,
            output_idx: 1,
            px: 200.,
            py: 100.,
        },
        // Exceed start threshold, then hold pointer in the left sidebar edge zone.
        Op::InteractiveMoveUpdate {
            window: 1,
            dx: 300.,
            dy: 0.,
            output_idx: 1,
            px: 10.,
            py: 100.,
        },
    ]);

    let output = layout.outputs().next().unwrap().clone();
    layout.update_render_elements(Some(&output));

    let mon = layout.active_monitor_ref().unwrap();
    let hint = mon.insert_hint.as_ref().expect("sidebar insert hint");
    assert!(matches!(
        hint.position,
        super::InsertPosition::Sidebar(niri_config::SidebarSide::Left)
    ));
    let loc = mon
        .insert_hint_render_loc
        .expect("sidebar insert hint overlay");
    assert!(loc.size.w > 0. && loc.size.h > 0.);
    assert!(loc.location.x < mon.view_size().w / 2.);
    assert!(
        loc.fixed_to_output,
        "output-scoped sidebar insert hint must render in screen space"
    );
}

#[test]
fn sidebar_open_panel_drop_adds_window_mid_panel() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
    ]);

    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&1));
    layout.verify_invariants();

    let panel = layout
        .active_monitor_ref()
        .unwrap()
        .left_sidebar
        .as_ref()
        .expect("open left sidebar")
        .panel_rect();
    let trigger = layout
        .active_monitor_ref()
        .unwrap()
        .options
        .gestures
        .sidebar_edge_drop
        .trigger_width;
    // Mid-panel, clearly outside the thin edge strip.
    let px = (panel.loc.x + panel.size.w * 0.5).max(trigger + 10.);
    assert!(
        px > trigger,
        "test pointer must be outside edge strip (px={px}, trigger={trigger})"
    );
    let py = panel.loc.y + panel.size.h * 0.5;

    let output = layout.outputs().next().unwrap().clone();
    assert!(layout.interactive_move_begin(2, &output, Point::from((640., 100.))));
    assert!(layout.interactive_move_update(
        &2,
        Point::from((400., 0.)),
        output.clone(),
        Point::from((px, py)),
    ));
    layout.update_render_elements(Some(&output));

    let mon = layout.active_monitor_ref().unwrap();
    let hint = mon.insert_hint.as_ref().expect("sidebar insert hint over open panel");
    assert!(matches!(
        hint.position,
        super::InsertPosition::Sidebar(niri_config::SidebarSide::Left)
    ));

    layout.interactive_move_end(&2);
    layout.verify_invariants();

    let sb = layout
        .active_monitor_ref()
        .unwrap()
        .left_sidebar
        .as_ref()
        .expect("left sidebar after drop");
    assert!(sb.contains(&1));
    assert!(sb.contains(&2), "mid-panel drop must add window to open sidebar");
}

#[test]
fn sidebar_absent_mid_panel_does_not_drop_without_edge() {
    // With no open sidebar, only the edge strip docks — mid-panel must not synthesize a drop.
    let mut layout = check_ops([
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::InteractiveMoveBegin {
            window: 1,
            output_idx: 1,
            px: 640.,
            py: 100.,
        },
        Op::InteractiveMoveUpdate {
            window: 1,
            dx: 400.,
            dy: 0.,
            output_idx: 1,
            // Default preset panel is ~20% (~256px); mid of that is far outside trigger_width.
            px: 120.,
            py: 100.,
        },
    ]);

    let output = layout.outputs().next().unwrap().clone();
    layout.update_render_elements(Some(&output));

    let mon = layout.active_monitor_ref().unwrap();
    assert!(
        !matches!(
            mon.insert_hint.as_ref().map(|h| h.position),
            Some(super::InsertPosition::Sidebar(_))
        ),
        "absent sidebar must not accept mid-panel drops; use edge strip only"
    );
}

#[test]
fn sidebar_output_windows_for_output_and_update_window() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
    ]);

    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&1));
    layout.verify_invariants();

    let output = layout.outputs().next().unwrap().clone();
    assert!(
        layout
            .windows_for_output(&output)
            .any(|win| win.id() == &1),
        "output-scoped sidebar window must be in windows_for_output (frame callbacks)"
    );

    // Commit plumbing must find the sidebar tile (no panic / no silent miss).
    layout.update_window(&1, None);
    layout.verify_invariants();

    layout.activate_window_without_raising(&1);
    assert_eq!(
        layout.active_monitor_ref().unwrap().sidebar_focus,
        Some(niri_config::SidebarSide::Left)
    );
}

#[test]
fn dnd_hold_over_output_sidebar_does_not_panic() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
    ]);

    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&1));
    layout.verify_invariants();

    // Focus a tiling window so the sidebar is not already focused.
    layout.activate_window(&2);
    assert!(layout.active_monitor_ref().unwrap().sidebar_focus.is_none());

    let (px, py) = {
        let mon = layout.active_monitor_ref().unwrap();
        let panel = mon.left_sidebar.as_ref().unwrap().panel_rect();
        (
            panel.loc.x + panel.size.w / 2.,
            panel.loc.y + panel.size.h / 2.,
        )
    };

    let output = layout.outputs().next().unwrap().clone();
    layout.dnd_update(output, Point::from((px, py)));

    // First tick arms the hold timer; second tick past 750ms fires activation.
    // Previously this panicked: workspace-only `.unwrap()` on output-scoped sidebars.
    check_ops_on_layout(
        &mut layout,
        [
            Op::AdvanceAnimations { msec_delta: 0 },
            Op::AdvanceAnimations { msec_delta: 800 },
        ],
    );

    assert_eq!(
        layout.active_monitor_ref().unwrap().sidebar_focus,
        Some(niri_config::SidebarSide::Left),
        "DnD hold over output sidebar must activate it without panicking"
    );

    layout.dnd_end();
    layout.verify_invariants();
}

#[test]
fn dnd_hold_over_scratchpad_does_not_panic() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
    ]);

    layout.move_window_to_scratchpad(Some(&1));
    layout.toggle_scratchpad();
    layout.verify_invariants();
    assert_eq!(
        layout.active_monitor_ref().unwrap().scratchpad.shown_id(),
        Some(&1)
    );

    layout.activate_window(&2);

    let (px, py) = {
        let mon = layout.active_monitor_ref().unwrap();
        let (tile, tile_pos) = mon.scratchpad.shown_tile_pos().unwrap();
        let size = tile.tile_size();
        // Scratchpad hit-test is in workspace space; geo.loc is (0,0) with one output.
        (tile_pos.x + size.w / 2., tile_pos.y + size.h / 2.)
    };

    assert_eq!(
        layout
            .active_monitor_ref()
            .unwrap()
            .window_under(Point::from((px, py)))
            .map(|(w, _)| *w.id()),
        Some(1),
        "test pointer must hit the shown scratchpad window"
    );

    let output = layout.outputs().next().unwrap().clone();
    layout.dnd_update(output, Point::from((px, py)));

    // Must not panic: same workspace-only unwrap bug as output sidebars.
    check_ops_on_layout(
        &mut layout,
        [
            Op::AdvanceAnimations { msec_delta: 0 },
            Op::AdvanceAnimations { msec_delta: 800 },
        ],
    );

    layout.dnd_end();
    layout.verify_invariants();
}

#[test]
fn sidebar_workspace_scope_update_window() {
    let options = Options {
        layout: niri_config::Layout {
            sidebar: niri_config::Sidebar {
                scope: niri_config::SidebarScope::Workspace,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    let mut layout = check_ops_with_options(
        options,
        [
            Op::AddOutput(1),
            Op::AddWindow {
                params: TestWindowParams::new(1),
            },
        ],
    );

    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&1));
    layout.verify_invariants();
    assert!(layout
        .active_monitor_ref()
        .unwrap()
        .active_workspace_ref()
        .left_sidebar
        .as_ref()
        .is_some_and(|s| s.contains(&1)));

    layout.update_window(&1, None);
    layout.verify_invariants();

    layout.activate_window_without_raising(&1);
    assert!(layout
        .active_monitor_ref()
        .unwrap()
        .active_workspace_ref()
        .left_sidebar
        .as_ref()
        .is_some_and(|s| s.is_focused()));
}

#[test]
fn sidebar_output_scope_survives_workspace_switch_and_overview() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
    ]);

    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&1));
    layout.verify_invariants();

    layout.switch_workspace_down();
    // Finish the switch animation so only the active card remains on-screen.
    for _ in 0..600 {
        layout.advance_animations();
        if layout
            .active_monitor_ref()
            .unwrap()
            .workspace_switch
            .is_none()
        {
            break;
        }
    }
    layout.verify_invariants();
    {
        let mon = layout.active_monitor_ref().unwrap();
        let idx = mon.active_workspace_idx();
        assert!(idx >= 1, "regression needs active workspace index ≥ 1");
        assert!(mon
            .left_sidebar
            .as_ref()
            .is_some_and(|s| s.contains(&1)));
        // Uncullled geo must resolve so render_workspaces can place scratchpad on the active card.
        assert!(mon.active_workspace_render_geo().is_some());
        // After settle, culled `.nth(idx)` is the old bug (often None when idx ≥ 1).
        assert!(
            mon.workspaces_with_render_geo().nth(idx).is_none(),
            "culled geo .nth(active_idx) should miss after switch; use active_workspace_render_geo"
        );
        let output = mon.output().clone();
        assert!(layout
            .windows_for_output(&output)
            .any(|win| win.id() == &1));
    }

    layout.toggle_overview();
    layout.verify_invariants();
    {
        let mon = layout.active_monitor_ref().unwrap();
        let sb = mon
            .left_sidebar
            .as_ref()
            .expect("output sidebar must survive overview");
        assert!(sb.contains(&1));
        // Screen-space hit-test: fixed sidebars remain hittable while overview is open.
        let panel = sb.panel_rect();
        let pos = Point::from((
            panel.loc.x + panel.size.w / 2.,
            panel.loc.y + panel.size.h / 2.,
        ));
        let output = mon.output().clone();
        let (win, _) = layout
            .window_under(&output, pos)
            .expect("output sidebar must hit-test in overview");
        assert_eq!(win.id(), &1);
    }
    layout.toggle_overview();
    layout.verify_invariants();
}

#[test]
fn sidebar_scope_reload_migrates_to_output() {
    let options = Options {
        layout: niri_config::Layout {
            sidebar: niri_config::Sidebar {
                scope: niri_config::SidebarScope::Workspace,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    let mut layout = check_ops_with_options(
        options,
        [
            Op::AddOutput(1),
            Op::AddWindow {
                params: TestWindowParams::new(1),
            },
        ],
    );

    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&1));
    assert!(layout
        .active_monitor_ref()
        .unwrap()
        .active_workspace_ref()
        .left_sidebar
        .as_ref()
        .is_some_and(|s| s.contains(&1)));

    Op::UpdateConfig {
        layout_config: Box::new(niri_config::LayoutPart {
            sidebar: Some(niri_config::SidebarPart {
                scope: Some(niri_config::SidebarScope::Output),
                ..Default::default()
            }),
            ..Default::default()
        }),
    }
    .apply(&mut layout);
    layout.verify_invariants();

    let mon = layout.active_monitor_ref().unwrap();
    assert!(mon
        .left_sidebar
        .as_ref()
        .is_some_and(|s| s.contains(&1)));
    assert!(mon
        .active_workspace_ref()
        .left_sidebar
        .as_ref()
        .map(|s| !s.contains(&1))
        .unwrap_or(true));
}

#[test]
fn sidebar_toggle_and_cross_side_binds() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
    ]);

    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&1));
    assert!(layout
        .active_monitor_ref()
        .unwrap()
        .left_sidebar
        .as_ref()
        .is_some_and(|s| s.contains(&1)));

    // Same side again toggles out.
    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&1));
    layout.verify_invariants();
    assert!(layout
        .active_monitor_ref()
        .unwrap()
        .left_sidebar
        .is_none());

    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&1));
    layout.move_window_to_sidebar(niri_config::SidebarSide::Right, Some(&1));
    layout.verify_invariants();
    let mon = layout.active_monitor_ref().unwrap();
    assert!(mon.left_sidebar.is_none());
    assert!(mon
        .right_sidebar
        .as_ref()
        .is_some_and(|s| s.contains(&1)));
}

#[test]
fn sidebar_interactive_move_out_to_tiling() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
    ]);

    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&1));
    let output = layout.outputs().next().unwrap().clone();

    assert!(layout.interactive_move_begin(1, &output, Point::from((50., 100.))));
    // Cross start threshold with pointer mid-output (outside edge strip).
    assert!(layout.interactive_move_update(
        &1,
        Point::from((300., 0.)),
        output.clone(),
        Point::from((640., 100.)),
    ));
    layout.interactive_move_end(&1);
    layout.verify_invariants();

    let mon = layout.active_monitor_ref().unwrap();
    assert!(mon.left_sidebar.is_none());
    assert!(mon.active_workspace_ref().has_window(&1));
}

#[test]
fn sidebar_focus_column_left_right_enters_and_leaves() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
    ]);

    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&1));
    layout.verify_invariants();

    // Focus the remaining tiling window, then focus left at the edge → sidebar.
    layout.activate_window(&2);
    layout.focus_left();
    assert_eq!(
        layout.active_monitor_ref().unwrap().sidebar_focus,
        Some(niri_config::SidebarSide::Left)
    );

    // Inward from left sidebar returns to tiling.
    layout.focus_right();
    assert!(layout
        .active_monitor_ref()
        .unwrap()
        .sidebar_focus
        .is_none());
    assert_eq!(
        layout
            .active_workspace()
            .and_then(|ws| ws.active_window())
            .map(|w| *w.id()),
        Some(2)
    );
}

#[test]
fn sidebar_active_window_visual_rectangle_follows_sidebar_focus() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
    ]);

    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&1));
    layout.activate_window(&2);

    let tiling_rect = layout
        .active_monitor_ref()
        .unwrap()
        .active_window_visual_rectangle()
        .expect("tiling window visual rect");

    layout.focus_left();
    assert_eq!(
        layout.active_monitor_ref().unwrap().sidebar_focus,
        Some(niri_config::SidebarSide::Left)
    );
    assert_eq!(layout.focus().map(|w| *w.id()), Some(1));

    let sidebar_rect = layout
        .active_monitor_ref()
        .unwrap()
        .active_window_visual_rectangle()
        .expect("sidebar window visual rect");

    // Left sidebar window should sit further left than the tiling column.
    assert!(
        sidebar_rect.loc.x < tiling_rect.loc.x,
        "sidebar rect {sidebar_rect:?} should be left of tiling rect {tiling_rect:?}"
    );
}

#[test]
fn sidebar_focus_window_or_workspace_up_down_moves_within_stack() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::AddWindow {
            params: TestWindowParams::new(3),
        },
    ]);

    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&1));
    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&2));
    layout.focus_sidebar(niri_config::SidebarSide::Left);
    layout.verify_invariants();

    // Last pushed window is active.
    assert_eq!(layout.focus().map(|w| *w.id()), Some(2));
    assert!(layout.focus_up());
    assert_eq!(layout.focus().map(|w| *w.id()), Some(1));
    assert!(!layout.focus_up());
    assert_eq!(layout.focus().map(|w| *w.id()), Some(1));

    assert!(layout.focus_down());
    assert_eq!(layout.focus().map(|w| *w.id()), Some(2));
    assert!(!layout.focus_down());
    assert_eq!(layout.focus().map(|w| *w.id()), Some(2));

    // At the bottom of the stack, focus-window-or-workspace-down switches workspace.
    let ws_idx_before = layout.active_monitor_ref().unwrap().active_workspace_idx();
    layout.focus_window_or_workspace_down();
    assert_eq!(
        layout.active_monitor_ref().unwrap().active_workspace_idx(),
        ws_idx_before + 1
    );

    // Still on the sidebar; walk to the top, then focus-window-or-workspace-up returns.
    assert_eq!(
        layout.active_monitor_ref().unwrap().sidebar_focus,
        Some(niri_config::SidebarSide::Left)
    );
    assert!(layout.focus_up());
    assert_eq!(layout.focus().map(|w| *w.id()), Some(1));
    assert!(!layout.focus_up());

    layout.focus_window_or_workspace_up();
    assert_eq!(
        layout.active_monitor_ref().unwrap().active_workspace_idx(),
        ws_idx_before
    );
}

#[test]
fn sidebar_move_window_up_down_reorders_stack() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::AddWindow {
            params: TestWindowParams::new(3),
        },
    ]);

    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&1));
    layout.move_window_to_sidebar(niri_config::SidebarSide::Left, Some(&2));
    layout.focus_sidebar(niri_config::SidebarSide::Left);
    layout.verify_invariants();

    // Push order: 1 on top, 2 active at bottom.
    let ids: Vec<_> = layout
        .active_monitor_ref()
        .unwrap()
        .left_sidebar
        .as_ref()
        .unwrap()
        .windows()
        .map(|w| *w.id())
        .collect();
    assert_eq!(ids, vec![1, 2]);
    assert_eq!(layout.focus().map(|w| *w.id()), Some(2));

    layout.move_up();
    layout.verify_invariants();
    let ids: Vec<_> = layout
        .active_monitor_ref()
        .unwrap()
        .left_sidebar
        .as_ref()
        .unwrap()
        .windows()
        .map(|w| *w.id())
        .collect();
    assert_eq!(ids, vec![2, 1]);
    assert_eq!(layout.focus().map(|w| *w.id()), Some(2));

    // Already at top: no-op.
    layout.move_up();
    layout.verify_invariants();
    let ids: Vec<_> = layout
        .active_monitor_ref()
        .unwrap()
        .left_sidebar
        .as_ref()
        .unwrap()
        .windows()
        .map(|w| *w.id())
        .collect();
    assert_eq!(ids, vec![2, 1]);
    assert_eq!(layout.focus().map(|w| *w.id()), Some(2));

    layout.move_down();
    layout.verify_invariants();
    let ids: Vec<_> = layout
        .active_monitor_ref()
        .unwrap()
        .left_sidebar
        .as_ref()
        .unwrap()
        .windows()
        .map(|w| *w.id())
        .collect();
    assert_eq!(ids, vec![1, 2]);
    assert_eq!(layout.focus().map(|w| *w.id()), Some(2));

    // Already at bottom: no-op (and or-to-workspace must not switch workspaces).
    let ws_idx_before = layout.active_monitor_ref().unwrap().active_workspace_idx();
    layout.move_down();
    layout.move_down_or_to_workspace_down();
    layout.verify_invariants();
    assert_eq!(
        layout.active_monitor_ref().unwrap().active_workspace_idx(),
        ws_idx_before
    );
    let ids: Vec<_> = layout
        .active_monitor_ref()
        .unwrap()
        .left_sidebar
        .as_ref()
        .unwrap()
        .windows()
        .map(|w| *w.id())
        .collect();
    assert_eq!(ids, vec![1, 2]);
    assert_eq!(layout.focus().map(|w| *w.id()), Some(2));
}

#[test]
fn move_column_to_workspace_down_focus_false_on_floating_window() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::ToggleWindowFloating { id: None },
        Op::MoveColumnToWorkspaceDown(false),
    ];

    let layout = check_ops(ops);

    let MonitorSet::Normal { monitors, .. } = layout.monitor_set else {
        unreachable!()
    };

    assert_eq!(monitors[0].active_workspace_idx, 0);
}

#[test]
fn move_column_to_workspace_focus_false_on_floating_window() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::ToggleWindowFloating { id: None },
        Op::MoveColumnToWorkspace(1, false),
    ];

    let layout = check_ops(ops);

    let MonitorSet::Normal { monitors, .. } = layout.monitor_set else {
        unreachable!()
    };

    assert_eq!(monitors[0].active_workspace_idx, 0);
}

#[test]
fn restore_to_floating_persists_across_fullscreen_maximize() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::ToggleWindowFloating { id: None },
        // Maximize then fullscreen.
        Op::MaximizeWindowToEdges { id: None },
        Op::FullscreenWindow(1),
        // Unfullscreen.
        Op::FullscreenWindow(1),
    ];

    let mut layout = check_ops(ops);

    // Unfullscreening should return the window to the maximized state.
    let scrolling = layout.active_workspace().unwrap().scrolling();
    assert!(scrolling.tiles().next().is_some());

    let ops = [
        // Unmaximize.
        Op::MaximizeWindowToEdges { id: None },
    ];
    check_ops_on_layout(&mut layout, ops);

    // Unmaximize should return the window back to floating.
    let scrolling = layout.active_workspace().unwrap().scrolling();
    assert!(scrolling.tiles().next().is_none());
}

#[test]
fn unmaximize_during_fullscreen_does_not_float() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::ToggleWindowFloating { id: None },
        // Maximize then fullscreen.
        Op::MaximizeWindowToEdges { id: None },
        Op::FullscreenWindow(1),
        // Unmaximize.
        Op::MaximizeWindowToEdges { id: None },
    ];

    let mut layout = check_ops(ops);

    // Unmaximize shouldn't have changed the window state since it's fullscreen.
    let scrolling = layout.active_workspace().unwrap().scrolling();
    assert!(scrolling.tiles().next().is_some());

    let ops = [
        // Unfullscreen.
        Op::FullscreenWindow(1),
    ];
    check_ops_on_layout(&mut layout, ops);

    // Unfullscreen should return the window back to floating.
    let scrolling = layout.active_workspace().unwrap().scrolling();
    assert!(scrolling.tiles().next().is_none());
}

#[test]
fn move_column_to_workspace_maximize_and_fullscreen() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::MaximizeWindowToEdges { id: None },
        Op::FullscreenWindow(1),
        Op::MoveColumnToWorkspaceDown(true),
        Op::FullscreenWindow(1),
    ];

    let layout = check_ops(ops);
    let (_, win) = layout.windows().next().unwrap();

    // Unfullscreening should return to maximized because the window was maximized before.
    assert_eq!(win.pending_sizing_mode(), SizingMode::Maximized);
}

#[test]
fn move_window_to_workspace_maximize_and_fullscreen() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::MaximizeWindowToEdges { id: None },
        Op::FullscreenWindow(1),
        Op::MoveWindowToWorkspaceDown(true),
        Op::FullscreenWindow(1),
    ];

    let layout = check_ops(ops);
    let (_, win) = layout.windows().next().unwrap();

    // Unfullscreening should return to maximized because the window was maximized before.
    //
    // FIXME: it currently doesn't because windows themselves can only be either fullscreen or
    // maximized. So when a window is fullscreen, whether it is also maximized or not is stored in
    // the column. MoveWindowToWorkspace removes the window from the column and this information is
    // forgotten.
    assert_eq!(win.pending_sizing_mode(), SizingMode::Normal);
}

#[test]
fn tabs_with_different_border() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams {
                rules: Some(ResolvedWindowRules {
                    border: niri_config::BorderRule {
                        on: true,
                        ..Default::default()
                    },
                    ..ResolvedWindowRules::default()
                }),
                ..TestWindowParams::new(2)
            },
        },
        Op::SwitchPresetWindowHeight { id: None },
        Op::ToggleColumnTabbedDisplay,
        Op::AddWindow {
            params: TestWindowParams::new(3),
        },
        Op::ConsumeOrExpelWindowLeft { id: None },
    ];

    let options = Options {
        layout: niri_config::Layout {
            struts: Struts {
                left: FloatOrInt(0.),
                right: FloatOrInt(0.),
                top: FloatOrInt(20000.),
                bottom: FloatOrInt(0.),
            },
            ..Default::default()
        },
        ..Default::default()
    };
    check_ops_with_options(options, ops);
}

#[test]
fn expel_pending_left_from_fullscreen_tabbed_column() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::FullscreenWindow(1),
        Op::Communicate(1),
        // 1 is now fullscreen, view_offset_to_restore is set.
        Op::ToggleColumnTabbedDisplay,
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::ConsumeOrExpelWindowLeft { id: Some(2) },
        // 2 is consumed into a fullscreen column, fullscreen is requested but not applied.
        //
        // Now, get it back out while keeping it focused.
        //
        // Importantly, we expel it *left*, which results in adding a new column with the exact
        // same active_column_idx.
        Op::FocusWindow(2),
        Op::ConsumeOrExpelWindowLeft { id: None },
    ];

    check_ops(ops);
}

#[test]
fn workspace_render_geo_at_fractional_scale() {
    let ops = [
        Op::AddScaledOutput {
            id: 1,
            scale: 1.1,
            layout_config: None,
        },
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::FocusWorkspaceDown,
        Op::CompleteAnimations,
    ];

    let layout = check_ops(ops);

    let MonitorSet::Normal { monitors, .. } = &layout.monitor_set else {
        unreachable!()
    };

    let mon = &monitors[0];
    let mut iter = mon.workspaces_with_render_geo();
    let (_ws, geo) = iter.next().unwrap();
    assert!(
        iter.next().is_none(),
        "animations are completed, only one workspace should be visible"
    );
    assert_eq!(
        geo.loc.y, 0.,
        "active workspace must be at y = 0 exactly, \
         otherwise a pointer against the screen edge at y = 0 won't hit it"
    );
}

fn parent_id_causes_loop(layout: &Layout<TestWindow>, id: usize, mut parent_id: usize) -> bool {
    if parent_id == id {
        return true;
    }

    'outer: loop {
        for (_, win) in layout.windows() {
            if win.0.id == parent_id {
                match win.0.parent_id.get() {
                    Some(new_parent_id) => {
                        if new_parent_id == id {
                            // Found a loop.
                            return true;
                        }

                        parent_id = new_parent_id;
                        continue 'outer;
                    }
                    // Reached window with no parent.
                    None => return false,
                }
            }
        }

        // Parent is not in the layout.
        return false;
    }
}

fn arbitrary_spacing() -> impl Strategy<Value = f64> {
    // Give equal weight to:
    // - 0: the element is disabled
    // - 4: some reasonable value
    // - random value, likely unreasonably big
    prop_oneof![Just(0.), Just(4.), ((1.)..=65535.)]
}

fn arbitrary_spacing_neg() -> impl Strategy<Value = f64> {
    // Give equal weight to:
    // - 0: the element is disabled
    // - 4: some reasonable value
    // - -4: some reasonable negative value
    // - random value, likely unreasonably big
    prop_oneof![Just(0.), Just(4.), Just(-4.), ((1.)..=65535.)]
}

fn arbitrary_struts() -> impl Strategy<Value = Struts> {
    (
        arbitrary_spacing_neg(),
        arbitrary_spacing_neg(),
        arbitrary_spacing_neg(),
        arbitrary_spacing_neg(),
    )
        .prop_map(|(left, right, top, bottom)| Struts {
            left: FloatOrInt(left),
            right: FloatOrInt(right),
            top: FloatOrInt(top),
            bottom: FloatOrInt(bottom),
        })
}

fn arbitrary_center_focused_column() -> impl Strategy<Value = CenterFocusedColumn> {
    prop_oneof![
        Just(CenterFocusedColumn::Never),
        Just(CenterFocusedColumn::OnOverflow),
        Just(CenterFocusedColumn::Always),
    ]
}

fn arbitrary_center_visible_columns() -> impl Strategy<Value = CenterVisibleColumns> {
    prop_oneof![
        Just(CenterVisibleColumns::Never),
        Just(CenterVisibleColumns::Always),
    ]
}

fn arbitrary_tab_indicator_position() -> impl Strategy<Value = TabIndicatorPosition> {
    prop_oneof![
        Just(TabIndicatorPosition::Left),
        Just(TabIndicatorPosition::Right),
        Just(TabIndicatorPosition::Top),
        Just(TabIndicatorPosition::Bottom),
    ]
}

prop_compose! {
    fn arbitrary_focus_ring()(
        off in any::<bool>(),
        width in prop::option::of(arbitrary_spacing().prop_map(FloatOrInt)),
    ) -> niri_config::BorderRule {
        niri_config::BorderRule {
            off,
            on: !off,
            width,
            ..Default::default()
        }
    }
}

prop_compose! {
    fn arbitrary_border()(
        off in any::<bool>(),
        width in prop::option::of(arbitrary_spacing().prop_map(FloatOrInt)),
    ) -> niri_config::BorderRule {
        niri_config::BorderRule {
            off,
            on: !off,
            width,
            ..Default::default()
        }
    }
}

prop_compose! {
    fn arbitrary_shadow()(
        off in any::<bool>(),
        softness in prop::option::of(arbitrary_spacing().prop_map(FloatOrInt)),
    ) -> niri_config::ShadowRule {
        niri_config::ShadowRule {
            off,
            on: !off,
            softness,
            ..Default::default()
        }
    }
}

prop_compose! {
    fn arbitrary_tab_indicator()(
        off in any::<bool>(),
        hide_when_single_tab in prop::option::of(any::<bool>().prop_map(Flag)),
        place_within_column in prop::option::of(any::<bool>().prop_map(Flag)),
        width in prop::option::of(arbitrary_spacing().prop_map(FloatOrInt)),
        gap in prop::option::of(arbitrary_spacing_neg().prop_map(FloatOrInt)),
        length in prop::option::of((0f64..2f64)
            .prop_map(|x| TabIndicatorLength { total_proportion: Some(x) })),
        position in prop::option::of(arbitrary_tab_indicator_position()),
    ) -> niri_config::TabIndicatorPart {
        niri_config::TabIndicatorPart {
            off,
            on: !off,
            hide_when_single_tab,
            place_within_column,
            width,
            gap,
            length,
            position,
            ..Default::default()
        }
    }
}

fn test_output(name: &str, size: (i32, i32)) -> Output {
    let output = Output::new(
        name.to_owned(),
        PhysicalProperties {
            size: Size::from(size),
            subpixel: Subpixel::Unknown,
            make: String::new(),
            model: String::new(),
            serial_number: String::new(),
        },
    );
    output.change_current_state(
        Some(Mode {
            size: Size::from(size),
            refresh: 60000,
        }),
        None,
        None,
        None,
    );
    output.user_data().insert_if_missing(|| OutputName {
        connector: name.to_owned(),
        make: None,
        model: None,
        serial: None,
    });
    output
}

fn requested_size(layout: &Layout<TestWindow>, id: usize) -> Size<i32, Logical> {
    layout
        .windows()
        .find(|(_, window)| *window.id() == id)
        .unwrap()
        .1
        .requested_size()
        .unwrap()
}

fn scrolling_positions(
    layout: &Layout<TestWindow>,
    output: &Output,
) -> Vec<(usize, Point<f64, Logical>)> {
    let mon = layout.monitor_for_output(output).unwrap();
    mon.active_workspace_ref()
        .tiles_with_render_positions()
        .map(|(tile, pos, _)| (*tile.window().id(), pos))
        .collect()
}

#[test]
fn vertical_main_axis_opens_rows_and_focuses_down() {
    let mut layout = Layout::default();
    let horizontal = test_output("DP-1", (1920, 1080));
    let vertical = test_output("DP-2", (1080, 1920));
    layout.add_output(horizontal.clone(), None);
    layout.add_output(
        vertical.clone(),
        Some(niri_config::LayoutPart {
            main_axis: Some(niri_config::MainAxis::Vertical),
            ..Default::default()
        }),
    );

    layout.add_window(
        TestWindow::new(TestWindowParams::new(1)),
        AddWindowTarget::Output(&horizontal),
        None,
        None,
        false,
        false,
        ActivateWindow::Yes,
    );
    layout.add_window(
        TestWindow::new(TestWindowParams::new(2)),
        AddWindowTarget::Output(&horizontal),
        None,
        None,
        false,
        false,
        ActivateWindow::No,
    );
    layout.verify_invariants();

    let horizontal_pos = scrolling_positions(&layout, &horizontal);
    let first = horizontal_pos.iter().find(|(id, _)| *id == 1).unwrap().1;
    let second = horizontal_pos.iter().find(|(id, _)| *id == 2).unwrap().1;
    assert!(
        second.x > first.x + 100.,
        "horizontal layout places the next window to the right: {first:?} {second:?}"
    );
    assert!(
        (second.y - first.y).abs() < 2.,
        "columns share the top of the working area: {first:?} {second:?}"
    );

    layout.focus_output(&horizontal);
    assert_eq!(layout.focus().unwrap().id(), &1usize);
    assert!(
        !layout.focus_down(),
        "screen-down stays inside a single-window column"
    );
    assert_eq!(layout.focus().unwrap().id(), &1usize);

    layout.add_window(
        TestWindow::new(TestWindowParams::new(3)),
        AddWindowTarget::Output(&vertical),
        None,
        None,
        false,
        false,
        ActivateWindow::Yes,
    );
    layout.add_window(
        TestWindow::new(TestWindowParams::new(4)),
        AddWindowTarget::Output(&vertical),
        None,
        None,
        false,
        false,
        ActivateWindow::No,
    );
    layout.verify_invariants();

    let vertical_pos = scrolling_positions(&layout, &vertical);
    let first = vertical_pos.iter().find(|(id, _)| *id == 3).unwrap().1;
    let second = vertical_pos.iter().find(|(id, _)| *id == 4).unwrap().1;
    assert!(
        second.y > first.y + 100.,
        "vertical layout places the next window below: {first:?} {second:?}"
    );
    assert!(
        (second.x - first.x).abs() < 2.,
        "rows share the left of the working area: {first:?} {second:?}"
    );

    layout.focus_output(&vertical);
    assert_eq!(layout.focus().unwrap().id(), &3usize);
    assert!(layout.focus_down());
    assert_eq!(
        layout.focus().unwrap().id(),
        &4usize,
        "screen-down moves to the next row"
    );
}

#[test]
fn moving_between_outputs_converts_column_and_row() {
    let mut layout = Layout::default();
    let horizontal = test_output("DP-1", (1920, 1080));
    let vertical = test_output("DP-2", (1080, 1920));
    layout.add_output(horizontal.clone(), None);
    layout.add_output(
        vertical.clone(),
        Some(niri_config::LayoutPart {
            main_axis: Some(niri_config::MainAxis::Vertical),
            ..Default::default()
        }),
    );

    layout.add_window(
        TestWindow::new(TestWindowParams::new(1)),
        AddWindowTarget::Output(&horizontal),
        None,
        None,
        false,
        false,
        ActivateWindow::Yes,
    );
    let column_size = requested_size(&layout, 1);
    // Default column width is half the strip. On the landscape output that is the width.
    assert!(column_size.w < column_size.h);
    assert!(column_size.w < 1920 / 2 + 32);

    layout.move_to_output(Some(&1), &vertical, None, ActivateWindow::Yes);
    layout.verify_invariants();
    let row_size = requested_size(&layout, 1);
    // The proportion becomes row height, and the window spans the portrait working width.
    assert!(
        row_size.w > 1000,
        "row uses the destination working width, got {row_size:?}"
    );
    assert!(
        row_size.h < row_size.w,
        "row height comes from the previous column width, got {row_size:?}"
    );
    assert_ne!(row_size, column_size);

    layout.move_to_output(Some(&1), &horizontal, None, ActivateWindow::Yes);
    layout.verify_invariants();
    assert_eq!(requested_size(&layout, 1), column_size);

    layout.add_window(
        TestWindow::new(TestWindowParams::new(2)),
        AddWindowTarget::Output(&horizontal),
        None,
        None,
        false,
        false,
        ActivateWindow::Yes,
    );
    let column_size = requested_size(&layout, 2);
    assert!(layout.interactive_move_begin(2, &horizontal, Point::from((100., 100.))));
    assert!(layout.interactive_move_update(
        &2,
        Point::from((400., 0.)),
        vertical.clone(),
        Point::from((200., 200.)),
    ));
    let preview = requested_size(&layout, 2);
    assert!(
        preview.w > 1000 && preview.h < preview.w,
        "drag preview retile to a row, got {preview:?} from column {column_size:?}"
    );

    layout.interactive_move_end(&2);
    layout.verify_invariants();
    let dropped = requested_size(&layout, 2);
    assert!(
        dropped.w > 1000 && dropped.h < dropped.w,
        "dropping commits the row, got {dropped:?}"
    );

    layout.move_to_output(Some(&2), &horizontal, None, ActivateWindow::Yes);
    layout.verify_invariants();
    assert_eq!(requested_size(&layout, 2), column_size);
}

prop_compose! {
    fn arbitrary_layout_part()(
        gaps in prop::option::of(arbitrary_spacing().prop_map(FloatOrInt)),
        struts in prop::option::of(arbitrary_struts()),
        focus_ring in prop::option::of(arbitrary_focus_ring()),
        border in prop::option::of(arbitrary_border()),
        shadow in prop::option::of(arbitrary_shadow()),
        tab_indicator in prop::option::of(arbitrary_tab_indicator()),
        center_focused_column in prop::option::of(arbitrary_center_focused_column()),
        center_visible_columns in prop::option::of(arbitrary_center_visible_columns()),
        always_center_single_column in prop::option::of(any::<bool>().prop_map(Flag)),
        empty_workspace_above_first in prop::option::of(any::<bool>().prop_map(Flag)),
    ) -> niri_config::LayoutPart {
        niri_config::LayoutPart {
            gaps,
            struts,
            center_focused_column,
            center_visible_columns,
            always_center_single_column,
            empty_workspace_above_first,
            focus_ring,
            border,
            shadow,
            tab_indicator,
            ..Default::default()
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: if std::env::var_os("RUN_SLOW_TESTS").is_none() {
            eprintln!("ignoring slow test");
            0
        } else {
            ProptestConfig::default().cases
        },
        ..ProptestConfig::default()
    })]

    #[test]
    fn random_operations_dont_panic(
        ops: Vec<Op>,
        layout_config in arbitrary_layout_part(),
    ) {
        // eprintln!("{ops:?}");
        let options = Options {
            layout: niri_config::Layout::from_part(&layout_config),
            ..Default::default()
        };

        check_ops_with_options(options, ops);
    }
}
