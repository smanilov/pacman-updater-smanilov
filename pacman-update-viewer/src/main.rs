use std::collections::{HashMap, HashSet};
use std::io;
use std::process::{self, Command};

use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Terminal,
};
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Depgraph types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct Package {
    #[serde(default)]
    version: String,
    #[serde(default)]
    required_by: Vec<String>,
    #[serde(default)]
    depends_on: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Depgraph {
    packages: HashMap<String, Package>,
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Updates,
    AllPackages,
}

struct UpdateInfo {
    name: String,
    old_version: String,
    new_version: String,
    impact: usize,
}

struct AllPkgInfo {
    name: String,
    version: String,
    impact: usize,
}

/// One row in the visible flat list.
struct VisibleItem {
    /// Actual package name, used as the expand/collapse key.
    name: String,
    depth: usize,
    kind: ItemKind,
    has_children: bool,
    is_expanded: bool,
    /// True when the package is already an ancestor in the current path (cycle guard).
    is_cycle: bool,
}

enum ItemKind {
    Root { old_version: String, new_version: String, impact: usize },
    AllPkgRoot { version: String, impact: usize },
    Dep { installed_version: Option<String>, impact: usize },
}

struct InfoPopup {
    package: String,
    content: String,
    scroll: u16,
}

struct SearchState {
    query: String,
    pre_search_cursor: usize,
}

struct AppState {
    roots: Vec<UpdateInfo>,
    all_roots: Vec<AllPkgInfo>,
    packages: HashMap<String, Package>,
    /// Pre-computed reverse-impact (dag_size via required_by) for every package.
    impacts: HashMap<String, usize>,
    /// Pre-computed forward-impact (closure size via depends_on) for every package.
    dep_impacts: HashMap<String, usize>,
    expanded: HashSet<String>,
    cursor: usize,
    mode: Mode,
    transposed: bool,
    update_succeeded: bool,
    info_popup: Option<InfoPopup>,
    search: Option<SearchState>,
    show_help: bool,
}

impl AppState {
    fn visible(&self) -> Vec<VisibleItem> {
        let query = self.search.as_ref().map(|s| s.query.to_lowercase());
        let matches = |name: &str| -> bool {
            query.as_ref().map_or(true, |q| name.to_lowercase().contains(q.as_str()))
        };
        let mut out = vec![];
        match self.mode {
            Mode::Updates => {
                for root in &self.roots {
                    if !matches(&root.name) { continue; }
                    let children = self.children_of(&root.name);
                    let is_expanded = self.expanded.contains(&root.name);
                    out.push(VisibleItem {
                        name: root.name.clone(),
                        depth: 0,
                        kind: ItemKind::Root {
                            old_version: root.old_version.clone(),
                            new_version: root.new_version.clone(),
                            impact: self.impact_of(&root.name),
                        },
                        has_children: !children.is_empty(),
                        is_expanded,
                        is_cycle: false,
                    });
                    if is_expanded {
                        let mut ancestors = HashSet::from([root.name.clone()]);
                        self.collect_children(children, 1, &mut ancestors, &mut out);
                    }
                }
            }
            Mode::AllPackages => {
                for pkg in &self.all_roots {
                    if !matches(&pkg.name) { continue; }
                    let children = self.children_of(&pkg.name);
                    let is_expanded = self.expanded.contains(&pkg.name);
                    out.push(VisibleItem {
                        name: pkg.name.clone(),
                        depth: 0,
                        kind: ItemKind::AllPkgRoot {
                            version: pkg.version.clone(),
                            impact: self.impact_of(&pkg.name),
                        },
                        has_children: !children.is_empty(),
                        is_expanded,
                        is_cycle: false,
                    });
                    if is_expanded {
                        let mut ancestors = HashSet::from([pkg.name.clone()]);
                        self.collect_children(children, 1, &mut ancestors, &mut out);
                    }
                }
            }
        }
        out
    }

    fn req_by(&self, name: &str) -> Vec<String> {
        self.packages.get(name).map(|p| { let mut v = p.required_by.clone(); v.sort(); v }).unwrap_or_default()
    }

    fn deps_of(&self, name: &str) -> Vec<String> {
        self.packages.get(name).map(|p| { let mut v = p.depends_on.clone(); v.sort(); v }).unwrap_or_default()
    }

    fn children_of(&self, name: &str) -> Vec<String> {
        if self.transposed { self.deps_of(name) } else { self.req_by(name) }
    }

    fn impact_of(&self, name: &str) -> usize {
        if self.transposed {
            *self.dep_impacts.get(name).unwrap_or(&1)
        } else {
            *self.impacts.get(name).unwrap_or(&1)
        }
    }

    fn collect_children(
        &self,
        names: Vec<String>,
        depth: usize,
        ancestors: &mut HashSet<String>,
        out: &mut Vec<VisibleItem>,
    ) {
        for name in names {
            let is_cycle = ancestors.contains(&name);
            let children = if is_cycle { vec![] } else { self.children_of(&name) };
            let has_children = !children.is_empty();
            let is_expanded = !is_cycle && self.expanded.contains(&name);
            let installed_version = self.packages.get(&name).map(|p| p.version.clone());
            let impact = self.impact_of(&name);
            out.push(VisibleItem {
                name: name.clone(),
                depth,
                kind: ItemKind::Dep { installed_version, impact },
                has_children,
                is_expanded,
                is_cycle,
            });
            if is_expanded {
                ancestors.insert(name.clone());
                self.collect_children(children, depth + 1, ancestors, out);
                ancestors.remove(&name);
            }
        }
    }

    fn clamp_cursor(&mut self) {
        let len = self.visible().len();
        if self.cursor >= len {
            self.cursor = len.saturating_sub(1);
        }
    }

    fn move_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    fn move_down(&mut self) {
        if self.cursor + 1 < self.visible().len() {
            self.cursor += 1;
        }
    }

    fn expand_current(&mut self) {
        let vis = self.visible();
        let item = &vis[self.cursor];
        if item.has_children && !item.is_cycle {
            self.expanded.insert(item.name.clone());
        }
    }

    fn collapse_or_go_to_parent(&mut self) {
        let vis = self.visible();
        let item = &vis[self.cursor];
        let name = item.name.clone();
        let depth = item.depth;
        let is_expanded = item.is_expanded;
        let _ = item;

        if is_expanded {
            self.expanded.remove(&name);
            self.clamp_cursor();
        } else if depth > 0 {
            if let Some(pos) = vis[..self.cursor].iter().rposition(|i| i.depth == depth - 1) {
                self.cursor = pos;
            }
        }
    }

    fn reload_packages(&mut self, packages: HashMap<String, Package>) {
        self.impacts = packages.keys()
            .map(|name| (name.clone(), dag_size(name, &packages)))
            .collect();
        self.dep_impacts = packages.keys()
            .map(|name| (name.clone(), dep_dag_size(name, &packages)))
            .collect();
        self.all_roots = packages.iter()
            .map(|(name, pkg)| AllPkgInfo {
                name: name.clone(),
                version: pkg.version.clone(),
                impact: *self.impacts.get(name).unwrap_or(&1),
            })
            .collect();
        self.roots.retain(|r| packages.contains_key(&r.name));
        for root in &mut self.roots {
            root.impact = *self.impacts.get(&root.name).unwrap_or(&1);
        }
        self.expanded.retain(|name| packages.contains_key(name));
        self.packages = packages;
        self.resort();
        self.clamp_cursor();
    }

    fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            Mode::Updates => Mode::AllPackages,
            Mode::AllPackages => Mode::Updates,
        };
        self.cursor = 0;
    }

    fn toggle_transposed(&mut self) {
        self.transposed = !self.transposed;
        self.resort();
        self.expanded.clear();
        self.cursor = 0;
    }

    fn resort(&mut self) {
        let impact_of = |name: &str| -> usize {
            if self.transposed {
                *self.dep_impacts.get(name).unwrap_or(&1)
            } else {
                *self.impacts.get(name).unwrap_or(&1)
            }
        };
        self.all_roots.sort_by(|a, b| impact_of(&b.name).cmp(&impact_of(&a.name)).then(a.name.cmp(&b.name)));
        self.roots.sort_by(|a, b| impact_of(&b.name).cmp(&impact_of(&a.name)));
    }
}

