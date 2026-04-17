# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Development & Testing

The applet (`applet.js`) runs directly from source as a Cinnamon applet — no build step.

The update viewer (`pacman-update-viewer/`) is a Rust TUI app and must be compiled:
```bash
cd pacman-update-viewer && cargo build --release
```
The release binary is what `applet.js` invokes.

**To reload the applet after changes:**
```
Alt+F2, r  # restart Cinnamon (reloads all applets)
```

**To view logs:**
```bash
journalctl -f /usr/bin/cinnamon
```
Log messages are prefixed with `[pacman-updater@smanilov]`.

**Runtime dependencies:**
- `checkupdates` (from `pacman-contrib`) — used by both the applet and the viewer
- `cargo` / Rust toolchain — to build the viewer

## Architecture

The entire applet is a single class `PacmanUpdater` in `applet.js` that extends `Applet.IconApplet`. It is instantiated by Cinnamon via the `main()` entry point.

**Core lifecycle:**
1. `constructor` → builds popup menu, updates depgraph, starts the update loop
2. `on_applet_removed_from_panel` (destructor) → cleans up network watcher, loop, menu

**Update loop** (`LOOP MANAGEMENT` section): A `Mainloop.timeout_add_seconds` timer fires every 10 minutes, calling `checkUpdates()`. The loop can be toggled via the popup menu's switch item.

**Network awareness** (`NETWORK WATCHER` section): Before checking updates, `hasNetwork()` is called. If there's no network, `checkUpdates()` skips and enables a `Gio.NetworkMonitor` watcher. When full connectivity is restored, the watcher restarts the loop and removes itself.

**Update check** (`CHECK UPDATES` section): Runs `checkupdates` as a subprocess via `Gio.Subprocess`. Parses output lines to get the list of updatable packages, then calls `_allImpactedOf()` to compute the full transitive impact count via `required_by` traversal. Sends a `Main.notify()` desktop notification showing `N (M)` where N = packages being updated and M = all transitively impacted installed packages.

**Depgraph** (`DEPGRAPH` section): `updateDepgraph()` runs `pacman -Qi` asynchronously and writes `depgraph.json` via `_parsePacmanQi` + `_writeDepgraph`. It is called on construction and after a successful viewer run (detected via `gnome-terminal --wait` + `spawnCommandLineAsync` success callback). Version constraints are stripped from dep names during parsing.

**Impact count** (`_allImpactedOf`): follows `required_by` edges all the way to the top of the dependency graph (no early stop at explicit packages), collecting every transitively impacted package into a single visited set across all updated packages.

**State fields** (all private, updated via setters that also call `_updateTooltip()`):
- `_timeout` — Mainloop timer ID (non-null = loop running)
- `_networkWatcherId` — GIO signal handler ID (non-null = watcher active)
- `_depgraphPath` — absolute path to `depgraph.json` derived from `metadata.path`
- `_updateCount` — number of packages with pending updates
- `_topLevelCount` — total count of all transitively impacted installed packages
- `_loopEnabled`, `_checkingForUpdates`, `_hasNetwork`

## Update Viewer (`pacman-update-viewer/`)

A Rust TUI app (`src/main.rs`) that replaces the old raw `sudo pacman -Syu` terminal action.

**Flow:**
1. Shows a loading screen while fetching package data and checking for updates
2. Runs `checkupdates` to get the pending update list
3. Reads `depgraph.json` to compute both reverse impact (`required_by`) and forward impact (`depends_on`)
4. Starts in updates mode, or all-packages mode if there are no pending updates but a depgraph is available
5. Sorts root rows by the active impact direction and renders them as a collapsible tree
6. `r` suspends the TUI, runs `sudo pacman -Syu` in the foreground, then resumes the TUI; also runs `fc-cache -fv` on success; sets an internal `update_succeeded` flag on success
7. `u` suspends the TUI, runs `sudo pacman -S <package>`, rebuilds `depgraph.json`, reloads package state, re-sorts; sets `update_succeeded` on success
8. `d` suspends the TUI, runs `sudo pacman -R <package>`, rebuilds `depgraph.json`, reloads package state, and re-sorts
9. `q` exits with code 0 if `update_succeeded`, else code 1

The applet's `spawnCommandLineAsync` success callback (which triggers `updateDepgraph()`) fires only when the viewer exits with code 0 — i.e., only after a successful update or install.

**Modes and controls:**
- `a` toggles updates vs all-packages mode
- `t` toggles tree direction between `used-by` (`required_by`) and `deps` (`depends_on`)
- `/` enters search mode, filtering visible root rows by package name
- `g` toggles package group labels
- `h` / `?` opens the help overlay

**Tree structure:** each updateable package or installed package root is shown as a top-level node. Expanding a node (`→`) shows either the packages that `required_by` it or the packages it `depends_on`, depending on the current transpose state. Child rows are sorted alphabetically. Packages that are already an ancestor in the current path are shown with a `(↺)` suffix and cannot be expanded. Collapsing (`←`) collapses the current node; pressing `←` on a collapsed node moves the cursor to its parent.

**All-packages mode top-level:** `all_top_level()` returns different root sets depending on the transpose state. In used-by mode (non-transposed), it returns `all_leaves` — packages with no `depends_on` (nothing they need), which serve as tree roots when expanding upward via `required_by`. In deps mode (transposed), it returns `all_roots` — packages with no `required_by` (nothing depends on them), which serve as tree roots when expanding downward via `depends_on`.

**Info popup:** pressing `i` on any node runs `pacman -Qi <package>` and displays the output in a scrollable overlay (`↑`/`↓` to scroll, any other key to close).

**Viewer rebuild path:** the Rust binary currently resolves `depgraph.json` via `$HOME/.local/share/cinnamon/applets/pacman-updater@smanilov/depgraph.json`, not via the applet metadata path.

## Data

`depgraph.json` is written at runtime and is gitignored. It is produced by the applet on startup and after a successful update, and by the viewer after package installation or deletion. `example-depgraph.json` is the committed reference copy showing the schema (keyed by package name, with `name`, `version`, `reason`, `depends_on`, `required_by`, `groups` fields, and a top-level `last_updated` Unix timestamp).
