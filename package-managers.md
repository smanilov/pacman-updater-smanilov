# package-managers.json

Config file describing the package managers the applet tracks. Lives next to
`applet.js`; read on startup. `depgraph.json` is sourced from pacman only, but
any manager whose packages live in the pacman database (e.g. yay for AUR) can
opt into impact counts via `uses_depgraph`.

## Schema

```json
{
  "package_managers": [
    {
      "name": "pacman",
      "check_cmd": "checkupdates",
      "check_no_updates_exit_code": 2,
      "check_parse": "lines",
      "update_cmd": "sudo pacman -Syu",
      "update_needs_terminal": true,
      "check_interval_minutes": 10,
      "provides_depgraph": true
    },
    {
      "name": "aur (yay)",
      "check_cmd": "yay -Qua",
      "check_no_updates_exit_code": 1,
      "check_parse": "lines",
      "update_cmd": "yay -Syu",
      "update_needs_terminal": true,
      "check_interval_minutes": 10,
      "uses_depgraph": true
    },
    {
      "name": "rustup",
      "check_cmd": "rustup check",
      "check_parse": { "count_regex": "Update available" },
      "update_cmd": "rustup self update && rustup update",
      "update_needs_terminal": false,
      "check_interval_minutes": 720
    },
    {
      "name": "claude",
      "self_managed": true
    }
  ]
}
```

## Fields

| Field | Meaning |
|---|---|
| `name` | Display name, used in tooltip/notification breakdown (`pacman: 5, aur: 2`). |
| `check_cmd` | Read-only command listing pending updates. Must be safe to run unattended: no sudo, no side effects. |
| `check_no_updates_exit_code` | Exit code meaning "no updates" (vs. a real error). `checkupdates` exits 2 when nothing is pending; `yay -Qua` exits 1 (like `pacman -Qu`). Default: exit 0 with empty output. |
| `check_parse` | How to count updates from output: `"lines"` = one non-empty line per update; `{ "count_regex": "..." }` = count matching lines (e.g. `rustup check` also prints up-to-date components). |
| `update_cmd` | Command to apply updates. Absent for self-managed entries. |
| `update_needs_terminal` | `true` → open interactive terminal (sudo/confirm prompts); `false` → run headless, notify on completion. |
| `check_interval_minutes` | Per-manager check cadence. Keeps rustup from being polled every 10 minutes. |
| `provides_depgraph` | This manager feeds `depgraph.json` (and implicitly gets impact counts). Only pacman. |
| `uses_depgraph` | This manager's updated packages are installed via pacman, so they appear in `depgraph.json`; show impact counts for them. Used by yay/AUR. |
| `self_managed` | Tool updates itself (e.g. claude). No commands run; shown in UI so the user knows it's accounted for. |

## Notes

- **yay/pacman overlap:** `yay -Syu` updates repo *and* AUR packages, and
  `checkupdates` already covers the repo side. So yay's check is AUR-only
  (`yay -Qua`) to avoid double counting; its superset update command is harmless.
- **AUR impact counts come for free:** installed AUR packages appear in
  `pacman -Qi`, so they are already in the depgraph — hence `uses_depgraph`
  on the yay entry.
- **Applet badge:** the applet runs every manager's check on its own interval
  and shows the summed count; the tooltip and notification show the
  per-manager breakdown.
