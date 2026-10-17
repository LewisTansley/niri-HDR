use crate::appearance::Color;
use crate::layout::{DefaultPresetSize, PresetSize};
use crate::utils::{Flag, MergeWith};
use crate::FloatOrInt;

#[derive(Debug, Clone, PartialEq)]
pub struct Sidebar {
    pub scope: SidebarScope,
    pub mode: SidebarMode,
    pub preset_widths: Vec<PresetSize>,
    pub left: SidebarSideConfig,
    pub right: SidebarSideConfig,
    pub border: SidebarBorder,
    /// Scooped (concave) corner radius on the workspace-facing edge.
    ///
    /// `None` means unset: `0` without DMS theming, or DMS `cornerRadius` when
    /// [`Self::use_dms_theme`] is enabled. `Some(0.)` disables scoops even with DMS.
    pub corner_radius: Option<f64>,
    pub use_dms_theme: bool,
    /// Hide sidebars when the active column is maximize-to-edges.
    pub hide_when_maximized: bool,
    /// Hide sidebars when the active column is fullscreen.
    pub hide_when_fullscreen: bool,
    /// Whether each dock shows only its side's shortcuts or both sides' lists.
    pub shortcuts_sharing: ShortcutsSharing,
    /// Linked open/closed tiling column widths. When non-empty, these take precedence over
    /// [`Self::default_column_width_when_closed`] / [`Self::default_column_width_when_open`] and
    /// over `layout.preset-column-widths` for Mod+R cycling on the tiling strip.
    pub column_width_pairs: Vec<ColumnWidthPair>,
    /// Default tiling column width while reserve sidebars contribute no inset (falls back to
    /// [`crate::Layout::default_column_width`] when unset). Ignored when
    /// [`Self::column_width_pairs`] is non-empty.
    pub default_column_width_when_closed: Option<PresetSize>,
    /// Default tiling column width while reserve sidebars contribute inset (falls back to layout
    /// default when unset). Ignored when [`Self::column_width_pairs`] is non-empty.
    pub default_column_width_when_open: Option<PresetSize>,
    /// When reserve sidebar insets change, keep the pixel width of proportional columns that do
    /// not match the current sidebar open/closed default (instead of reflowing to the same
    /// proportion of the narrower strip).
    pub keep_proportions_on_sidebar_inset: bool,
    /// Cap proportional tiling column widths when reserve sidebars contribute no inset.
    pub max_column_proportion_when_closed: Option<f64>,
    /// Floor proportional tiling column widths while reserve sidebars contribute inset.
    pub min_column_proportion_when_open: Option<f64>,
}

/// One linked closed↔open tiling column width for reserve sidebars.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColumnWidthPair {
    pub closed: Option<PresetSize>,
    pub open: Option<PresetSize>,
}

impl ColumnWidthPair {
    pub fn side(self, sidebar_open: bool) -> Option<PresetSize> {
        if sidebar_open {
            self.open
        } else {
            self.closed
        }
    }
}

impl Default for Sidebar {
    fn default() -> Self {
        Self {
            scope: SidebarScope::Output,
            mode: SidebarMode::Reserve,
            preset_widths: vec![
                PresetSize::Proportion(0.2),
                PresetSize::Proportion(0.3),
                PresetSize::Proportion(0.4),
            ],
            left: SidebarSideConfig::default(),
            right: SidebarSideConfig::default(),
            border: SidebarBorder::default(),
            corner_radius: None,
            use_dms_theme: false,
            hide_when_maximized: false,
            hide_when_fullscreen: false,
            shortcuts_sharing: ShortcutsSharing::default(),
            column_width_pairs: Vec::new(),
            default_column_width_when_closed: None,
            default_column_width_when_open: None,
            keep_proportions_on_sidebar_inset: false,
            max_column_proportion_when_closed: None,
            min_column_proportion_when_open: None,
        }
    }
}

