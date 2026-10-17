//! Live DankMaterialShell theme reader for sidebar chrome.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use niri_config::Color;
use serde::Deserialize;

/// Resolved colors/radius from DMS for sidebar chrome.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DmsSidebarTheme {
    pub active_color: Color,
    pub inactive_color: Color,
    pub background_color: Color,
    pub corner_radius: f64,
}

impl Default for DmsSidebarTheme {
    fn default() -> Self {
        Self {
            // primary_container / outline_variant defaults (darker than primary/outline).
            active_color: Color::from_rgba8_unpremul(0x00, 0x51, 0x3e, 255),
            inactive_color: Color::from_rgba8_unpremul(0x3d, 0x49, 0x44, 255),
            background_color: Color::from_rgba8_unpremul(0x0a, 0x16, 0x11, 255),
            corner_radius: 12.,
        }
    }
}

#[derive(Debug)]
pub struct DmsThemeCache {
    colors_path: PathBuf,
    session_path: PathBuf,
    settings_path: PathBuf,
    last_mtimes: [Option<SystemTime>; 3],
    theme: Option<DmsSidebarTheme>,
    warned_missing: bool,
}

impl Default for DmsThemeCache {
    fn default() -> Self {
        Self::new()
    }
}

impl DmsThemeCache {
    pub fn new() -> Self {
        Self {
            colors_path: xdg_cache_path("DankMaterialShell/dms-colors.json"),
            session_path: xdg_state_path("DankMaterialShell/session.json"),
            settings_path: xdg_config_path("DankMaterialShell/settings.json"),
            last_mtimes: [None, None, None],
            theme: None,
            warned_missing: false,
        }
    }

    pub fn theme(&self) -> Option<DmsSidebarTheme> {
        self.theme
    }

    /// Re-read DMS files if mtimes changed. Returns `true` if the resolved theme changed.
    pub fn refresh(&mut self) -> bool {
        let mtimes = [
            mtime(&self.colors_path),
            mtime(&self.session_path),
            mtime(&self.settings_path),
        ];
        if mtimes == self.last_mtimes && self.theme.is_some() {
            return false;
        }
        self.last_mtimes = mtimes;

        match load_theme(&self.colors_path, &self.session_path, &self.settings_path) {
            Ok(theme) => {
                self.warned_missing = false;
                if self.theme != Some(theme) {
                    self.theme = Some(theme);
                    true
                } else {
                    false
                }
            }
            Err(err) => {
                if !self.warned_missing {
                    warn!("DMS theme unavailable for sidebar chrome: {err:#}");
                    self.warned_missing = true;
                }
                false
            }
        }
    }
}

fn xdg_cache_path(rel: &str) -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_CACHE_HOME") {
        return PathBuf::from(dir).join(rel);
    }
    home_join(".cache").join(rel)
}

fn xdg_state_path(rel: &str) -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_STATE_HOME") {
        return PathBuf::from(dir).join(rel);
    }
    home_join(".local/state").join(rel)
}

fn xdg_config_path(rel: &str) -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(dir).join(rel);
    }
    home_join(".config").join(rel)
}

fn home_join(rel: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(rel)
}

fn mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn load_theme(
    colors_path: &Path,
    session_path: &Path,
    settings_path: &Path,
) -> anyhow::Result<DmsSidebarTheme> {
    let colors_text = fs::read_to_string(colors_path)
        .map_err(|e| anyhow::anyhow!("{}: {e}", colors_path.display()))?;
    let colors: DmsColorsFile = serde_json::from_str(&colors_text)
        .map_err(|e| anyhow::anyhow!("parse {}: {e}", colors_path.display()))?;

    let is_light = fs::read_to_string(session_path)
        .ok()
        .and_then(|t| serde_json::from_str::<DmsSessionFile>(&t).ok())
        .map(|s| s.is_light_mode)
        .unwrap_or(false);

    let palette = if is_light {
        &colors.colors.light
    } else {
        &colors.colors.dark
    };

    let active = parse_color(
        palette
            .primary_container
            .as_deref()
            .unwrap_or(&palette.primary),
    )?;
    let inactive = parse_color(
        palette
            .outline_variant
            .as_deref()
            .unwrap_or(&palette.outline),
    )?;
    let background = parse_color(
        palette
            .surface_container
            .as_deref()
            .or(palette.surface.as_deref())
            .unwrap_or("#000000"),
    )?;

    let corner_radius = fs::read_to_string(settings_path)
        .ok()
        .and_then(|t| serde_json::from_str::<DmsSettingsFile>(&t).ok())
        .map(|s| {
            if s.niri_layout_radius_override >= 0. {
                s.niri_layout_radius_override
            } else {
                s.corner_radius.max(0.)
            }
        })
        .unwrap_or(12.);

    Ok(DmsSidebarTheme {
        active_color: active,
        inactive_color: inactive,
        background_color: background,
        corner_radius,
    })
}

fn parse_color(s: &str) -> anyhow::Result<Color> {
    s.parse()
        .map_err(|e| anyhow::anyhow!("invalid color {s:?}: {e}"))
}

#[derive(Debug, Deserialize)]
struct DmsColorsFile {
    colors: DmsColorsBranches,
}

#[derive(Debug, Deserialize)]
struct DmsColorsBranches {
    dark: DmsPalette,
    light: DmsPalette,
}

#[derive(Debug, Deserialize)]
struct DmsPalette {
    primary: String,
    outline: String,
    #[serde(default)]
    primary_container: Option<String>,
    #[serde(default)]
    outline_variant: Option<String>,
    #[serde(default)]
    surface: Option<String>,
    #[serde(default)]
    surface_container: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DmsSessionFile {
    #[serde(rename = "isLightMode", default)]
    is_light_mode: bool,
}

#[derive(Debug, Deserialize)]
struct DmsSettingsFile {
    #[serde(rename = "cornerRadius", default = "default_corner_radius")]
    corner_radius: f64,
    #[serde(rename = "niriLayoutRadiusOverride", default = "default_radius_override")]
    niri_layout_radius_override: f64,
}

fn default_corner_radius() -> f64 {
    12.
}

fn default_radius_override() -> f64 {
    -1.
}
