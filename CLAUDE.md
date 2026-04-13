# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Development & Testing

There is no build step. The applet runs directly from source as a Cinnamon applet.

**To reload the applet after changes:**
```
Alt+F2, r  # restart Cinnamon (reloads all applets)
```

**To view logs:**
```bash
journalctl -f /usr/bin/cinnamon
```
Log messages are prefixed with `[pacman-updater@smanilov]`.

**Runtime dependency:** `checkupdates` must be installed (from `pacman-contrib`).

## Architecture

The entire applet is a single class `PacmanUpdater` in `applet.js` that extends `Applet.IconApplet`. It is instantiated by Cinnamon via the `main()` entry point.

**Core lifecycle:**
1. `constructor` → builds popup menu, updates depgraph, starts the update loop
2. `on_applet_removed_from_panel` (destructor) → cleans up network watcher, loop, menu

**Update loop** (`LOOP MANAGEMENT` section): A `Mainloop.timeout_add_seconds` timer fires every 10 minutes, calling `checkUpdates()`. The loop can be toggled via the popup menu's switch item.

**Network awareness** (`NETWORK WATCHER` section): Before checking updates, `hasNetwork()` is called. If there's no network, `checkUpdates()` skips and enables a `Gio.NetworkMonitor` watcher. When full connectivity is restored, the watcher restarts the loop and removes itself.

**Update check** (`CHECK UPDATES` section): Runs `checkupdates` as a subprocess via `Gio.Subprocess`. Counts output lines to determine pending update count. Sends a `Main.notify()` desktop notification if `count > 0`, including the full list for ≤10 updates.

**Depgraph** (`DEPGRAPH` section): `updateDepgraph()` runs `pacman -Qi` asynchronously and writes `depgraph.json` via `_parsePacmanQi` + `_writeDepgraph`. It is called on construction and after a successful `sudo pacman -Syu` (detected via `gnome-terminal --wait` + `spawnCommandLineAsync` success callback). Version constraints are stripped from dep names during parsing.

**State fields** (all private, updated via setters that also call `_updateTooltip()`):
- `_timeout` — Mainloop timer ID (non-null = loop running)
- `_networkWatcherId` — GIO signal handler ID (non-null = watcher active)
- `_depgraphPath` — absolute path to `depgraph.json` derived from `metadata.path`
- `_loopEnabled`, `_updateCount`, `_checkingForUpdates`, `_hasNetwork`

## Data

`depgraph.json` is written by the applet at runtime and is gitignored. `example-depgraph.json` is the committed reference copy showing the schema (keyed by package name, with `name`, `reason`, `version`, `depends_on`, `required_by` fields, and a top-level `last_updated` Unix timestamp).
