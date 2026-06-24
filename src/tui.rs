use agentport::{
    AgentKind, Artifact, ArtifactScanner, DefaultScanner, DefaultSourceProvider, InstallPlan,
    InstallRequest, InstallScope, PreparedSource, SourceProvider, StateStore, build_plan,
    detect_agents, execute_plan, uninstall,
};
use anyhow::{Context, Result};
use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::process::Command;

#[derive(Clone)]
struct TargetChoice {
    kind: AgentKind,
    selected: bool,
    scope: InstallScope,
    evidence: String,
}

enum Screen {
    AgentCheck,
    Source,
    Artifacts,
    Targets,
    ProjectApproval,
    Review,
    Result,
}

struct InstallerApp {
    screen: Screen,
    source_input: String,
    source_cursor: usize,
    prepared: Option<PreparedSource>,
    artifacts: Vec<Artifact>,
    artifact_selected: Vec<bool>,
    targets: Vec<TargetChoice>,
    cursor: usize,
    approve_active: bool,
    plans: Vec<InstallPlan>,
    message: String,
    project: PathBuf,
    project_is_git: bool,
    non_git_project_approved: bool,
}

impl InstallerApp {
    fn new(source: Option<String>, global: bool) -> Result<Self> {
        let source_input = source.unwrap_or_default();
        let source_cursor = source_input.chars().count();
        let default_scope = if global {
            InstallScope::Global
        } else {
            InstallScope::Project
        };
        let (project, project_is_git) = project_root()?;
        let targets = detect_agents()
            .into_iter()
            .map(|agent| TargetChoice {
                kind: agent.kind,
                selected: true,
                scope: default_scope,
                evidence: agent.evidence.join(", "),
            })
            .collect();
        let app = Self {
            screen: Screen::AgentCheck,
            source_input,
            source_cursor,
            prepared: None,
            artifacts: Vec::new(),
            artifact_selected: Vec::new(),
            targets,
            cursor: 0,
            approve_active: false,
            plans: Vec::new(),
            message: String::new(),
            project,
            project_is_git,
            non_git_project_approved: false,
        };
        Ok(app)
    }

    fn continue_after_agent_check(&mut self) {
        if self.source_input.is_empty() {
            self.screen = Screen::Source;
        } else {
            self.load_source();
        }
        self.cursor = 0;
    }

    fn load_source(&mut self) {
        self.message = "Loading and scanning source…".into();
        match DefaultSourceProvider::default().prepare(self.source_input.trim()) {
            Ok(prepared) => match DefaultScanner.scan(&prepared.root) {
                Ok(mut artifacts) if !artifacts.is_empty() => {
                    let codex_plugins_available = self.targets.iter().any(|target| {
                        target.kind == AgentKind::Codex
                            && target.evidence.contains("'codex' found on PATH")
                    });
                    let has_standalone = artifacts
                        .iter()
                        .any(|artifact| artifact.codex_plugin.is_none());
                    if has_standalone && !codex_plugins_available {
                        artifacts.retain(|artifact| artifact.codex_plugin.is_none());
                    }
                    let has_standalone = artifacts
                        .iter()
                        .any(|artifact| artifact.codex_plugin.is_none());
                    self.artifact_selected = artifacts
                        .iter()
                        .map(|artifact| !has_standalone || artifact.codex_plugin.is_none())
                        .collect();
                    self.artifacts = artifacts;
                    self.prepared = Some(prepared);
                    self.screen = Screen::Artifacts;
                    self.cursor = 0;
                    self.message.clear();
                }
                Ok(_) => {
                    self.screen = Screen::Source;
                    self.message = "No skills or plugin artifacts were found in this source.".into()
                }
                Err(error) => {
                    self.screen = Screen::Source;
                    self.message = format!("Scan failed: {error:#}");
                }
            },
            Err(error) => {
                self.screen = Screen::Source;
                self.message = format!("Source failed: {error:#}");
            }
        }
    }

