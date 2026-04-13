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

**Depgraph** (`DEPGRAPH` section): `updateDepgraph()` runs `pacman -Qi` asynchronously and writes `depgraph.json` via `_parsePacmanQi` + `_writeDepgraph`. It is called on construction and after a successful `sudo pacman -Syu` (detected via `gnome-terminal --wait` + `spawnCommandLineAsync` success callback). Version constraints are stripped from dep names during parsing.

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
1. Runs `checkupdates` to get the pending update list
2. Reads `depgraph.json` to compute an impact factor per package (DAG size via `required_by` edges)
3. Sorts updates by impact descending and renders them as a collapsible tree
4. `r` suspends the TUI, runs `sudo pacman -Syu` in the foreground, then resumes the TUI; sets an internal `update_succeeded` flag on exit code 0
5. `q` exits with code 0 if `update_succeeded`, else code 1

The applet's `spawnCommandLineAsync` success callback (which triggers `updateDepgraph()`) fires only when the viewer exits with code 0 — i.e., only after a successful update.

**Tree structure:** each updateable package is a root node. Expanding a node (`→`) shows all installed packages that `required_by` it (from the full depgraph), sorted alphabetically. Those nodes can be expanded further to show what requires them, and so on. Packages that are already an ancestor in the current path are shown with a `(↺)` suffix and cannot be expanded. Collapsing (`←`) collapses the current node; pressing `←` on a collapsed node moves the cursor to its parent.

**Info popup:** pressing `i` on any node runs `pacman -Qi <package>` and displays the output in a scrollable overlay (`↑`/`↓` to scroll, any other key to close).

## Data

`depgraph.json` is written by the applet at runtime and is gitignored. `example-depgraph.json` is the committed reference copy showing the schema (keyed by package name, with `name`, `reason`, `version`, `depends_on`, `required_by` fields, and a top-level `last_updated` Unix timestamp).
