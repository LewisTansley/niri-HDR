### Overview

Sidebars are vertical stacks of windows docked to the left or right of a monitor. You can use them for permanent companion windows (chat, music, terminals) while the scrolling tiling strip continues in the middle.

### Configuration

Under `layout { sidebar { ... } }`:

```kdl
layout {
    sidebar {
        // "output" = sticky across workspace switches (default);
        // stays fixed at full size during overview (does not zoom with cards).
        // "workspace" = each workspace has its own sidebars (zoom with cards)
        scope "output"

        // Both modes paint above the tiling strip.
        // "reserve" also shrinks the strip (default);
        // "overlay" draws above without shrinking.
        mode "reserve"

        preset-widths {
            proportion 0.2
            proportion 0.3
            proportion 0.4
        }

        // Linked tiling column widths for reserve sidebars open vs closed.
        // When set, these replace default-column-width-when-* and also replace
        // layout.preset-column-widths for Mod+R on the tiling strip.
        // column-width-pairs {
        //     pair {
        //         closed { proportion 0.5; }
        //         open { proportion 1.0; }
        //     }
        //     pair {
        //         closed { proportion 0.33333; }
        //         open { proportion 0.5; }
        //     }
        // }
        //
        // Or a single open/closed default (ignored while column-width-pairs is set):
        // default-column-width-when-closed { proportion 0.5; }
        // default-column-width-when-open { proportion 0.35; }
        //
        // When a reserve sidebar opens or closes, columns that match a pair's
        // (or the single default's) previous side are resized to the new side.
        //
        // While reserve insets change (including open/close animation), keep the
        // pixel width of other proportional columns instead of reflowing them to
        // the same fraction of the narrower strip:
        // keep-proportions-on-sidebar-inset
        //
        // Clamp proportional tiling columns when reserve insets change:
        // max-column-proportion-when-closed 0.5
        // min-column-proportion-when-open 0.5

        // Optional per-side mode override and shortcut seeds:
        left {
            // mode "overlay"
            shortcuts {
                shortcut app-id="org.gnome.Calendar" spawn="gnome-calendar"
                shortcut app-id="spotify" title="Spotify" spawn="spotify"
            }
        }
        // right { mode "reserve" }

        // Optional chrome around the panel strip (gaps between windows and screen edges).
        // border {
        //     on
        //     width 2
        //     active-color "#00e0b1"
        //     inactive-color "#86948d"
        //     // background-color "#0a161180"
        // }
        //
        // DMS-style goth wingtips: quarter-circle brackets that extend into the
        // viewport and frame the tiling strip (left sidebar: right corners;
        // right sidebar: left corners). 0 / omitted disables wingtips unless
        // use-dms-theme is set (then DMS cornerRadius is used when this is omitted).
        // corner-radius 32
        //
        // Pull border/fill colors (and default wingtip radius) from DankMaterialShell's
        // live theme files (~/.cache/DankMaterialShell/dms-colors.json).
        // use-dms-theme

        // Auto-hide the sidebars when the active column is maximize-to-edges
        // and/or true fullscreen (not full-width maximize-column).
        // hide-when-maximized
        // hide-when-fullscreen

        // Shortcut dock lists: "per-side" (default) shows only that side's pins;
        // "shared" shows the union of left and right on both docks.
        // shortcuts-sharing "shared"
    }
}
```

### Linked open/closed column widths

On ultrawides, proportional columns grow when a reserve sidebar closes. Use `column-width-pairs` to keep linked sizes for each sidebar state:

```kdl
layout {
    sidebar {
        column-width-pairs {
            // Full strip while the sidebar is open (narrower working area),
            // half strip while closed — roughly similar pixel width on ultrawides.
            pair {
                closed { proportion 0.5; }
                open { proportion 1.0; }
            }
            pair {
                closed { proportion 0.33333; }
                open { proportion 0.5; }
            }
        }
    }
}
```

Behavior when `column-width-pairs` is non-empty:

