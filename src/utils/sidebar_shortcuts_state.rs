//! Persistent sidebar shortcut lists under XDG state.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use niri_config::{ShortcutsSharing, Sidebar, SidebarShortcut, SidebarSide};
use tracing::{debug, warn};

/// In-memory shortcut lists for left/right sides.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SidebarShortcutsStore {
    pub left: Vec<SidebarShortcut>,
    pub right: Vec<SidebarShortcut>,
    /// True once the user has added/removed via UI (state file is authoritative).
    pub has_state_file: bool,
    state_path: Option<PathBuf>,
}

impl SidebarShortcutsStore {
    pub fn load(config_sidebar: &Sidebar) -> Self {
        let state_path = state_file_path();
        if let Some(path) = &state_path {
            if path.is_file() {
                match load_from_path(path) {
                    Ok(mut store) => {
                        store.has_state_file = true;
                        store.state_path = state_path;
                        return store;
                    }
                    Err(err) => {
                        warn!("failed to load sidebar shortcuts state: {err:?}");
                    }
                }
            }
        }

        Self {
            left: config_sidebar.left.shortcuts.clone(),
            right: config_sidebar.right.shortcuts.clone(),
            has_state_file: false,
            state_path,
        }
    }

    /// Re-seed from config only when no state file is in use.
    pub fn apply_config_seed(&mut self, config_sidebar: &Sidebar) {
        if self.has_state_file {
            return;
        }
        self.left = config_sidebar.left.shortcuts.clone();
        self.right = config_sidebar.right.shortcuts.clone();
    }

    pub fn list(&self, side: SidebarSide) -> &[SidebarShortcut] {
        match side {
            SidebarSide::Left => &self.left,
            SidebarSide::Right => &self.right,
        }
    }

    pub fn list_mut(&mut self, side: SidebarSide) -> &mut Vec<SidebarShortcut> {
        match side {
            SidebarSide::Left => &mut self.left,
            SidebarSide::Right => &mut self.right,
        }
    }

    /// Shortcuts shown on a dock for the given sharing mode.
    ///
    /// Shared mode returns left then right, skipping duplicates by `app_id`.
    pub fn visible(&self, side: SidebarSide, sharing: ShortcutsSharing) -> Vec<SidebarShortcut> {
        match sharing {
            ShortcutsSharing::PerSide => self.list(side).to_vec(),
            ShortcutsSharing::Shared => {
                let mut out = Vec::with_capacity(self.left.len() + self.right.len());
                let mut seen = std::collections::HashSet::new();
                for s in self.left.iter().chain(self.right.iter()) {
                    if seen.insert(s.app_id.clone()) {
                        out.push(s.clone());
                    }
                }
                out
            }
        }
    }

    pub fn add(&mut self, side: SidebarSide, shortcut: SidebarShortcut) -> bool {
        let list = self.list_mut(side);
        if list.iter().any(|s| s.app_id == shortcut.app_id) {
            return false;
        }
        list.push(shortcut);
        self.persist();
        true
    }

    pub fn remove(&mut self, side: SidebarSide, index: usize) -> bool {
        let list = self.list_mut(side);
        if index >= list.len() {
            return false;
        }
        list.remove(index);
        self.persist();
        true
    }

    /// Remove the shortcut at `idx` in the visible list for `side` / `sharing`.
    pub fn remove_visible(
        &mut self,
        side: SidebarSide,
        sharing: ShortcutsSharing,
        idx: usize,
    ) -> bool {
        let visible = self.visible(side, sharing);
        let Some(target) = visible.get(idx) else {
            return false;
        };
        let app_id = target.app_id.clone();
        match sharing {
            ShortcutsSharing::PerSide => self.remove_by_app_id(side, &app_id),
            ShortcutsSharing::Shared => {
                // Prefer removing from the side that owns this entry in the
                // visible order (left wins on duplicate app-ids).
                if self.left.iter().any(|s| s.app_id == app_id) {
                    self.remove_by_app_id(SidebarSide::Left, &app_id)
                } else {
                    self.remove_by_app_id(SidebarSide::Right, &app_id)
                }
            }
        }
    }

    pub fn remove_by_app_id(&mut self, side: SidebarSide, app_id: &str) -> bool {
        let list = self.list_mut(side);
        let Some(idx) = list.iter().position(|s| s.app_id == app_id) else {
            return false;
        };
        list.remove(idx);
        self.persist();
        true
    }

    fn persist(&mut self) {
        let Some(path) = self.state_path.clone().or_else(state_file_path) else {
            warn!("cannot persist sidebar shortcuts: no state directory");
            return;
        };
        self.state_path = Some(path.clone());
        if let Some(parent) = path.parent() {
            if let Err(err) = fs::create_dir_all(parent) {
                warn!("failed to create state dir {}: {err}", parent.display());
                return;
            }
        }
        let body = serialize(&self.left, &self.right);
        match fs::File::create(&path).and_then(|mut f| f.write_all(body.as_bytes())) {
            Ok(()) => {
                self.has_state_file = true;
                debug!("wrote sidebar shortcuts state to {}", path.display());
            }
            Err(err) => warn!("failed to write sidebar shortcuts state: {err}"),
        }
    }
}