// ---------------------------------------------------------------------------
// Impact computation
// ---------------------------------------------------------------------------

fn dag_size(name: &str, packages: &HashMap<String, Package>) -> usize {
    let mut visited = HashSet::new();
    visit_dag(name, packages, &mut visited);
    visited.len()
}

fn visit_dag(name: &str, packages: &HashMap<String, Package>, visited: &mut HashSet<String>) {
    if !visited.insert(name.to_string()) { return; }
    if let Some(pkg) = packages.get(name) {
        for parent in &pkg.required_by { visit_dag(parent, packages, visited); }
    }
}

fn dep_dag_size(name: &str, packages: &HashMap<String, Package>) -> usize {
    let mut visited = HashSet::new();
    visit_dep_dag(name, packages, &mut visited);
    visited.len()
}

fn visit_dep_dag(name: &str, packages: &HashMap<String, Package>, visited: &mut HashSet<String>) {
    if !visited.insert(name.to_string()) { return; }
    if let Some(pkg) = packages.get(name) {
        for dep in &pkg.depends_on { visit_dep_dag(dep, packages, visited); }
    }
}

// ---------------------------------------------------------------------------
// Subprocess helpers
// ---------------------------------------------------------------------------

fn run_checkupdates() -> Vec<(String, String, String)> {
    // Output format: "pkgname old_ver -> new_ver"
    match Command::new("checkupdates").output() {
        Ok(out) => String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| {
                let p: Vec<&str> = line.split_whitespace().collect();
                (p.len() >= 4).then(|| (p[0].to_string(), p[1].to_string(), p[3].to_string()))
            })
            .collect(),
        Err(e) => {
            eprintln!("Failed to run checkupdates: {e}");
            vec![]
        }
    }
}

