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
1. `constructor` → loads `package-managers.json`, builds popup menu, updates depgraph, starts the update loop
2. `on_applet_removed_from_panel` (destructor) → cleans up network watcher, loop, menu

**Package managers config** (`PACKAGE MANAGERS` section): `loadPackageManagers()` reads `package-managers.json` (next to `applet.js`, gitignored; schema documented in `package-managers.md`) and extends each entry with runtime state (`_timeout`, `_updateCount`, `_impactedCount`, `_checking`, `_error`). If the file is missing or invalid, no checks run and a usage message in the tooltip (and journal) tells the user to copy the committed `example-package-managers.json` (pacman-only) to `package-managers.json`. Currently only pacman is configured; `update_cmd`/`update_needs_terminal` are carried in the config but not yet used (pacman updates go through the viewer).

**Update loop** (`LOOP MANAGEMENT` section): One `Mainloop.timeout_add_seconds` timer per package manager, firing every `check_interval_minutes` (default 10), calling `checkUpdates(manager)`. `self_managed` entries get no timer. All loops are toggled together via the popup menu's switch item.

**Network awareness** (`NETWORK WATCHER` section): Before checking updates, `hasNetwork()` is called. If there's no network, `checkUpdates()` skips and enables a `Gio.NetworkMonitor` watcher. When full connectivity is restored, the watcher restarts the loop and removes itself.

**Update check** (`CHECK UPDATES` section): `checkUpdates(manager)` runs the manager's `check_cmd` as a subprocess via `Gio.Subprocess`. Exit code handling: 127 → command not installed; `check_no_updates_exit_code` → zero updates; other non-zero or stderr → per-manager error shown in the tooltip. Output is counted per `check_parse` (`"lines"` or `{ count_regex }`). For depgraph-using managers (`provides_depgraph` — pacman, which sources `depgraph.json` — or `uses_depgraph` — yay, whose AUR packages are installed via pacman and thus present in the depgraph), package names from the lines feed `_allImpactedOf()` to compute the transitive impact count via `required_by` traversal. When a check finds updates, `Main.notify()` shows the per-manager breakdown, e.g. `updates available: pacman: 5 (12)` where 5 = packages being updated and 12 = all transitively impacted installed packages.

**Depgraph** (`DEPGRAPH` section): `updateDepgraph()` runs `pacman -Qi` asynchronously and writes `depgraph.json` via `_parsePacmanQi` + `_writeDepgraph`. It is called on construction and after a successful viewer run (detected via `gnome-terminal --wait` + `spawnCommandLineAsync` success callback). Version constraints are stripped from dep names during parsing.

**Impact count** (`_allImpactedOf`): follows `required_by` edges all the way to the top of the dependency graph (no early stop at explicit packages), collecting every transitively impacted package into a single visited set across all updated packages.

**State fields** (all private, updated via setters that also call `_updateTooltip()`):
- `_managers` — package manager entries from `package-managers.json`, each with per-manager runtime state: `_timeout` (Mainloop timer ID; non-null = loop running), `_updateCount`, `_impactedCount`, `_checking`, `_error`
- `_networkWatcherId` — GIO signal handler ID (non-null = watcher active)
- `_appletPath` — `metadata.path`; `_depgraphPath` — absolute path to `depgraph.json`
- `_loopEnabled`, `_hasNetwork`

## Update Viewer (`pacman-update-viewer/`)

A Rust TUI app (`src/main.rs`) that replaces the old raw `sudo pacman -Syu` terminal action.

**Flow:**
1. Shows a loading screen while fetching package data and checking for updates
2. Runs `checkupdates` (repo updates) and `yay -Qua` (AUR updates) to get the pending update lists; yay being absent just yields an empty AUR list
3. Reads `depgraph.json` to compute both reverse impact (`required_by`) and forward impact (`depends_on`)
4. Starts in updates mode; if no repo updates, AUR-updates mode; if neither, all-packages mode (given a depgraph)
5. Sorts root rows by the active impact direction and renders them as a collapsible tree
6. `r` suspends the TUI, runs `sudo pacman -Syu` (or `yay -Syu` in AUR mode — yay elevates itself, no sudo) in the foreground, then resumes the TUI; also runs `fc-cache -fv` on success; sets an internal `update_succeeded` flag on success
7. `u` suspends the TUI, runs `sudo pacman -S <package>` (or `yay -S <package>` in AUR mode), rebuilds `depgraph.json`, reloads package state, re-sorts; sets `update_succeeded` on success
8. `d` suspends the TUI, runs `sudo pacman -R <package>`, rebuilds `depgraph.json`, reloads package state, and re-sorts
9. `q` exits with code 0 if `update_succeeded`, else code 1

The applet's `spawnCommandLineAsync` success callback (which triggers `updateDepgraph()`) fires only when the viewer exits with code 0 — i.e., only after a successful update or install.

**Modes and controls:**
- `a` cycles updates → AUR updates (yay) → all-packages mode; AUR updates use the same depgraph impact trees since installed AUR packages are in the pacman database
- `t` toggles tree direction between `used-by` (`required_by`) and `deps` (`depends_on`)
- `/` enters search mode, filtering visible root rows by package name
- `p` exports the current mode's update graph: writes `/tmp/pacman-updates.dot` (update roots plus everything transitively depending on them, roots highlighted), renders `/tmp/pacman-updates.png` via graphviz `dot`, and opens it with `xdg-open`; errors show in the info popup
- `g` toggles package group labels
- `h` / `?` opens the help overlay

**Tree structure:** each updateable package or installed package root is shown as a top-level node. Expanding a node (`→`) shows either the packages that `required_by` it or the packages it `depends_on`, depending on the current transpose state. Child rows are sorted alphabetically. Packages that are already an ancestor in the current path are shown with a `(↺)` suffix and cannot be expanded. Collapsing (`←`) collapses the current node; pressing `←` on a collapsed node moves the cursor to its parent.

**All-packages mode top-level:** `all_top_level()` returns different root sets depending on the transpose state. In used-by mode (non-transposed), it returns `all_leaves` — packages with no `depends_on` (nothing they need), which serve as tree roots when expanding upward via `required_by`. In deps mode (transposed), it returns `all_roots` — packages with no `required_by` (nothing depends on them), which serve as tree roots when expanding downward via `depends_on`.

**Info popup:** pressing `i` on any node runs `pacman -Qi <package>` and displays the output in a scrollable overlay (`↑`/`↓` to scroll, any other key to close).

**Viewer rebuild path:** the Rust binary currently resolves `depgraph.json` via `$HOME/.local/share/cinnamon/applets/pacman-updater@smanilov/depgraph.json`, not via the applet metadata path.

## Data

`depgraph.json` is written at runtime and is gitignored. It is produced by the applet on startup and after a successful update, and by the viewer after package installation or deletion. `example-depgraph.json` is the committed reference copy showing the schema (keyed by package name, with `name`, `version`, `reason`, `depends_on`, `required_by`, `groups` fields, and a top-level `last_updated` Unix timestamp).