    fn insert_source_text(&mut self, text: &str) {
        let text = text.replace(['\r', '\n'], "");
        let byte = char_byte_index(&self.source_input, self.source_cursor);
        self.source_input.insert_str(byte, &text);
        self.source_cursor += text.chars().count();
        self.message.clear();
    }

    fn backspace_source(&mut self) {
        if self.source_cursor == 0 {
            return;
        }
        let start = char_byte_index(&self.source_input, self.source_cursor - 1);
        let end = char_byte_index(&self.source_input, self.source_cursor);
        self.source_input.replace_range(start..end, "");
        self.source_cursor -= 1;
        self.message.clear();
    }

    fn delete_source(&mut self) {
        if self.source_cursor == self.source_input.chars().count() {
            return;
        }
        let start = char_byte_index(&self.source_input, self.source_cursor);
        let end = char_byte_index(&self.source_input, self.source_cursor + 1);
        self.source_input.replace_range(start..end, "");
        self.message.clear();
    }

    fn make_plans(&mut self, store: &StateStore) {
        let Some(prepared) = &self.prepared else {
            return;
        };
        let targets: Vec<_> = self
            .targets
            .iter()
            .filter(|target| target.selected)
            .map(|target| (target.kind, target.scope))
            .collect();
        if targets.is_empty() {
            self.message = "Select at least one detected agent.".into();
            return;
        }
        if targets
            .iter()
            .any(|(_, scope)| *scope == InstallScope::Project)
            && !self.project_is_git
            && !self.non_git_project_approved
        {
            self.screen = Screen::ProjectApproval;
            self.cursor = 0;
            self.message.clear();
            return;
        }
        let mut plans = Vec::new();
        for (index, artifact) in self.artifacts.iter().enumerate() {
            if !self.artifact_selected[index] {
                continue;
            }
            let request = InstallRequest {
                artifact: artifact.clone(),
                source: prepared.display.clone(),
                revision: prepared.revision.clone(),
                targets: targets.clone(),
                project: self.project.clone(),
                approve_active: self.approve_active,
            };
            match build_plan(&request, store) {
                Ok(plan) => plans.push(plan),
                Err(error) => {
                    self.message = format!("Cannot build install plan: {error:#}");
                    return;
                }
            }
        }
        if plans.is_empty() {
            self.message = "Select at least one artifact.".into();
            return;
        }
        if plans
            .iter()
            .flat_map(|plan| &plan.targets)
            .all(|target| target.operations.is_empty())
        {
            let reasons = plans
                .iter()
                .flat_map(|plan| &plan.targets)
                .flat_map(|target| &target.skipped)
                .chain(plans.iter().flat_map(|plan| &plan.warnings))
                .cloned()
                .collect::<Vec<_>>();
            self.message = if reasons.is_empty() {
                "Nothing can be installed with the current artifact and target choices.".into()
            } else {
                format!("Nothing can be installed: {}", reasons.join("; "))
            };
            return;
        }
        self.plans = plans;
        self.screen = Screen::Review;
        self.cursor = 0;
        self.message.clear();
    }

    fn install(&mut self, store: &StateStore) {
        let mut installed = Vec::new();
        for plan in &self.plans {
            match execute_plan(plan, store) {
                Ok(item) => installed.push(format!("{} ({})", item.package, item.id)),
                Err(error) => {
                    self.message = format!("Installation failed: {error:#}");
                    self.screen = Screen::Result;
                    return;
                }
            }
        }
        let has_codex_plugin = self
            .plans
            .iter()
            .flat_map(|p| &p.targets)
            .flat_map(|t| &t.operations)
            .any(|op| matches!(op, agentport::PlannedOperation::InstallCodexPlugin { .. }));
        let suffix = if has_codex_plugin {
            "\n\nRestart Codex or start a new thread. Open /hooks to review and trust installed hooks."
        } else {
            ""
        };
        self.message = format!(
            "Installed successfully:\n{}{}",
            installed.join("\n"),
            suffix
        );
        self.screen = Screen::Result;
    }
}

