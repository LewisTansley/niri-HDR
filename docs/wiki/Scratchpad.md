### Overview

The scratchpad lets you park windows out of the way and bring the most recently scratched one back as a floating overlay on the active workspace, similar to i3.

Each monitor has its own scratchpad. Parked windows stay mapped; only one scratched window is shown at a time.

### Actions

| Action | Behavior |
|--------|----------|
| `move-window-to-scratchpad` | Move the focused window onto this monitor's scratchpad (becomes the most recent). |
| `toggle-scratchpad` | Show or hide the most recently scratched window as an overlay. |
| `cycle-scratchpad` / `cycle-scratchpad-back` | Rotate which parked window is current. If one was shown, the new current window is shown instead. |

Example binds:

```kdl
binds {
    Mod+Shift+Minus { move-window-to-scratchpad; }
    Mod+Minus { toggle-scratchpad; }
    Mod+Equal { cycle-scratchpad; }
}
```

You can also drive these via IPC, for example `niri msg action toggle-scratchpad`.