fn state_file_path() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("XDG_STATE_HOME") {
        return Some(PathBuf::from(dir).join("niri/sidebar-shortcuts.kdl"));
    }
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".local/state/niri/sidebar-shortcuts.kdl"))
}

fn serialize(left: &[SidebarShortcut], right: &[SidebarShortcut]) -> String {
    let mut out = String::from("// niri sidebar shortcuts state — edited by the compositor\n");
    out.push_str("left {\n");
    for s in left {
        out.push_str(&format_shortcut(s));
    }
    out.push_str("}\nright {\n");
    for s in right {
        out.push_str(&format_shortcut(s));
    }
    out.push_str("}\n");
    out
}

fn format_shortcut(s: &SidebarShortcut) -> String {
    let mut line = format!("    shortcut app-id=\"{}\"", escape_kdl(&s.app_id));
    if let Some(title) = &s.title {
        line.push_str(&format!(" title=\"{}\"", escape_kdl(title)));
    }
    if let Some(cmd) = s.spawn.first() {
        line.push_str(&format!(" spawn=\"{}\"", escape_kdl(cmd)));
    }
    line.push('\n');
    line
}

fn escape_kdl(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn load_from_path(path: &std::path::Path) -> anyhow::Result<SidebarShortcutsStore> {
    let text = fs::read_to_string(path)?;
    Ok(parse_state_text(&text))
}

fn parse_state_text(text: &str) -> SidebarShortcutsStore {
    let mut store = SidebarShortcutsStore::default();
    let mut side: Option<SidebarSide> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
            continue;
        }
        if line.starts_with("left") {
            side = Some(SidebarSide::Left);
            continue;
        }
        if line.starts_with("right") {
            side = Some(SidebarSide::Right);
            continue;
        }
        if line == "}" {
            side = None;
            continue;
        }
        if let Some(s) = line.strip_prefix("shortcut") {
            let Some(side) = side else { continue };
            if let Some(shortcut) = parse_shortcut_line(s) {
                store.list_mut(side).push(shortcut);
            }
        }
    }
    store
}

fn parse_shortcut_line(s: &str) -> Option<SidebarShortcut> {
    let mut app_id = None;
    let mut title = None;
    let mut spawn = None;
    let mut rest = s.trim();
    while !rest.is_empty() {
        rest = rest.trim_start();
        if rest.starts_with('{') || rest.starts_with('}') {
            break;
        }
        let (key, after_eq) = rest.split_once('=')?;
        let key = key.trim();
        let after_eq = after_eq.trim_start();
        if !after_eq.starts_with('"') {
            return None;
        }
        let (value, next) = parse_quoted(after_eq)?;
        match key {
            "app-id" => app_id = Some(value),
            "title" => title = Some(value),
            "spawn" => spawn = Some(value),
            _ => {}
        }
        rest = next;
    }
    Some(SidebarShortcut {
        app_id: app_id?,
        title,
        spawn: spawn.map(|s| vec![s]).unwrap_or_default(),
    })
}

fn parse_quoted(s: &str) -> Option<(String, &str)> {
    let s = s.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            '"' => {
                let rest = chars.as_str();
                return Some((out, rest));
            }
            c => out.push(c),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_parse() {
        let left = vec![SidebarShortcut {
            app_id: "spotify".into(),
            title: Some("Spotify".into()),
            spawn: vec!["spotify".into()],
        }];
        let text = serialize(&left, &[]);
        let store = parse_state_text(&text);
        assert_eq!(store.left, left);
        assert!(store.right.is_empty());
    }

    #[test]
    fn visible_shared_dedupes_left_first() {
        let store = SidebarShortcutsStore {
            left: vec![SidebarShortcut {
                app_id: "a".into(),
                title: None,
                spawn: vec![],
            }],
            right: vec![
                SidebarShortcut {
                    app_id: "a".into(),
                    title: Some("dup".into()),
                    spawn: vec![],
                },
                SidebarShortcut {
                    app_id: "b".into(),
                    title: None,
                    spawn: vec![],
                },
            ],
            has_state_file: false,
            state_path: None,
        };
        let vis = store.visible(SidebarSide::Right, ShortcutsSharing::Shared);
        assert_eq!(vis.len(), 2);
        assert_eq!(vis[0].app_id, "a");
        assert!(vis[0].title.is_none());
        assert_eq!(vis[1].app_id, "b");
        let per = store.visible(SidebarSide::Right, ShortcutsSharing::PerSide);
        assert_eq!(per.len(), 2);
        assert_eq!(per[0].title.as_deref(), Some("dup"));
    }
}
