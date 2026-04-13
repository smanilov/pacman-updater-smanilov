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
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Terminal,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Package {
    reason: String,
    required_by: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Depgraph {
    packages: HashMap<String, Package>,
}

#[derive(Debug)]
struct UpdateEntry {
    name: String,
    old_version: String,
    new_version: String,
    impact: usize,
}

fn dag_size(name: &str, packages: &HashMap<String, Package>) -> usize {
    let mut visited = HashSet::new();
    visit(name, packages, &mut visited);
    visited.len()
}

fn visit(name: &str, packages: &HashMap<String, Package>, visited: &mut HashSet<String>) {
    if visited.contains(name) {
        return;
    }
    visited.insert(name.to_string());
    if let Some(pkg) = packages.get(name) {
        if pkg.reason == "explicit" || pkg.required_by.is_empty() {
            return;
        }
        for parent in &pkg.required_by {
            visit(parent, packages, visited);
        }
    }
}

fn run_checkupdates() -> Vec<(String, String, String)> {
    // checkupdates output format: "pkgname old_ver -> new_ver"
    let output = Command::new("checkupdates").output();
    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout
                .lines()
                .filter_map(|line| {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 4 {
                        Some((
                            parts[0].to_string(),
                            parts[1].to_string(),
                            parts[3].to_string(),
                        ))
                    } else {
                        None
                    }
                })
                .collect()
        }
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

fn main() -> Result<(), io::Error> {
    let depgraph = load_depgraph();
    let updates_raw = run_checkupdates();

    let mut entries: Vec<UpdateEntry> = updates_raw
        .into_iter()
        .map(|(name, old_version, new_version)| {
            let impact = depgraph
                .as_ref()
                .map(|g| dag_size(&name, &g.packages))
                .unwrap_or(1);
            UpdateEntry { name, old_version, new_version, impact }
        })
        .collect();

    entries.sort_by(|a, b| b.impact.cmp(&a.impact));

    if entries.is_empty() {
        println!("No updates available.");
        return Ok(());
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut list_state = ListState::default();
    list_state.select(Some(0));

    let update_succeeded = run_app(&mut terminal, &entries, &mut list_state)?;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    process::exit(if update_succeeded { 0 } else { 1 });
}

/// Runs the TUI event loop. Returns true if `sudo pacman -Syu` completed
/// successfully at least once before the user quit.
fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    entries: &[UpdateEntry],
    list_state: &mut ListState,
) -> Result<bool, io::Error> {
    let max_name = entries.iter().map(|e| e.name.len()).max().unwrap_or(10);
    let max_old = entries.iter().map(|e| e.old_version.len()).max().unwrap_or(10);

    let mut update_succeeded = false;

    loop {
        terminal.draw(|f| {
            let [list_area, footer_area] = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1)])
                .areas(f.area());

            let items: Vec<ListItem> = entries
                .iter()
                .map(|e| {
                    let line = format!(
                        "{:>4}  {:<name_w$}  {:<old_w$}  ->  {}",
                        e.impact,
                        e.name,
                        e.old_version,
                        e.new_version,
                        name_w = max_name,
                        old_w = max_old,
                    );
                    ListItem::new(Line::from(line))
                })
                .collect();

            let footer_text = if update_succeeded {
                "  ↑↓ navigate   r run update   q quit (update succeeded)"
            } else {
                "  ↑↓ navigate   r run update   q quit"
            };

            let list = List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Pacman Updates — impact  package  old → new "),
                )
                .highlight_style(
                    Style::default()
                        .bg(Color::Blue)
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("> ");

            f.render_stateful_widget(list, list_area, list_state);

            let footer_style = if update_succeeded {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let footer = Paragraph::new(footer_text).style(footer_style);
            f.render_widget(footer, footer_area);
        })?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => return Ok(update_succeeded),
                    KeyCode::Char('r') => {
                        // Suspend TUI, run pacman, resume TUI.
                        disable_raw_mode()?;
                        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
                        terminal.show_cursor()?;

                        let status = Command::new("sudo")
                            .args(["pacman", "-Syu"])
                            .status();

                        if let Ok(s) = status {
                            if s.success() {
                                update_succeeded = true;
                            }
                        }

                        enable_raw_mode()?;
                        execute!(terminal.backend_mut(), EnterAlternateScreen)?;
                        terminal.clear()?;
                    }
                    KeyCode::Up => {
                        let i = list_state.selected().unwrap_or(0);
                        if i > 0 {
                            list_state.select(Some(i - 1));
                        }
                    }
                    KeyCode::Down => {
                        let i = list_state.selected().unwrap_or(0);
                        if i + 1 < entries.len() {
                            list_state.select(Some(i + 1));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