fn project_root() -> Result<(PathBuf, bool)> {
    let current = std::env::current_dir().context("determine current project directory")?;
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(&current)
        .output();
    let Ok(output) = output else {
        return Ok((current, false));
    };
    if !output.status.success() {
        return Ok((current, false));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let root = stdout.trim();
    if root.is_empty() {
        Ok((current, false))
    } else {
        Ok((PathBuf::from(root), true))
    }
}

fn char_byte_index(value: &str, char_index: usize) -> usize {
    value
        .char_indices()
        .nth(char_index)
        .map_or(value.len(), |(index, _)| index)
}

pub fn run_installer(source: Option<String>, global: bool, store: &StateStore) -> Result<()> {
    ensure_terminal()?;
    let mut app = InstallerApp::new(source, global)?;
    with_terminal(|terminal| {
        loop {
            terminal.draw(|frame| draw_installer(frame, &app))?;
            let input_event = event::read()?;
            if matches!(app.screen, Screen::Source)
                && let Event::Paste(text) = &input_event
            {
                app.insert_source_text(text);
                continue;
            }
            let Event::Key(key) = input_event else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if (matches!(key.code, KeyCode::Esc) && !matches!(app.screen, Screen::ProjectApproval))
                || (!matches!(app.screen, Screen::Source) && matches!(key.code, KeyCode::Char('q')))
            {
                break Ok(());
            }
            match app.screen {
                Screen::AgentCheck => {
                    if matches!(key.code, KeyCode::Enter) {
                        app.continue_after_agent_check();
                    }
                }
                Screen::Source => match key.code {
                    KeyCode::Enter => app.load_source(),
                    KeyCode::Left => app.source_cursor = app.source_cursor.saturating_sub(1),
                    KeyCode::Right => {
                        app.source_cursor =
                            (app.source_cursor + 1).min(app.source_input.chars().count())
                    }
                    KeyCode::Home => app.source_cursor = 0,
                    KeyCode::End => app.source_cursor = app.source_input.chars().count(),
                    KeyCode::Backspace => app.backspace_source(),
                    KeyCode::Delete => app.delete_source(),
                    KeyCode::Char(character) => app.insert_source_text(&character.to_string()),
                    _ => {}
                },
                Screen::Artifacts => match key.code {
                    KeyCode::Up => app.cursor = app.cursor.saturating_sub(1),
                    KeyCode::Down => {
                        app.cursor = (app.cursor + 1).min(app.artifacts.len().saturating_sub(1))
                    }
                    KeyCode::Char(' ') => {
                        if let Some(selected) = app.artifact_selected.get_mut(app.cursor) {
                            *selected = !*selected;
                        }
                    }
                    KeyCode::Char('a') => {
                        let select = app.artifact_selected.iter().any(|selected| !selected);
                        app.artifact_selected.fill(select);
                    }
                    KeyCode::Enter => {
                        app.screen = Screen::Targets;
                        app.cursor = 0;
                        app.message.clear();
                    }
                    KeyCode::Backspace => {
                        app.screen = Screen::Source;
                        app.cursor = 0;
                    }
                    _ => {}
                },
                Screen::Targets => match key.code {
                    KeyCode::Up => app.cursor = app.cursor.saturating_sub(1),
                    KeyCode::Down => {
                        app.cursor = (app.cursor + 1).min(app.targets.len().saturating_sub(1))
                    }
                    KeyCode::Char(' ') => {
                        if let Some(target) = app.targets.get_mut(app.cursor) {
                            target.selected = !target.selected;
                        }
                    }
                    KeyCode::Char('a') => {
                        let select = app.targets.iter().any(|target| !target.selected);
                        for target in &mut app.targets {
                            target.selected = select;
                        }
                    }
                    KeyCode::Char('g') => {
                        if let Some(target) = app.targets.get_mut(app.cursor) {
                            target.scope = InstallScope::Global;
                        }
                    }
                    KeyCode::Char('p') => {
                        if let Some(target) = app.targets.get_mut(app.cursor) {
                            let requires_global = target.kind == AgentKind::Codex
                                && app.artifacts.iter().enumerate().any(|(index, artifact)| {
                                    app.artifact_selected[index]
                                        && (artifact.codex_plugin.is_some()
                                            || artifact.components.iter().any(|component| {
                                                component.kind == agentport::ComponentKind::Hook
                                            }))
                                });
                            if requires_global {
                                app.message = "Codex plugins and hooks are global-only.".into();
                            } else {
                                target.scope = InstallScope::Project;
                            }
                        }
                    }
                    KeyCode::Char('x') => app.approve_active = !app.approve_active,
                    KeyCode::Enter => app.make_plans(store),
                    KeyCode::Backspace => {
                        app.screen = Screen::Artifacts;
                        app.cursor = 0;
                    }
                    _ => {}
                },
                Screen::ProjectApproval => match key.code {
                    KeyCode::Char('y') => {
                        app.non_git_project_approved = true;
                        app.make_plans(store);
                    }
                    KeyCode::Char('n') | KeyCode::Backspace | KeyCode::Esc => {
                        app.screen = Screen::Targets;
                        app.cursor = 0;
                    }
                    _ => {}
                },
                Screen::Review => match key.code {
                    KeyCode::Enter => app.install(store),
                    KeyCode::Backspace => {
                        app.screen = Screen::Targets;
                        app.cursor = 0;
                    }
                    _ => {}
                },
                Screen::Result => {
                    if matches!(key.code, KeyCode::Enter) {
                        break Ok(());
                    }
                }
            }
        }
    })
}

pub fn run_uninstall(package: Option<String>, store: &StateStore) -> Result<()> {
    ensure_terminal()?;
    let installations = store.list()?;
    if installations.is_empty() {
        println!("No Agentport-managed installations.");
        return Ok(());
    }
    let mut cursor = package
        .as_ref()
        .and_then(|query| {
            installations
                .iter()
                .position(|item| item.id == *query || item.package == *query)
        })
        .unwrap_or(0);
    let mut confirmation = false;
    let mut message = String::new();
    with_terminal(|terminal| {
        loop {
            terminal.draw(|frame| {
                let area = frame.area();
                let title = if confirmation {
                    " Confirm uninstall "
                } else {
                    " Managed installations "
                };
                let items: Vec<_> = installations
                    .iter()
                    .enumerate()
                    .map(|(index, item)| {
                        let marker = if index == cursor { ">" } else { " " };
                        ListItem::new(format!(
                            "{marker} {}  {}  {} target(s)",
                            item.package,
                            item.id,
                            item.targets.len()
                        ))
                    })
                    .collect();
                let block = Block::default().title(title).borders(Borders::ALL);
                frame.render_widget(List::new(items).block(block), area);
                if confirmation {
                    let popup = centered(area, 64, 7);
                    frame.render_widget(ratatui::widgets::Clear, popup);
                    frame.render_widget(
                        Paragraph::new(format!(
                            "Remove '{}'?\n\n[y] yes   [n] no",
                            installations[cursor].package
                        ))
                        .block(Block::default().borders(Borders::ALL))
                        .wrap(Wrap { trim: true }),
                        popup,
                    );
                }
                if !message.is_empty() {
                    let popup = centered(area, 70, 7);
                    frame.render_widget(ratatui::widgets::Clear, popup);
                    frame.render_widget(
                        Paragraph::new(message.clone())
                            .block(Block::default().title(" Result ").borders(Borders::ALL))
                            .wrap(Wrap { trim: true }),
                        popup,
                    );
                }
            })?;
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if !message.is_empty() {
                if matches!(key.code, KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q')) {
                    break Ok(());
                }
                continue;
            }
            if confirmation {
                match key.code {
                    KeyCode::Char('y') => {
                        let report = uninstall(store, &installations[cursor].id)?;
                        message = format!(
                            "Removed {} file(s). Preserved {} modified file(s).\nPress Enter.",
                            report.removed.len(),
                            report.preserved.len()
                        );
                    }
                    KeyCode::Char('n') | KeyCode::Esc => confirmation = false,
                    _ => {}
                }
                continue;
            }
            match key.code {
                KeyCode::Up => cursor = cursor.saturating_sub(1),
                KeyCode::Down => cursor = (cursor + 1).min(installations.len() - 1),
                KeyCode::Enter => confirmation = true,
                KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                _ => {}
            }
        }
    })
}

fn draw_installer(frame: &mut ratatui::Frame<'_>, app: &InstallerApp) {
    let panel_border = Style::default().fg(Color::Cyan);
    let surface = Style::default().fg(Color::White).bg(Color::Rgb(12, 28, 38));
    let focused = Style::default()
        .fg(Color::White)
        .bg(Color::Rgb(24, 52, 72))
        .add_modifier(Modifier::BOLD);
    let checked_style = Style::default()
        .fg(Color::LightGreen)
        .bg(Color::Rgb(12, 28, 38))
        .add_modifier(Modifier::BOLD);
    let focused_checked_style = Style::default()
        .fg(Color::LightGreen)
        .bg(Color::Rgb(24, 52, 72))
        .add_modifier(Modifier::BOLD);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(frame.area());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " Agentport ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Source → Artifact → Agents → Review → Install"),
        ]))
        .style(surface)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(panel_border)
                .style(surface),
        ),
        chunks[0],
    );
    match app.screen {
        Screen::AgentCheck => {
            let mut lines = vec![
                Line::from("Check which coding agents are available on this system:"),
                Line::from(""),
            ];
            for kind in AgentKind::ALL {
                if let Some(target) = app.targets.iter().find(|target| target.kind == kind) {
                    lines.push(Line::from(vec![
                        Span::styled("● ", Style::default().fg(Color::Green)),
                        Span::styled(
                            format!("{:<15}", kind.label()),
                            Style::default().add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(format!(" available  {}", target.evidence)),
                    ]));
                } else {
                    lines.push(Line::from(vec![
                        Span::styled("● ", Style::default().fg(Color::Red)),
                        Span::styled(
                            format!("{:<15}", kind.label()),
                            Style::default().add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(" not detected", Style::default().fg(Color::DarkGray)),
                    ]));
                }
            }
            lines.push(Line::from(""));
            lines.push(Line::from("Press Enter when you have checked the list."));
            frame.render_widget(
                Paragraph::new(lines)
                    .style(surface)
                    .block(
                        Block::default()
                            .title(" Agent availability ")
                            .borders(Borders::ALL)
                            .border_style(panel_border)
                            .style(surface),
                    )
                    .wrap(Wrap { trim: false }),
                chunks[1],
            );
        }
        Screen::Source => {
            let source_block = Block::default()
                .title(" Source ")
                .borders(Borders::ALL)
                .border_style(panel_border)
                .style(surface);
            let source_area = source_block.inner(chunks[1]);
            frame.render_widget(source_block, chunks[1]);
            let source_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(2),
                    Constraint::Length(3),
                    Constraint::Min(0),
                ])
                .split(source_area);
            frame.render_widget(
                Paragraph::new("Paste a public GitHub URL or enter a local package path:")
                    .style(surface),
                source_chunks[0],
            );
            let input_width = source_chunks[1].width.saturating_sub(2) as usize;
            let viewport_start = app
                .source_cursor
                .saturating_sub(input_width.saturating_sub(1));
            let before_cursor = app
                .source_input
                .chars()
                .skip(viewport_start)
                .take(app.source_cursor.saturating_sub(viewport_start))
                .collect::<String>();
            let cursor_character = app
                .source_input
                .chars()
                .nth(app.source_cursor)
                .unwrap_or(' ')
                .to_string();
            let after_cursor = app
                .source_input
                .chars()
                .skip(app.source_cursor.saturating_add(1))
                .take(
                    input_width
                        .saturating_sub(app.source_cursor.saturating_sub(viewport_start) + 1),
                )
                .collect::<String>();
            let input_style = Style::default().fg(Color::White).bg(Color::Rgb(24, 52, 72));
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::raw(before_cursor),
                    Span::styled(
                        cursor_character,
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(after_cursor),
                ]))
                .style(input_style)
                .block(
                    Block::default()
                        .title(" URL or path ")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan))
                        .style(input_style),
                ),
                source_chunks[1],
            );
        }
        Screen::Artifacts => {
            let items = app
                .artifacts
                .iter()
                .enumerate()
                .map(|(index, artifact)| {
                    let cursor = if index == app.cursor { ">" } else { " " };
                    let checked = if app.artifact_selected[index] {
                        "x"
                    } else {
                        " "
                    };
                    let style = match (index == app.cursor, app.artifact_selected[index]) {
                        (true, true) => focused_checked_style,
                        (true, false) => focused,
                        (false, true) => checked_style,
                        (false, false) => surface,
                    };
                    ListItem::new(format!(
                        "{cursor} [{checked}] {} — {}",
                        artifact.name,
                        artifact.summary()
                    ))
                    .style(style)
                })
                .collect::<Vec<_>>();
            frame.render_widget(
                List::new(items).style(surface).block(
                    Block::default()
                        .title(" Select artifacts ")
                        .borders(Borders::ALL)
                        .border_style(panel_border)
                        .style(surface),
                ),
                chunks[1],
            );
        }
        Screen::Targets => {
            let mut lines = Vec::new();
            if app.targets.is_empty() {
                lines.push(Line::from(
                    "No Codex, Claude Code, or Copilot installation was detected.",
                ));
            }
            for (index, target) in app.targets.iter().enumerate() {
                let cursor = if index == app.cursor { ">" } else { " " };
                let checked = if target.selected { "x" } else { " " };
                lines.push(Line::styled(
                    format!(
                        "{cursor} [{checked}] {:<15} {:<7}  {}",
                        target.kind.label(),
                        target.scope.label(),
                        target.evidence
                    ),
                    match (index == app.cursor, target.selected) {
                        (true, true) => focused_checked_style,
                        (true, false) => focused,
                        (false, true) => checked_style,
                        (false, false) => surface,
                    },
                ));
            }
            lines.push(Line::from(""));
            lines.push(Line::styled(
                format!(
                    "[{}] Approve active hooks/scripts/MCP commands  (x toggles)",
                    if app.approve_active { "x" } else { " " }
                ),
                if app.approve_active {
                    Style::default()
                        .fg(Color::LightGreen)
                        .bg(Color::Rgb(24, 52, 72))
                        .add_modifier(Modifier::BOLD)
                } else {
                    surface
                },
            ));
            frame.render_widget(
                Paragraph::new(lines)
                    .style(surface)
                    .block(
                        Block::default()
                            .title(" Agent compatibility and scope ")
                            .borders(Borders::ALL)
                            .border_style(panel_border)
                            .style(surface),
                    )
                    .wrap(Wrap { trim: false }),
                chunks[1],
            );
        }
        Screen::ProjectApproval => {
            let lines = vec![
                Line::from("The current directory is not inside a Git repository."),
                Line::from(""),
                Line::from(format!(
                    "Project-scope skills will be installed into {}",
                    app.project.join(".agents/skills").display()
                )),
                Line::from(""),
                Line::from("Press y to approve this local install, or n to go back."),
            ];
            frame.render_widget(
                Paragraph::new(lines)
                    .style(surface)
                    .block(
                        Block::default()
                            .title(" Confirm non-repository install ")
                            .borders(Borders::ALL)
                            .border_style(panel_border)
                            .style(surface),
                    )
                    .wrap(Wrap { trim: false }),
                chunks[1],
            );
        }
        Screen::Review => {
            let mut lines = Vec::new();
            for plan in &app.plans {
                lines.push(Line::styled(
                    plan.package.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ));
                for target in &plan.targets {
                    lines.push(Line::from(format!(
                        "  {} ({})",
                        target.agent.label(),
                        target.scope.label()
                    )));
                    for operation in &target.operations {
                        lines.push(Line::from(format!("    + {}", operation.display())));
                    }
                    for skipped in &target.skipped {
                        lines.push(Line::styled(
                            format!("    – {skipped}"),
                            Style::default().fg(Color::Yellow),
                        ));
                    }
                }
            }
            frame.render_widget(
                Paragraph::new(lines)
                    .style(surface)
                    .block(
                        Block::default()
                            .title(" Review exact changes ")
                            .borders(Borders::ALL)
                            .border_style(panel_border)
                            .style(surface),
                    )
                    .wrap(Wrap { trim: false }),
                chunks[1],
            );
        }
        Screen::Result => {
            frame.render_widget(
                Paragraph::new(app.message.clone())
                    .style(surface)
                    .block(
                        Block::default()
                            .title(" Result ")
                            .borders(Borders::ALL)
                            .border_style(panel_border)
                            .style(surface),
                    )
                    .wrap(Wrap { trim: false }),
                chunks[1],
            );
        }
    }
    let help = match app.screen {
        Screen::AgentCheck => "Enter continue  •  Esc quit",
        Screen::Source => "Type or paste  •  ←→ move  •  Backspace/Delete edit  •  Enter load",
        Screen::Artifacts => "↑↓ move  •  Space toggle  •  a all  •  Enter next  •  Backspace back",
        Screen::Targets => {
            "↑↓ move  •  Space toggle  •  g global  •  p project  •  x active content  •  Enter next"
        }
        Screen::ProjectApproval => "y approve  •  n/backspace return",
        Screen::Review => "Enter install  •  Backspace back  •  Esc quit",
        Screen::Result => "Enter close",
    };
    let footer = if app.message.is_empty() {
        help.to_owned()
    } else {
        format!("{}  │  {}", app.message, help)
    };
    frame.render_widget(
        Paragraph::new(footer)
            .style(surface)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(panel_border)
                    .style(surface),
            )
            .wrap(Wrap { trim: true }),
        chunks[2],
    );
}