- New tiled windows use the **first** pair’s `open` or `closed` side for the current reserve-sidebar state (instead of `default-column-width` / `default-column-width-when-*`).
- `switch-preset-column-width` on the tiling strip cycles **pairs** in ascending size order of the current open/closed side (not declaration order), applying that side for the current open state (instead of `layout.preset-column-widths`). Sidebar-focused cycling still uses `preset-widths`.
- Opening or closing a reserve sidebar resizes every column that matches a pair’s previous side to that pair’s new side.
- `default-column-width-when-open` / `when-closed` are ignored while pairs are set.
- Maximize-column, maximize-to-edges, and fullscreen are not pair-swapped.

Without pairs, the single `default-column-width-when-open` / `when-closed` options still swap columns that match the previous default.

Independently of pairs/defaults, you can clamp **proportional** tiling columns whenever reserve insets change:

- `max-column-proportion-when-closed` — ceiling while reserve insets are ~0 (sidebar closed)
- `min-column-proportion-when-open` — floor while reserve insets are present (sidebar open)

Fixed-width, maximize-column, maximize-to-edges, and fullscreen columns are not clamped.

### Hide when maximized or fullscreen

With `hide-when-maximized` and/or `hide-when-fullscreen`, sidebars animate closed when the active workspace’s active column is [maximize-to-edges](./Fullscreen-and-Maximize.md#windows-maximized-to-edges) or [fullscreen](./Fullscreen-and-Maximize.md#fullscreen-windows). Leaving that state restores sidebars that were auto-hidden.

These flags do **not** apply to full-width `maximize-column`. Manual show (toggle, focus, or adding a window) while still maximized/fullscreen keeps the sidebar open until you hide it again or leave that state. The shortcuts dock still appears on the screen edge while the panel is hidden.

### Panel border and viewport wingtips

When `border { on }` is set, niri draws a frame around the sidebar panel strip (the reserved/overlay band that fills the gap between tiling windows and the screen edge). An optional `background-color` fills that strip behind the stacked windows.

`corner-radius` draws DMS-style **goth wingtips**: material that extends into the viewport by that radius and frames the workspace opening (not cutouts into the sidebar):

- left sidebar → top-right and bottom-right wingtips poking into the tiling area
- right sidebar → top-left and bottom-left wingtips

With `corner-radius 0` (or omitted without `use-dms-theme`), chrome stays flush with the panel and does not expand into the viewport.

### DankMaterialShell theming

With `use-dms-theme`, sidebar border/fill colors track DMS’s current palette:

| Role | DMS token |
|------|-----------|
| Active border | `primary_container` (falls back to `primary`) |
| Inactive border | `outline_variant` (falls back to `outline`) |
| Fill / shortcuts dock | `surface_container` (or `surface`), opaque |

Light vs dark is taken from `~/.local/state/DankMaterialShell/session.json` (`isLightMode`). If `corner-radius` is omitted, the wingtip radius falls back to DMS Settings `cornerRadius` (or `niriLayoutRadiusOverride` when that is ≥ 0). Theme files are polled for changes; a full config reload is not required.

For custom colors instead, omit `use-dms-theme` and set `border { active-color …; inactive-color …; background-color …; }`.

### Shortcuts dock

When a sidebar is hidden (or empty), hovering the far left/right edge reveals a small **shortcuts dock**: app icons plus a trailing **+** button.

- **Click an icon**: focuses a matching window in that sidebar, or spawns the app and opens it into the sidebar.
- **Click +**: opens an application browser (installed `.desktop` apps). Pick one to pin it; scroll with the mouse wheel; Escape or click outside closes the list.
- **Right-click an icon**: removes that shortcut.
- Hover reveal uses `gestures.sidebar-edge-drop` `trigger-width` and `delay-ms`.
- Open/close of the dock (and the app picker) uses the same `sidebar-open-close` animation as the sidebar panel.

Config `shortcuts { }` under `left` / `right` seeds the lists. Storage stays per-side; `shortcuts-sharing` only changes what each dock displays:

- `per-side` (default): left dock shows left pins, right dock shows right pins
- `shared`: both docks show left then right (duplicates by `app-id` appear once)

Pinning (including **+** and keybinds) still writes to the side where you acted. Removing a shared entry deletes it from the side that owns that `app-id`.

After you add or remove via the dock UI, niri writes `$XDG_STATE_HOME/niri/sidebar-shortcuts.kdl` (usually `~/.local/state/niri/sidebar-shortcuts.kdl`) and that file becomes authoritative until deleted.

Drag-to-edge drop zones (separate from DnD view scrolling):

```kdl
gestures {
    sidebar-edge-drop {
        trigger-width 20
        // delay-ms 150
    }
}
```

While Mod+dragging a window, release anywhere over an **open sidebar panel** to keep or add that window to the sidebar, or drag onto the workspace area to pull it back into tiling. When a sidebar is closed or empty, only the thin `trigger-width` strips at the far left/right dock into that sidebar (same insert-hint overlay as column drops).

Changing `scope` on config reload migrates existing sidebar windows between the monitor and the workspaces (workspace→output appends from every workspace so nothing is stranded).

### Actions

| Action | Behavior |
|--------|----------|
| `move-window-to-sidebar-left` / `right` | Move the focused window into that sidebar (creates it if needed). Pressing the **same** side again toggles it back out; the opposite side moves it across. |
| `move-window-out-of-sidebar` | Explicitly move the focused sidebar window back into the tiling layout. |
| `toggle-sidebar-left` / `right` | Hide or show that sidebar (windows stay parked). |
| `focus-sidebar-left` / `right` | Focus that sidebar; `focus-window-up` / `down` move within the stack; `move-window-up` / `down` reorder within the stack. |
| `set-sidebar-width` | Change width (`set-sidebar-width "30%"` / `"+10"` etc.). |
| `switch-preset-column-width` | When a sidebar is focused, cycles **sidebar** width presets. On the tiling strip, cycles `column-width-pairs` when configured, otherwise `layout.preset-column-widths`. |
| `toggle-sidebar-shortcuts-left` / `right` | Force show/hide the shortcuts dock without hovering. |
| `pin-window-to-sidebar-shortcuts-left` / `right` | Pin the focused window as a shortcut (does not open the app browser). |

`focus-column-left` / `focus-column-right` jump into the corresponding **visible** sidebar when there is no further column (or floating window) in that direction. From a focused sidebar, the inward direction returns focus to tiling.

`focus-tiling` / `focus-floating` clear sidebar focus.

Mod+drag also works on sidebar windows: drag onto the workspace and release to drop into tiling. Releasing over the full open panel keeps the window in the sidebar (including after dragging the last window out — the former panel region stays a drop target until release). If you leave via the workspace and return only to the **edge strip** without having left the origin latch once, release there does not instantly re-dock to the sidebar you came from.

Example binds:

```kdl
binds {
    Mod+BracketLeft { move-window-to-sidebar-left; }
    Mod+BracketRight { move-window-to-sidebar-right; }
    Mod+Shift+BracketLeft { toggle-sidebar-left; }
    Mod+Shift+BracketRight { toggle-sidebar-right; }
    Mod+Ctrl+BracketLeft { focus-sidebar-left; }
    Mod+Ctrl+BracketRight { focus-sidebar-right; }
}
```

### Resize

- Mod+RMB on the inner edge of a visible sidebar adjusts its width continuously.
- With the sidebar focused, `switch-preset-column-width` / `switch-preset-column-width-back` cycle `layout.sidebar.preset-widths`.

### Animations

Show/hide (`toggle-sidebar-*`) and the shortcuts dock peek use the `sidebar-open-close` animation (see [Configuration: Animations](Configuration:-Animations.md)). Discrete width changes (presets / `set-sidebar-width`) animate sidebar tiles with `window-resize`; the tiling strip tracks the changing inset without a resize animation so sizes stay in lockstep. Interactive edge resize stays immediate. Window open/close and stack reflow reuse `window-open`, `window-close`, and `window-movement`.
