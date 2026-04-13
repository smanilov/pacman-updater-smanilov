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

struct AppState {
    roots: Vec<UpdateInfo>,
    all_roots: Vec<AllPkgInfo>,
    packages: HashMap<String, Package>,
    /// Pre-computed impact (dag_size) for every package in the depgraph.
    impacts: HashMap<String, usize>,
    expanded: HashSet<String>,
    cursor: usize,
    mode: Mode,
    update_succeeded: bool,
    info_popup: Option<InfoPopup>,
}

impl AppState {
    fn visible(&self) -> Vec<VisibleItem> {
        let mut out = vec![];
        match self.mode {
            Mode::Updates => {
                for root in &self.roots {
                    let req_by = self.req_by(&root.name);
                    let is_expanded = self.expanded.contains(&root.name);
                    out.push(VisibleItem {
                        name: root.name.clone(),
                        depth: 0,
                        kind: ItemKind::Root {
                            old_version: root.old_version.clone(),
                            new_version: root.new_version.clone(),
                            impact: root.impact,
                        },
                        has_children: !req_by.is_empty(),
                        is_expanded,
                        is_cycle: false,
                    });
                    if is_expanded {
                        let mut ancestors = HashSet::from([root.name.clone()]);
                        self.collect_dep_children(req_by, 1, &mut ancestors, &mut out);
                    }
                }
            }
            Mode::AllPackages => {
                for pkg in &self.all_roots {
                    let req_by = self.req_by(&pkg.name);
                    let is_expanded = self.expanded.contains(&pkg.name);
                    out.push(VisibleItem {
                        name: pkg.name.clone(),
                        depth: 0,
                        kind: ItemKind::AllPkgRoot {
                            version: pkg.version.clone(),
                            impact: pkg.impact,
                        },
                        has_children: !req_by.is_empty(),
                        is_expanded,
                        is_cycle: false,
                    });
                    if is_expanded {
                        let mut ancestors = HashSet::from([pkg.name.clone()]);
                        self.collect_dep_children(req_by, 1, &mut ancestors, &mut out);
                    }
                }
            }
        }
        out
    }

    fn req_by(&self, name: &str) -> Vec<String> {
        self.packages
            .get(name)
            .map(|p| {
                let mut v = p.required_by.clone();
                v.sort();
                v
            })
            .unwrap_or_default()
    }

    fn collect_dep_children(
        &self,
        names: Vec<String>,
        depth: usize,
        ancestors: &mut HashSet<String>,
        out: &mut Vec<VisibleItem>,
    ) {
        for name in names {
            let is_cycle = ancestors.contains(&name);
            let req_by = if is_cycle { vec![] } else { self.req_by(&name) };
            let has_children = !req_by.is_empty();
            let is_expanded = !is_cycle && self.expanded.contains(&name);
            let installed_version = self.packages.get(&name).map(|p| p.version.clone());
            let impact = *self.impacts.get(&name).unwrap_or(&1);
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
                self.collect_dep_children(req_by, depth + 1, ancestors, out);
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

    fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            Mode::Updates => Mode::AllPackages,
            Mode::AllPackages => Mode::Updates,
        };
        self.cursor = 0;
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
    if !visited.insert(name.to_string()) {
        return;
    }
    if let Some(pkg) = packages.get(name) {
        for parent in &pkg.required_by {
            visit_dag(parent, packages, visited);
        }
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

fn load_depgraph() -> Option<Depgraph> {
    let home = std::env::var("HOME").ok()?;
    let path = format!(
        "{home}/.local/share/cinnamon/applets/pacman-updater@smanilov/depgraph.json"
    );
    let content = std::fs::read_to_string(&path)
        .map_err(|e| eprintln!("Could not read depgraph at {path}: {e}"))
        .ok()?;
    serde_json::from_str(&content)
        .map_err(|e| eprintln!("Could not parse depgraph: {e}"))
        .ok()
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

    AppState {
        roots,
        all_roots,
        packages,
        impacts,
        expanded: HashSet::new(),
        cursor: 0,
        mode: initial_mode,
        update_succeeded: false,
        info_popup: None,
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

    loop {
        let vis = state.visible();
        let cursor = state.cursor;
        let update_succeeded = state.update_succeeded;
        let mode = state.mode;

        let popup_snapshot = state.info_popup.as_ref().map(|p| {
            (p.package.clone(), p.content.clone(), p.scroll)
        });

        terminal.draw(|f| {
            let [list_area, footer_area] = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1)])
                .areas(f.area());

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

            let mut list_state = ListState::default();
            list_state.select(Some(cursor));

            let title = match mode {
                Mode::Updates => " Pacman Updates — impact  package  old → new ",
                Mode::AllPackages => " All Packages — impact  package  version ",
            };

            let list = List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(title),
                )
                .highlight_style(
                    Style::default()
                        .bg(Color::Green)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("> ");

            f.render_stateful_widget(list, list_area, &mut list_state);

            let tab_hint = match mode {
                Mode::Updates => "a all pkgs",
                Mode::AllPackages => "a updates",
            };
            let (footer_text, footer_style) = if update_succeeded {
                (
                    format!("  ↑↓ navigate   ←→ collapse/expand   i info   r run update   {tab_hint}   q quit (update succeeded)"),
                    Style::default().fg(Color::Green),
                )
            } else {
                (
                    format!("  ↑↓ navigate   ←→ collapse/expand   i info   r run update   {tab_hint}   q quit"),
                    Style::default().fg(Color::DarkGray),
                )
            };
            f.render_widget(Paragraph::new(footer_text).style(footer_style), footer_area);

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
                if state.info_popup.is_some() {
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
                } else {
                    match key.code {
                        KeyCode::Char('q') => return Ok(state.update_succeeded),
                        KeyCode::Char('a') => state.toggle_mode(),
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