fn with_terminal<T>(
    run: impl FnOnce(&mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<T>,
) -> Result<T> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.hide_cursor()?;
    let result = run(&mut terminal);
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableBracketedPaste,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    result
}

fn ensure_terminal() -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        anyhow::bail!("Agentport's interactive command requires a terminal");
    }
    Ok(())
}

fn centered(area: ratatui::layout::Rect, width: u16, height: u16) -> ratatui::layout::Rect {
    let width = width.min(area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2));
    ratatui::layout::Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_input_supports_insertion_and_deletion_at_cursor() {
        let mut app = InstallerApp::new(Some("ab界d".into()), false).unwrap();
        app.source_cursor = 2;
        app.insert_source_text("c");
        assert_eq!(app.source_input, "abc界d");
        assert_eq!(app.source_cursor, 3);

        app.delete_source();
        assert_eq!(app.source_input, "abcd");
        app.backspace_source();
        assert_eq!(app.source_input, "abd");
        assert_eq!(app.source_cursor, 2);
    }

    #[test]
    fn pasted_source_discards_line_endings() {
        let mut app = InstallerApp::new(None, false).unwrap();
        app.insert_source_text("https://example.test/repo\r\n");
        assert_eq!(app.source_input, "https://example.test/repo");
        assert_eq!(app.source_cursor, app.source_input.chars().count());
    }

    #[test]
    fn global_flag_controls_initial_scope() {
        let local = InstallerApp::new(None, false).unwrap();
        assert!(
            local
                .targets
                .iter()
                .all(|target| target.scope == InstallScope::Project)
        );

        let global = InstallerApp::new(None, true).unwrap();
        assert!(
            global
                .targets
                .iter()
                .all(|target| target.scope == InstallScope::Global)
        );
    }
}