/// Effective tiling default column width for the current reserve-sidebar open state.
pub fn sidebar_effective_default_column_width(
    sidebar: &Sidebar,
    layout_default: Option<PresetSize>,
    sidebar_open: bool,
) -> Option<PresetSize> {
    if let Some(pair) = sidebar.column_width_pairs.first() {
        return pair.side(sidebar_open);
    }

    if sidebar_open {
        sidebar
            .default_column_width_when_open
            .or(layout_default)
    } else {
        sidebar
            .default_column_width_when_closed
            .or(layout_default)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SidebarBorder {
    pub off: bool,
    pub width: f64,
    pub active_color: Color,
    pub inactive_color: Color,
    /// Optional fill of the panel strip behind sidebar windows.
    pub background_color: Option<Color>,
}

impl Default for SidebarBorder {
    fn default() -> Self {
        Self {
            off: true,
            width: 2.,
            active_color: Color::from_rgba8_unpremul(127, 200, 255, 255),
            inactive_color: Color::from_rgba8_unpremul(80, 80, 80, 255),
            background_color: None,
        }
    }
}

#[derive(knuffel::Decode, Debug, Default, Clone, Copy, PartialEq)]
pub struct SidebarBorderPart {
    #[knuffel(child)]
    pub off: bool,
    #[knuffel(child)]
    pub on: bool,
    #[knuffel(child, unwrap(argument))]
    pub width: Option<FloatOrInt<0, 65535>>,
    #[knuffel(child)]
    pub active_color: Option<Color>,
    #[knuffel(child)]
    pub inactive_color: Option<Color>,
    #[knuffel(child)]
    pub background_color: Option<Color>,
}

impl MergeWith<SidebarBorderPart> for SidebarBorder {
    fn merge_with(&mut self, part: &SidebarBorderPart) {
        self.off |= part.off;
        if part.on {
            self.off = false;
        }

        merge!((self, part), width);
        merge_clone!((self, part), active_color, inactive_color);
        merge_clone_opt!((self, part), background_color);
    }
}

#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq)]
pub struct SidebarPart {
    #[knuffel(child, unwrap(argument))]
    pub scope: Option<SidebarScope>,
    #[knuffel(child, unwrap(argument))]
    pub mode: Option<SidebarMode>,
    #[knuffel(child, unwrap(children))]
    pub preset_widths: Option<Vec<PresetSize>>,
    #[knuffel(child)]
    pub left: Option<SidebarSideConfigPart>,
    #[knuffel(child)]
    pub right: Option<SidebarSideConfigPart>,
    #[knuffel(child)]
    pub border: Option<SidebarBorderPart>,
    #[knuffel(child, unwrap(argument))]
    pub corner_radius: Option<FloatOrInt<0, 65535>>,
    #[knuffel(child)]
    pub use_dms_theme: Option<Flag>,
    #[knuffel(child)]
    pub hide_when_maximized: Option<Flag>,
    #[knuffel(child)]
    pub hide_when_fullscreen: Option<Flag>,
    #[knuffel(child, unwrap(argument))]
    pub shortcuts_sharing: Option<ShortcutsSharing>,
    #[knuffel(child)]
    pub column_width_pairs: Option<ColumnWidthPairsPart>,
    #[knuffel(child)]
    pub default_column_width_when_closed: Option<DefaultPresetSize>,
    #[knuffel(child)]
    pub default_column_width_when_open: Option<DefaultPresetSize>,
    #[knuffel(child)]
    pub keep_proportions_on_sidebar_inset: Option<Flag>,
    #[knuffel(child, unwrap(argument))]
    pub max_column_proportion_when_closed: Option<FloatOrInt<0, 1>>,
    #[knuffel(child, unwrap(argument))]
    pub min_column_proportion_when_open: Option<FloatOrInt<0, 1>>,
}

impl MergeWith<SidebarPart> for Sidebar {
    fn merge_with(&mut self, part: &SidebarPart) {
        merge_clone!((self, part), scope, mode, shortcuts_sharing);
        if let Some(widths) = &part.preset_widths {
            if !widths.is_empty() {
                self.preset_widths = widths.clone();
            }
        }
        if let Some(pairs) = &part.column_width_pairs {
            self.column_width_pairs = pairs.pairs.iter().map(ColumnWidthPair::from).collect();
        }
        if let Some(left) = &part.left {
            self.left.merge_with(left);
        }
        if let Some(right) = &part.right {
            self.right.merge_with(right);
        }
        if let Some(border) = &part.border {
            self.border.merge_with(border);
        }
        if let Some(radius) = part.corner_radius {
            self.corner_radius = Some(radius.0);
        }
        merge!(
            (self, part),
            use_dms_theme,
            hide_when_maximized,
            hide_when_fullscreen,
            keep_proportions_on_sidebar_inset
        );
        if let Some(x) = part.default_column_width_when_closed {
            self.default_column_width_when_closed = x.0;
        }
        if let Some(x) = part.default_column_width_when_open {
            self.default_column_width_when_open = x.0;
        }
        if let Some(x) = part.max_column_proportion_when_closed {
            self.max_column_proportion_when_closed = Some(x.0);
        }
        if let Some(x) = part.min_column_proportion_when_open {
            self.min_column_proportion_when_open = Some(x.0);
        }
        if self.preset_widths.is_empty() {
            self.preset_widths = Sidebar::default().preset_widths;
        }
    }
}

/// Knuffel form of `column-width-pairs { pair { ... }; ... }`.
#[derive(knuffel::Decode, Debug, Clone, PartialEq, Default)]
pub struct ColumnWidthPairsPart {
    #[knuffel(children(name = "pair"))]
    pub pairs: Vec<ColumnWidthPairPart>,
}

/// Knuffel form of [`ColumnWidthPair`] (`pair { closed { ... }; open { ... }; }`).
#[derive(knuffel::Decode, Debug, Clone, PartialEq)]
pub struct ColumnWidthPairPart {
    #[knuffel(child)]
    pub closed: Option<DefaultPresetSize>,
    #[knuffel(child)]
    pub open: Option<DefaultPresetSize>,
}

impl From<&ColumnWidthPairPart> for ColumnWidthPair {
    fn from(part: &ColumnWidthPairPart) -> Self {
        Self {
            closed: part.closed.and_then(|x| x.0),
            open: part.open.and_then(|x| x.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarShortcut {
    pub app_id: String,
    pub title: Option<String>,
    /// Spawn command argv. Empty means resolve from the desktop entry at runtime.
    pub spawn: Vec<String>,
}

#[derive(knuffel::Decode, Debug, Clone, PartialEq, Eq)]
pub struct SidebarShortcutPart {
    #[knuffel(property(name = "app-id"))]
    pub app_id: String,
    #[knuffel(property)]
    pub title: Option<String>,
    /// Spawn command: `spawn="app"` or first token of a shell-like string.
    #[knuffel(property)]
    pub spawn: Option<String>,
}

impl From<SidebarShortcutPart> for SidebarShortcut {
    fn from(part: SidebarShortcutPart) -> Self {
        let spawn = part.spawn.map(|s| vec![s]).unwrap_or_default();
        Self {
            app_id: part.app_id,
            title: part.title,
            spawn,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SidebarSideConfig {
    pub mode: Option<SidebarMode>,
    pub shortcuts: Vec<SidebarShortcut>,
}

#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq)]
pub struct SidebarSideConfigPart {
    #[knuffel(child, unwrap(argument))]
    pub mode: Option<SidebarMode>,
    #[knuffel(child)]
    pub shortcuts: Option<SidebarShortcutsPart>,
}

#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq)]
pub struct SidebarShortcutsPart {
    #[knuffel(children(name = "shortcut"))]
    pub shortcuts: Vec<SidebarShortcutPart>,
}

impl MergeWith<SidebarSideConfigPart> for SidebarSideConfig {
    fn merge_with(&mut self, part: &SidebarSideConfigPart) {
        if part.mode.is_some() {
            self.mode = part.mode;
        }
        if let Some(shortcuts) = &part.shortcuts {
            self.shortcuts = shortcuts
                .shortcuts
                .iter()
                .cloned()
                .map(SidebarShortcut::from)
                .collect();
        }
    }
}

#[derive(knuffel::DecodeScalar, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum SidebarScope {
    #[default]
    Output,
    Workspace,
}

#[derive(knuffel::DecodeScalar, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum SidebarMode {
    Overlay,
    #[default]
    Reserve,
}

/// How shortcut docks filter left/right pin lists.
#[derive(knuffel::DecodeScalar, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutsSharing {
    /// Each dock shows only that side's shortcuts.
    #[default]
    PerSide,
    /// Each dock shows the union of left and right (left first; dedupe by app-id).
    Shared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarSide {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SidebarEdgeDrop {
    pub trigger_width: f64,
    pub delay_ms: u16,
}

impl Default for SidebarEdgeDrop {
    fn default() -> Self {
        Self {
            trigger_width: 20.,
            delay_ms: 150,
        }
    }
}

#[derive(knuffel::Decode, Debug, Default, Clone, Copy, PartialEq)]
pub struct SidebarEdgeDropPart {
    #[knuffel(child, unwrap(argument))]
    pub trigger_width: Option<FloatOrInt<0, 65535>>,
    #[knuffel(child, unwrap(argument))]
    pub delay_ms: Option<u16>,
}

impl MergeWith<SidebarEdgeDropPart> for SidebarEdgeDrop {
    fn merge_with(&mut self, part: &SidebarEdgeDropPart) {
        merge!((self, part), trigger_width);
        merge_clone!((self, part), delay_ms);
    }
}
