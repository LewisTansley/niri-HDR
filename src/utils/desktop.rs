//! Freestktop desktop-entry and icon lookup for sidebar shortcuts.

use std::collections::HashSet;
use std::fs;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};

/// Parsed subset of a `.desktop` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopEntry {
    /// Desktop file id (filename stem without `.desktop`).
    pub app_id: String,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub exec: Vec<String>,
    pub path: PathBuf,
    listable: bool,
}

/// Locate and parse `{app_id}.desktop` (or `{app_id}`) under XDG application dirs.
pub fn find_desktop_entry(app_id: &str) -> Option<DesktopEntry> {
    for dir in application_dirs() {
        for name in [format!("{app_id}.desktop"), app_id.to_owned()] {
            let path = dir.join(&name);
            if path.is_file() {
                if let Ok(entry) = parse_desktop_file(&path) {
                    return Some(entry);
                }
            }
        }
    }
    None
}

/// List installed applications from XDG application dirs (deduped, sorted by name).
///
/// Earlier XDG dirs win on id conflicts. Skips `NoDisplay` / `Hidden` / non-Application entries.
pub fn list_desktop_entries() -> Vec<DesktopEntry> {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();

    for dir in application_dirs() {
        let Ok(read) = fs::read_dir(&dir) else {
            continue;
        };
        for item in read.flatten() {
            let path = item.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            let Ok(entry) = parse_desktop_file(&path) else {
                continue;
            };
            if !entry.is_listable_application() {
                continue;
            }
            if !seen.insert(entry.app_id.clone()) {
                continue;
            }
            entries.push(entry);
        }
    }

    entries.sort_by(|a, b| {
        let an = a.name.as_deref().unwrap_or(a.app_id.as_str());
        let bn = b.name.as_deref().unwrap_or(b.app_id.as_str());
        an.to_ascii_lowercase()
            .cmp(&bn.to_ascii_lowercase())
            .then_with(|| a.app_id.cmp(&b.app_id))
    });
    entries
}