fn depgraph_path() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    Some(format!("{home}/.local/share/cinnamon/applets/pacman-updater@smanilov/depgraph.json"))
}

fn load_depgraph() -> Option<Depgraph> {
    let path = depgraph_path()?;
    let content = std::fs::read_to_string(&path)
        .map_err(|e| eprintln!("Could not read depgraph at {path}: {e}"))
        .ok()?;
    serde_json::from_str(&content)
        .map_err(|e| eprintln!("Could not parse depgraph: {e}"))
        .ok()
}

/// Run `pacman -Qi`, rebuild depgraph.json on disk, and return the new packages map.
fn rebuild_depgraph() -> HashMap<String, Package> {
    let output = match Command::new("pacman").args(["-Qi"]).output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
        Err(e) => { eprintln!("Failed to run pacman -Qi: {e}"); return HashMap::new(); }
    };

    let mut result: HashMap<String, Package> = HashMap::new();
    let mut json_pkgs = serde_json::Map::new();
    let mut fields: HashMap<String, String> = HashMap::new();

    for line in output.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if let Some(name) = fields.get("Name").cloned() {
                let version = fields.get("Version").cloned().unwrap_or_default();
                let reason = if fields.get("Install Reason").map_or(false, |r| r.contains("Explicitly")) {
                    "explicit"
                } else {
                    "dependency"
                };
                let parse_list = |key: &str| -> Vec<String> {
                    fields.get(key)
                        .filter(|v| *v != "None")
                        .map(|v| v.split_whitespace().map(str::to_string).collect())
                        .unwrap_or_default()
                };
                let required_by = parse_list("Required By");
                let depends_on: Vec<String> = parse_list("Depends On")
                    .into_iter()
                    .map(|s| {
                        let end = s.find(|c| c == '>' || c == '<' || c == '=').unwrap_or(s.len());
                        s[..end].to_string()
                    })
                    .collect();

                json_pkgs.insert(name.clone(), serde_json::json!({
                    "name": name,
                    "version": version,
                    "reason": reason,
                    "depends_on": depends_on,
                    "required_by": required_by,
                }));
                result.insert(name, Package { version, required_by, depends_on });
            }
            fields.clear();
        } else if !line.starts_with(' ') {
            if let Some(idx) = line.find(" : ") {
                fields.insert(line[..idx].trim().to_string(), line[idx + 3..].to_string());
            }
        }
    }

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let json = serde_json::json!({ "last_updated": timestamp, "packages": json_pkgs });
    if let Some(path) = depgraph_path() {
        if let Ok(s) = serde_json::to_string_pretty(&json) {
            let _ = std::fs::write(path, s);
        }
    }

    result
}

