use agentport::{
    AgentKind, Artifact, ArtifactScanner, DefaultScanner, DefaultSourceProvider, InstallPlan,
    InstallRequest, InstallScope, PreparedSource, SourceProvider, StateStore, build_plan,
    detect_agents, execute_plan, uninstall,
};
use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
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
    Review,
    Result,
}

struct InstallerApp {
    screen: Screen,
    source_input: String,
    prepared: Option<PreparedSource>,
    artifacts: Vec<Artifact>,
    artifact_selected: Vec<bool>,
    targets: Vec<TargetChoice>,
    cursor: usize,
    approve_active: bool,
    plans: Vec<InstallPlan>,
    message: String,
    project: PathBuf,
}

impl InstallerApp {
    fn new(source: Option<String>) -> Result<Self> {
        let targets = detect_agents()
            .into_iter()
            .map(|agent| TargetChoice {
                kind: agent.kind,
                selected: true,
                scope: InstallScope::Global,
                evidence: agent.evidence.join(", "),
            })
            .collect();
        let app = Self {
            screen: Screen::AgentCheck,
            source_input: source.unwrap_or_default(),
            prepared: None,
            artifacts: Vec::new(),
            artifact_selected: Vec::new(),
            targets,
            cursor: 0,
            approve_active: false,
            plans: Vec::new(),
            message: String::new(),
            project: std::env::current_dir().context("determine current project directory")?,
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
                Ok(artifacts) if !artifacts.is_empty() => {
                    self.artifact_selected = vec![true; artifacts.len()];
                    self.artifacts = artifacts;
                    self.prepared = Some(prepared);
                    self.screen = Screen::Artifacts;
                    self.cursor = 0;
                    self.message.clear();
                }
                Ok(_) => {
                    self.message = "No skills or plugin artifacts were found in this source.".into()
                }
                Err(error) => self.message = format!("Scan failed: {error:#}"),
            },
            Err(error) => self.message = format!("Source failed: {error:#}"),
        }
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
        if targets.is_empty() {
            self.message = "Select at least one detected agent.".into();
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

pub fn run_installer(source: Option<String>, store: &StateStore) -> Result<()> {
    ensure_terminal()?;
    let mut app = InstallerApp::new(source)?;
    with_terminal(|terminal| {
        loop {
            terminal.draw(|frame| draw_installer(frame, &app))?;
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
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
                    KeyCode::Backspace => {
                        app.source_input.pop();
                    }
                    KeyCode::Char(character) => app.source_input.push(character),
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
        .block(Block::default().borders(Borders::ALL)),
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
                    .block(
                        Block::default()
                            .title(" Agent availability ")
                            .borders(Borders::ALL),
                    )
                    .wrap(Wrap { trim: false }),
                chunks[1],
            );
        }
        Screen::Source => {
            let text = format!(
                "Enter a public GitHub URL, local directory, ZIP, or tar.gz:\n\n{}",
                app.source_input
            );
            frame.render_widget(
                Paragraph::new(text)
                    .block(Block::default().title(" Source ").borders(Borders::ALL))
                    .wrap(Wrap { trim: false }),
                chunks[1],
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
                    ListItem::new(format!(
                        "{cursor} [{checked}] {} — {}",
                        artifact.name,
                        artifact.summary()
                    ))
                })
                .collect::<Vec<_>>();
            frame.render_widget(
                List::new(items).block(Block::default().title(" Artifacts ").borders(Borders::ALL)),
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
                lines.push(Line::from(format!(
                    "{cursor} [{checked}] {:<15} {:<7}  {}",
                    target.kind.label(),
                    target.scope.label(),
                    target.evidence
                )));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(format!(
                "[{}] Approve active hooks/scripts/MCP commands",
                if app.approve_active { "x" } else { " " }
            )));
            frame.render_widget(
                Paragraph::new(lines)
                    .block(
                        Block::default()
                            .title(" Agent compatibility and scope ")
                            .borders(Borders::ALL),
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
                    .block(
                        Block::default()
                            .title(" Review exact changes ")
                            .borders(Borders::ALL),
                    )
                    .wrap(Wrap { trim: false }),
                chunks[1],
            );
        }
        Screen::Result => {
            frame.render_widget(
                Paragraph::new(app.message.clone())
                    .block(Block::default().title(" Result ").borders(Borders::ALL))
                    .wrap(Wrap { trim: false }),
                chunks[1],
            );
        }
    }
    let help = match app.screen {
        Screen::AgentCheck => "Enter continue  •  Esc quit",
        Screen::Source => "Enter load  •  Esc quit",
        Screen::Artifacts => "↑↓ move  •  Space toggle  •  a all  •  Enter next  •  Backspace back",
        Screen::Targets => {
            "↑↓ move  •  Space toggle  •  g global  •  p project  •  x active content  •  Enter next"
        }
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
            .block(Block::default().borders(Borders::ALL))
            .wrap(Wrap { trim: true }),
        chunks[2],
    );
}

fn with_terminal<T>(
    run: impl FnOnce(&mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<T>,
) -> Result<T> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let result = run(&mut terminal);
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
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
