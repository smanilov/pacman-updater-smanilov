# Installation

1. git clone this-repo ~/.local/share/cinnamon/applets/pacman-updater@smanilov
2. Build the update viewer: `cd pacman-update-viewer && cargo build --release`
3. Alt+F2, r to restart Cinnamon
4. add pacman-updater@smanilov to Applets

# Usage

1. this applet will run `checkupdates` (you need to install it separately)
   repeatedly and count the number of returned lines
2. if updates are available, a notification is shown as `N (M)` where N is the
   number of packages being updated and M is the total count of all transitively
   impacted installed packages (via `required_by` traversal)
3. from the taskbar, the user can click the smiley and run the updater; this
   opens a terminal running `pacman-update-viewer` — a TUI that lists pending
   updates sorted by impact factor as a collapsible tree
4. inside the viewer:
   - `↑`/`↓` navigate the list
   - `→` expands a node to show all installed packages that depend on it
   - `←` collapses an expanded node, or moves to the parent if already collapsed
   - `i` shows `pacman -Qi` output for the current item in a scrollable popup
   - `/` filters the visible root packages by name; `Esc` cancels and `Enter`
     keeps the selected match
   - `a` toggles between pending updates and all installed packages
   - `t` transposes the tree between `required_by` (`used-by`) and
     `depends_on` (`deps`) traversal
   - `g` toggles package group labels
   - `d` removes the currently selected package with `sudo pacman -R`, then
     rebuilds `depgraph.json`
   - `h` or `?` opens the help popup
   - `r` runs `sudo pacman -Syu` (suspends the TUI, then resumes it)
   - `q` quits; exits with code 0 only if the update succeeded
5. if there are no pending updates but `depgraph.json` exists, the viewer opens
   directly in all-packages mode instead of exiting
6. after the update completes successfully, `depgraph.json` is refreshed
   automatically; it is also refreshed on applet startup and rebuilt after
   package deletion from inside the viewer

# Data

depgraph.json is written by the applet and contains the dependency graph between
pacman managed packages. It is updated on startup and after each successful
`sudo pacman -Syu`. The viewer also rewrites it after package deletion. An
example follows:

```
{
  "last_updated": 1774870313,
  "packages": {
    "firefox": {
      "name": "firefox",
      "version": "123.0-1",
      "depends_on": ["nss", "gtk3"],
      "reason": "explicit",
      "required_by": [],
      "groups": []
    },
    "nss": {
      "name": "nss",
      "version": "3.98-1",
      "depends_on": ["nspr"],
      "reason": "dependency",
      "required_by": ["firefox"],
      "groups": []
    }
  }
}
```