// ---------------------------------------------------------------------------
// State construction
// ---------------------------------------------------------------------------

fn build_state(
    updates_raw: Vec<(String, String, String)>,
    depgraph: Option<Depgraph>,
    initial_mode: Mode,
) -> AppState {
    let packages = depgraph.map(|d| d.packages).unwrap_or_default();

    let impacts: HashMap<String, usize> = packages
        .keys()
        .map(|name| (name.clone(), dag_size(name, &packages)))
        .collect();

    let mut roots: Vec<UpdateInfo> = updates_raw
        .into_iter()
        .map(|(name, old_version, new_version)| {
            let impact = *impacts.get(&name).unwrap_or(&1);
            UpdateInfo { name, old_version, new_version, impact }
        })
        .collect();
    roots.sort_by(|a, b| b.impact.cmp(&a.impact));

    let mut all_roots: Vec<AllPkgInfo> = packages
        .iter()
        .map(|(name, pkg)| {
            let impact = *impacts.get(name).unwrap_or(&1);
            AllPkgInfo { name: name.clone(), version: pkg.version.clone(), impact }
        })
        .collect();
    all_roots.sort_by(|a, b| b.impact.cmp(&a.impact).then(a.name.cmp(&b.name)));

    let dep_impacts: HashMap<String, usize> = packages
        .keys()
        .map(|name| (name.clone(), dep_dag_size(name, &packages)))
        .collect();

    AppState {
        roots,
        all_roots,
        packages,
        impacts,
        dep_impacts,
        expanded: HashSet::new(),
        cursor: 0,
        mode: initial_mode,
        transposed: false,
        update_succeeded: false,
        info_popup: None,
        search: None,
        show_help: false,
    }
}