/// Resolve an icon name or absolute path to a raster file (PNG preferred).
pub fn resolve_icon_path(icon: &str, size: u32) -> Option<PathBuf> {
    let path = Path::new(icon);
    if path.is_absolute() && path.is_file() {
        return Some(path.to_owned());
    }

    // Absolute path without extension, or relative path that exists.
    if path.extension().is_some() && path.is_file() {
        return Some(path.to_owned());
    }

    let sizes = [
        format!("{size}x{size}"),
        format!("{}x{}", size * 2, size * 2),
        "48x48".into(),
        "64x64".into(),
        "32x32".into(),
        "128x128".into(),
        "256x256".into(),
        "scalable".into(),
    ];

    for theme_dir in icon_theme_dirs() {
        for size_dir in &sizes {
            for category in ["apps", "applications", "legacy", "places", "status"] {
                for ext in ["png", "svg"] {
                    let candidate = theme_dir
                        .join(size_dir)
                        .join(category)
                        .join(format!("{icon}.{ext}"));
                    if candidate.is_file() {
                        // Prefer PNG; accept SVG only if no PNG found later — return PNG first.
                        if ext == "png" {
                            return Some(candidate);
                        }
                        // Keep looking for PNG in other themes; stash SVG as fallback.
                        if let Some(png) = find_png_icon(&theme_dir, icon, &sizes) {
                            return Some(png);
                        }
                        return Some(candidate);
                    }
                }
            }
        }
        // Flat pixmaps-style: theme/icon.png
        for ext in ["png", "svg"] {
            let candidate = theme_dir.join(format!("{icon}.{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    // /usr/share/pixmaps
    for base in pixmaps_dirs() {
        for ext in ["png", "xpm", "svg"] {
            let candidate = base.join(format!("{icon}.{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

/// Load a PNG icon into RGBA8 bytes and return (width, height, pixels).
pub fn load_icon_rgba(path: &Path) -> anyhow::Result<(u32, u32, Vec<u8>)> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext == "png" {
        return load_png_rgba(path);
    }
    // SVG / XPM: not decoded here; caller should fall back to a letter glyph.
    bail!("unsupported icon format: {ext}");
}

fn load_png_rgba(path: &Path) -> anyhow::Result<(u32, u32, Vec<u8>)> {
    let file = fs::File::open(path).context("open icon")?;
    let decoder = png::Decoder::new(BufReader::new(file));
    let mut reader = decoder.read_info().context("read png info")?;
    let mut buf = vec![0; reader.output_buffer_size().context("png buffer size")?];
    let info = reader.next_frame(&mut buf).context("decode png")?;
    let width = info.width;
    let height = info.height;
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => {
            let rgb = &buf[..info.buffer_size()];
            let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
            for chunk in rgb.chunks(3) {
                rgba.extend_from_slice(chunk);
                rgba.push(255);
            }
            rgba
        }
        png::ColorType::Grayscale => {
            let g = &buf[..info.buffer_size()];
            let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
            for &v in g {
                rgba.extend_from_slice(&[v, v, v, 255]);
            }
            rgba
        }
        png::ColorType::GrayscaleAlpha => {
            let ga = &buf[..info.buffer_size()];
            let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
            for chunk in ga.chunks(2) {
                let v = chunk[0];
                rgba.extend_from_slice(&[v, v, v, chunk[1]]);
            }
            rgba
        }
        other => bail!("unsupported png color type: {other:?}"),
    };
    Ok((width, height, rgba))
}

pub fn parse_desktop_file(path: &Path) -> anyhow::Result<DesktopEntry> {
    let text = fs::read_to_string(path).context("read desktop file")?;
    let mut in_desktop_entry = false;
    let mut name = None;
    let mut icon = None;
    let mut exec_raw = None;
    let mut entry_type = None;
    let mut no_display = false;
    let mut hidden = false;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_desktop_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_desktop_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        // Prefer unlocalized keys; skip localized Name[xx]=.
        match key {
            "Name" => name = Some(value.to_owned()),
            "Icon" => icon = Some(value.to_owned()),
            "Exec" => exec_raw = Some(value.to_owned()),
            "Type" => entry_type = Some(value.to_owned()),
            "NoDisplay" => no_display = parse_desktop_bool(value),
            "Hidden" => hidden = parse_desktop_bool(value),
            _ => {}
        }
    }

    let exec = exec_raw.as_deref().map(parse_exec).unwrap_or_default();
    let app_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_owned();

    let listable = entry_type.as_deref().unwrap_or("Application") == "Application"
        && !no_display
        && !hidden
        && !exec.is_empty();

    Ok(DesktopEntry {
        app_id,
        name,
        icon,
        exec,
        path: path.to_owned(),
        listable,
    })
}

fn parse_desktop_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "yes"
    )
}

impl DesktopEntry {
    /// Whether this entry should appear in an application browser.
    pub fn is_listable_application(&self) -> bool {
        self.listable
    }
}

/// Split an Exec= line and strip freestktop field codes (%f, %u, …).
pub fn parse_exec(exec: &str) -> Vec<String> {
    let mut args = shell_split(exec);
    args.retain(|a| {
        if a.starts_with('%') && a.len() == 2 {
            return false;
        }
        true
    });
    // Also strip codes glued to args like "%U".
    args
}

fn shell_split(s: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut chars = s.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    while let Some(c) = chars.next() {
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '\\' if !in_single => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            c if c.is_whitespace() && !in_single && !in_double => {
                if !cur.is_empty() {
                    args.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        args.push(cur);
    }
    args
}

fn find_png_icon(theme_dir: &Path, icon: &str, sizes: &[String]) -> Option<PathBuf> {
    for size_dir in sizes {
        for category in ["apps", "applications", "legacy"] {
            let candidate = theme_dir
                .join(size_dir)
                .join(category)
                .join(format!("{icon}.png"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn application_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share/applications"));
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        dirs.insert(0, PathBuf::from(xdg).join("applications"));
    }
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    for d in data_dirs.split(':').filter(|s| !s.is_empty()) {
        dirs.push(PathBuf::from(d).join("applications"));
    }
    dirs
}

fn icon_theme_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let themes = ["hicolor", "Adwaita", "breeze", "Papirus"];
    let mut bases = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        bases.push(PathBuf::from(&home).join(".local/share/icons"));
        bases.push(PathBuf::from(&home).join(".icons"));
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        bases.insert(0, PathBuf::from(xdg).join("icons"));
    }
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    for d in data_dirs.split(':').filter(|s| !s.is_empty()) {
        bases.push(PathBuf::from(d).join("icons"));
    }
    for base in bases {
        for theme in themes {
            dirs.push(base.join(theme));
        }
    }
    dirs
}

fn pixmaps_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/usr/share/pixmaps")];
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share/pixmaps"));
    }
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_exec_strips_field_codes() {
        assert_eq!(
            parse_exec("foo %u %F --bar"),
            vec!["foo".to_owned(), "--bar".to_owned()]
        );
    }

    #[test]
    fn shell_split_quotes() {
        assert_eq!(
            shell_split(r#"foo "bar baz" 'qux'"#),
            vec!["foo", "bar baz", "qux"]
        );
    }

    #[test]
    fn parse_skips_nodisplay() {
        let path = std::env::temp_dir().join("niri-test-hidden.desktop");
        fs::write(
            &path,
            "[Desktop Entry]\nType=Application\nName=Hidden\nExec=true\nNoDisplay=true\n",
        )
        .unwrap();
        let entry = parse_desktop_file(&path).unwrap();
        let _ = fs::remove_file(&path);
        assert!(!entry.is_listable_application());
        assert_eq!(entry.app_id, "niri-test-hidden");
    }
}