fn pacman_qi(package: &str) -> String {
    match Command::new("pacman").args(["-Qi", package]).output() {
        Ok(out) => String::from_utf8_lossy(&out.stdout).into_owned(),
        Err(e) => format!("Failed to run pacman -Qi: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> Result<(), io::Error> {
    let depgraph = load_depgraph();
    let updates_raw = run_checkupdates();

    let has_depgraph = depgraph.is_some();

    if updates_raw.is_empty() && !has_depgraph {
        println!("No updates available and no depgraph found.");
        return Ok(());
    }

    let initial_mode = if updates_raw.is_empty() { Mode::AllPackages } else { Mode::Updates };
    let mut state = build_state(updates_raw, depgraph, initial_mode);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let update_succeeded = run_app(&mut terminal, &mut state)?;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    process::exit(if update_succeeded { 0 } else { 1 });
}

// ---------------------------------------------------------------------------
// TUI loop
// ---------------------------------------------------------------------------

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
) -> Result<bool, io::Error> {
    let max_update_name = state.roots.iter().map(|r| r.name.len()).max().unwrap_or(10);
    let max_old = state.roots.iter().map(|r| r.old_version.len()).max().unwrap_or(10);
    let max_all_name = state.all_roots.iter().map(|r| r.name.len()).max().unwrap_or(10);
    let mut list_state = ListState::default();

    loop {
        let vis = state.visible();
        let cursor = state.cursor;
        let update_succeeded = state.update_succeeded;
        let mode = state.mode;
        let search_query: Option<String> = state.search.as_ref().map(|s| s.query.clone());
        let show_help = state.show_help;
        let transposed = state.transposed;

        let popup_snapshot = state.info_popup.as_ref().map(|p| {
            (p.package.clone(), p.content.clone(), p.scroll)
        });

        list_state.select(Some(cursor));
        terminal.draw(|f| {
            let searching = search_query.is_some();
            let areas = Layout::default()
                .direction(Direction::Vertical)
                .constraints(if searching {
                    vec![Constraint::Min(1), Constraint::Length(1), Constraint::Length(1)]
                } else {
                    vec![Constraint::Min(1), Constraint::Length(1)]
                })
                .split(f.area());
            let list_area = areas[0];
            let (search_area, footer_area) = if searching {
                (Some(areas[1]), areas[2])
            } else {
                (None, areas[1])
            };

            let items: Vec<ListItem> = vis
                .iter()
                .map(|item| {
                    let indent = "  ".repeat(item.depth);
                    let icon = if item.is_cycle || !item.has_children {
                        " "
                    } else if item.is_expanded {
                        "▼"
                    } else {
                        "▶"
                    };
                    let cycle_suffix = if item.is_cycle { " (↺)" } else { "" };

                    let line = match &item.kind {
                        ItemKind::Root { old_version, new_version, impact } => format!(
                            "{indent}{icon} {impact:>4}  {:<name_w$}  {:<old_w$}  ->  {new_version}{cycle_suffix}",
                            item.name,
                            old_version,
                            name_w = max_update_name,
                            old_w = max_old,
                        ),
                        ItemKind::AllPkgRoot { version, impact } => format!(
                            "{indent}{icon} {impact:>4}  {:<name_w$}  {version}{cycle_suffix}",
                            item.name,
                            name_w = max_all_name,
                        ),
                        ItemKind::Dep { installed_version, impact } => {
                            let ver = installed_version.as_deref().unwrap_or("?");
                            format!("{indent}{icon} {impact:>4}  {}{cycle_suffix}  {ver}", item.name)
                        }
                    };
                    ListItem::new(Line::from(line))
                })
                .collect();

            let dir = if transposed { "deps↓" } else { "used-by↑" };
            let title = match mode {
                Mode::Updates => format!(" Pacman Updates [{dir}] — impact  package  old → new "),
                Mode::AllPackages => format!(" All Packages [{dir}] — impact  package  version "),
            };

            let list = List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(title.as_str()),
                )
                .highlight_style(
                    Style::default()
                        .bg(Color::Green)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("> ");

            f.render_stateful_widget(list, list_area, &mut list_state);

            if let (Some(area), Some(query)) = (search_area, &search_query) {
                f.render_widget(
                    Paragraph::new(format!("/ {query}█"))
                        .style(Style::default().fg(Color::Yellow)),
                    area,
                );
            }

            let (footer_text, footer_style) = if search_query.is_some() {
                (
                    "  ↑↓ navigate   enter confirm   esc cancel".to_string(),
                    Style::default().fg(Color::Yellow),
                )
            } else {
                let a_hint = match mode {
                    Mode::Updates => "a all pkgs",
                    Mode::AllPackages => "a updates",
                };
                let base = format!("  ↑↓ navigate   ←→ collapse/expand   i info   / search   r update   d remove   {a_hint}   t transpose   h help   q quit");
                if update_succeeded {
                    (format!("{base} (update succeeded)"), Style::default().fg(Color::Green))
                } else {
                    (base, Style::default().fg(Color::DarkGray))
                }
            };
            f.render_widget(Paragraph::new(footer_text).style(footer_style), footer_area);

            // Help popup overlay
            if show_help {
                let area = centered_rect(55, 60, f.area());
                f.render_widget(Clear, area);
                f.render_widget(
                    Paragraph::new(concat!(
                        "  Normal mode\n",
                        "\n",
                        "    ↑ / ↓        Navigate\n",
                        "    → / ←        Expand / collapse\n",
                        "    i            Package info (pacman -Qi)\n",
                        "    /            Search packages\n",
                        "    r            Run update (sudo pacman -Syu)\n",
                        "    d            Remove package under cursor (sudo pacman -R)\n",
                        "    a            Toggle updates / all packages\n",
                        "    t            Transpose tree (used-by ↔ depends-on)\n",
                        "    h / ?        Show this help\n",
                        "    q            Quit\n",
                        "\n",
                        "  Search mode\n",
                        "\n",
                        "    Type         Filter packages by name\n",
                        "    Backspace    Delete last character\n",
                        "    ↑ / ↓        Navigate filtered list\n",
                        "    Enter        Confirm selection\n",
                        "    Esc          Cancel search\n",
                        "\n",
                        "  any key closes this popup",
                    ))
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(" Help ")
                            .border_style(Style::default().fg(Color::Yellow)),
                    ),
                    area,
                );
            }

            // Info popup overlay
            if let Some((pkg, content, scroll)) = &popup_snapshot {
                let area = centered_rect(80, 80, f.area());
                f.render_widget(Clear, area);
                f.render_widget(
                    Paragraph::new(content.as_str())
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title(format!(" pacman -Qi {pkg} — ↑↓ scroll   any other key closes "))
                                .border_style(Style::default().fg(Color::Yellow)),
                        )
                        .wrap(Wrap { trim: false })
                        .scroll((*scroll, 0)),
                    area,
                );
            }
        })?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if state.show_help {
                    state.show_help = false;
                } else if state.info_popup.is_some() {
                    match key.code {
                        KeyCode::Up => {
                            if let Some(p) = &mut state.info_popup {
                                p.scroll = p.scroll.saturating_sub(1);
                            }
                        }
                        KeyCode::Down => {
                            if let Some(p) = &mut state.info_popup {
                                p.scroll += 1;
                            }
                        }
                        _ => state.info_popup = None,
                    }
                } else if state.search.is_some() {
                    match key.code {
                        KeyCode::Esc => {
                            let pre = state.search.as_ref().map(|s| s.pre_search_cursor).unwrap_or(0);
                            state.search = None;
                            state.cursor = pre;
                            state.clamp_cursor();
                        }
                        KeyCode::Enter => {
                            let selected_name = vis.get(state.cursor).map(|i| i.name.clone());
                            state.search = None;
                            if let Some(name) = selected_name {
                                let full_vis = state.visible();
                                if let Some(pos) = full_vis.iter().position(|i| i.depth == 0 && i.name == name) {
                                    state.cursor = pos;
                                } else {
                                    state.clamp_cursor();
                                }
                            } else {
                                state.clamp_cursor();
                            }
                        }
                        KeyCode::Up => state.move_up(),
                        KeyCode::Down => state.move_down(),
                        KeyCode::Backspace => {
                            if let Some(s) = &mut state.search {
                                s.query.pop();
                            }
                            state.cursor = 0;
                            state.clamp_cursor();
                        }
                        KeyCode::Char(c) => {
                            if let Some(s) = &mut state.search {
                                s.query.push(c);
                            }
                            state.cursor = 0;
                            state.clamp_cursor();
                        }
                        _ => {}
                    }
                } else {
                    match key.code {
                        KeyCode::Char('q') => return Ok(state.update_succeeded),
                        KeyCode::Char('h') | KeyCode::Char('?') => state.show_help = true,
                        KeyCode::Char('a') => state.toggle_mode(),
                        KeyCode::Char('t') => state.toggle_transposed(),
                        KeyCode::Char('/') => {
                            state.search = Some(SearchState {
                                query: String::new(),
                                pre_search_cursor: state.cursor,
                            });
                            state.cursor = 0;
                        }
                        KeyCode::Char('i') => {
                            let name = vis[cursor].name.clone();
                            state.info_popup = Some(InfoPopup {
                                content: pacman_qi(&name),
                                package: name,
                                scroll: 0,
                            });
                        }
                        KeyCode::Char('r') => {
                            disable_raw_mode()?;
                            execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
                            terminal.show_cursor()?;

                            if let Ok(s) = Command::new("sudo").args(["pacman", "-Syu"]).status() {
                                if s.success() {
                                    state.update_succeeded = true;
                                }
                            }

                            enable_raw_mode()?;
                            execute!(terminal.backend_mut(), EnterAlternateScreen)?;
                            terminal.clear()?;
                        }
                        KeyCode::Char('d') => {
                            let name = vis[cursor].name.clone();
                            disable_raw_mode()?;
                            execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
                            terminal.show_cursor()?;

                            Command::new("sudo").args(["pacman", "-R", &name]).status().ok();
                            let packages = rebuild_depgraph();

                            enable_raw_mode()?;
                            execute!(terminal.backend_mut(), EnterAlternateScreen)?;
                            terminal.clear()?;
                            state.reload_packages(packages);
                        }
                        KeyCode::Up => state.move_up(),
                        KeyCode::Down => state.move_down(),
                        KeyCode::Right => state.expand_current(),
                        KeyCode::Left => state.collapse_or_go_to_parent(),
                        _ => {}
                    }
                }
            }
        }
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vert[1])[1]
}
