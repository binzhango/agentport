use crate::model::{AgentFormat, AgentKind, Component, ComponentKind, DetectedAgent, InstallScope};
use std::env;
use std::path::{Path, PathBuf};

pub trait AgentAdapter {
    fn kind(&self) -> AgentKind;
    fn supports(&self, component: &Component) -> bool;
    fn destination(
        &self,
        component: &Component,
        scope: InstallScope,
        project: &Path,
    ) -> Option<PathBuf>;
}

#[derive(Debug, Clone, Copy)]
pub struct NativeAdapter(pub AgentKind);

impl AgentAdapter for NativeAdapter {
    fn kind(&self) -> AgentKind {
        self.0
    }

    fn supports(&self, component: &Component) -> bool {
        match component.kind {
            ComponentKind::Plugin => matches!(self.0, AgentKind::Codex),
            ComponentKind::Skill | ComponentKind::Command => true,
            ComponentKind::Agent => matches!(
                (self.0, component.agent_format),
                (AgentKind::Codex, Some(AgentFormat::CodexToml))
                    | (
                        AgentKind::Claude
                            | AgentKind::Cursor
                            | AgentKind::Gemini
                            | AgentKind::Copilot,
                        Some(AgentFormat::Markdown)
                    )
                    | (
                        AgentKind::Claude
                            | AgentKind::Cursor
                            | AgentKind::Gemini
                            | AgentKind::Copilot,
                        Some(AgentFormat::Harness)
                    )
            ),
            ComponentKind::Hook => {
                matches!(self.0, AgentKind::Claude | AgentKind::Copilot)
                    && hook_schema_matches(&component.source, self.0)
            }
            // Standalone MCP destinations differ too much to promise a lossless merge in v1.
            ComponentKind::Mcp => false,
        }
    }

    fn destination(
        &self,
        component: &Component,
        scope: InstallScope,
        project: &Path,
    ) -> Option<PathBuf> {
        if !self.supports(component) {
            return None;
        }
        let home = dirs::home_dir()?;
        let base = match (self.0, scope, component.kind) {
            (AgentKind::Codex, _, ComponentKind::Plugin) => return None,
            (
                AgentKind::Codex,
                InstallScope::Global,
                ComponentKind::Skill | ComponentKind::Command,
            ) => env_path("CODEX_HOME")
                .unwrap_or_else(|| home.join(".codex"))
                .join("skills"),
            (_, InstallScope::Project, ComponentKind::Skill | ComponentKind::Command) => {
                project.join(".agents/skills")
            }
            (
                AgentKind::Claude,
                InstallScope::Global,
                ComponentKind::Skill | ComponentKind::Command,
            ) => env_path("CLAUDE_CONFIG_DIR")
                .unwrap_or_else(|| home.join(".claude"))
                .join("skills"),
            (
                AgentKind::Copilot,
                InstallScope::Global,
                ComponentKind::Skill | ComponentKind::Command,
            ) => env_path("COPILOT_HOME")
                .unwrap_or_else(|| home.join(".copilot"))
                .join("skills"),
            (
                AgentKind::Cursor,
                InstallScope::Global,
                ComponentKind::Skill | ComponentKind::Command,
            ) => env_path("CURSOR_HOME")
                .unwrap_or_else(|| home.join(".cursor"))
                .join("skills"),
            (
                AgentKind::Gemini,
                InstallScope::Global,
                ComponentKind::Skill | ComponentKind::Command,
            ) => env_path("GEMINI_HOME")
                .unwrap_or_else(|| home.join(".gemini"))
                .join("skills"),
            (AgentKind::Codex, InstallScope::Global, ComponentKind::Agent) => {
                env_path("CODEX_HOME")
                    .unwrap_or_else(|| home.join(".codex"))
                    .join("agents")
            }
            (AgentKind::Codex, InstallScope::Project, ComponentKind::Agent) => {
                project.join(".codex/agents")
            }
            (AgentKind::Claude, InstallScope::Global, ComponentKind::Agent) => {
                env_path("CLAUDE_CONFIG_DIR")
                    .unwrap_or_else(|| home.join(".claude"))
                    .join("agents")
            }
            (AgentKind::Claude, InstallScope::Project, ComponentKind::Agent) => {
                project.join(".claude/agents")
            }
            (AgentKind::Cursor, InstallScope::Global, ComponentKind::Agent) => {
                env_path("CURSOR_HOME")
                    .unwrap_or_else(|| home.join(".cursor"))
                    .join("agents")
            }
            (AgentKind::Cursor, InstallScope::Project, ComponentKind::Agent) => {
                project.join(".cursor/agents")
            }
            (AgentKind::Gemini, InstallScope::Global, ComponentKind::Agent) => {
                env_path("GEMINI_HOME")
                    .unwrap_or_else(|| home.join(".gemini"))
                    .join("agents")
            }
            (AgentKind::Gemini, InstallScope::Project, ComponentKind::Agent) => {
                project.join(".gemini/agents")
            }
            (AgentKind::Copilot, InstallScope::Global, ComponentKind::Agent) => {
                env_path("COPILOT_HOME")
                    .unwrap_or_else(|| home.join(".copilot"))
                    .join("agents")
            }
            (AgentKind::Copilot, InstallScope::Project, ComponentKind::Agent) => {
                project.join(".github/agents")
            }
            (AgentKind::Copilot, InstallScope::Global, ComponentKind::Hook) => {
                env_path("COPILOT_HOME")
                    .unwrap_or_else(|| home.join(".copilot"))
                    .join("hooks")
            }
            (AgentKind::Copilot, InstallScope::Project, ComponentKind::Hook) => {
                project.join(".github/hooks")
            }
            // Claude hook files are retained as package assets but cannot be loaded standalone without
            // mutating settings.json, so schema-compatible hooks are reported but skipped for now.
            (AgentKind::Claude | AgentKind::Cursor | AgentKind::Gemini, _, ComponentKind::Hook) => {
                return None;
            }
            _ => return None,
        };
        let file_name = match component.kind {
            ComponentKind::Agent if component.agent_format == Some(AgentFormat::CodexToml) => {
                format!("{}.toml", component.name)
            }
            ComponentKind::Agent => format!("{}.md", component.name),
            ComponentKind::Hook => format!("{}.json", component.name),
            _ => component.name.clone(),
        };
        Some(base.join(file_name))
    }
}

pub fn detect_agents() -> Vec<DetectedAgent> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    AgentKind::ALL
        .iter()
        .filter_map(|kind| {
            let (binary, directory) = match kind {
                AgentKind::Codex => (
                    "codex",
                    env_path("CODEX_HOME").unwrap_or_else(|| home.join(".codex")),
                ),
                AgentKind::Claude => (
                    "claude",
                    env_path("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.join(".claude")),
                ),
                AgentKind::Cursor => (
                    "cursor",
                    env_path("CURSOR_HOME").unwrap_or_else(|| home.join(".cursor")),
                ),
                AgentKind::Gemini => (
                    "gemini",
                    env_path("GEMINI_HOME").unwrap_or_else(|| home.join(".gemini")),
                ),
                AgentKind::Copilot => (
                    "copilot",
                    env_path("COPILOT_HOME").unwrap_or_else(|| home.join(".copilot")),
                ),
            };
            let mut evidence = Vec::new();
            if binary_on_path(binary) {
                evidence.push(format!("'{binary}' found on PATH"));
            }
            if directory.is_dir() {
                evidence.push(format!("{} exists", directory.display()));
            }
            (!evidence.is_empty()).then_some(DetectedAgent {
                kind: *kind,
                evidence,
                home: directory,
            })
        })
        .collect()
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn binary_on_path(name: &str) -> bool {
    env::var_os("PATH").is_some_and(|path| {
        env::split_paths(&path).any(|directory| {
            let candidate = directory.join(name);
            candidate.is_file()
        })
    })
}

fn hook_schema_matches(path: &Path, agent: AgentKind) -> bool {
    let Ok(data) = std::fs::read(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&data) else {
        return false;
    };
    let Some(hooks) = value.get("hooks").and_then(serde_json::Value::as_object) else {
        return false;
    };
    let allowed: &[&str] = match agent {
        AgentKind::Claude => &[
            "PreToolUse",
            "PostToolUse",
            "PostToolUseFailure",
            "PermissionRequest",
            "UserPromptSubmit",
            "Notification",
            "Stop",
            "SubagentStart",
            "SubagentStop",
            "SessionStart",
            "SessionEnd",
            "PreCompact",
        ],
        AgentKind::Copilot => &[
            "sessionStart",
            "sessionEnd",
            "userPromptSubmitted",
            "preToolUse",
            "postToolUse",
            "errorOccurred",
            "agentStop",
            "subagentStop",
            "permissionRequest",
            "notification",
        ],
        AgentKind::Codex | AgentKind::Cursor | AgentKind::Gemini => &[],
    };
    !hooks.is_empty() && hooks.keys().all(|key| allowed.contains(&key.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_skill_destinations_use_shared_agents_directory() {
        let project = Path::new("/work");
        let component = Component {
            name: "demo".into(),
            kind: ComponentKind::Skill,
            source: PathBuf::from("demo"),
            active: false,
            agent_format: None,
        };
        assert_eq!(
            NativeAdapter(AgentKind::Codex)
                .destination(&component, InstallScope::Project, project)
                .unwrap(),
            PathBuf::from("/work/.agents/skills/demo")
        );
        assert_eq!(
            NativeAdapter(AgentKind::Claude)
                .destination(&component, InstallScope::Project, project)
                .unwrap(),
            PathBuf::from("/work/.agents/skills/demo")
        );
        assert_eq!(
            NativeAdapter(AgentKind::Copilot)
                .destination(&component, InstallScope::Project, project)
                .unwrap(),
            PathBuf::from("/work/.agents/skills/demo")
        );
        assert_eq!(
            NativeAdapter(AgentKind::Cursor)
                .destination(&component, InstallScope::Project, project)
                .unwrap(),
            PathBuf::from("/work/.agents/skills/demo")
        );
        assert_eq!(
            NativeAdapter(AgentKind::Gemini)
                .destination(&component, InstallScope::Project, project)
                .unwrap(),
            PathBuf::from("/work/.agents/skills/demo")
        );
    }

    #[test]
    fn project_subagent_destinations_use_native_agent_directories() {
        let project = Path::new("/work");
        let markdown = Component {
            name: "demo".into(),
            kind: ComponentKind::Agent,
            source: PathBuf::from("demo.md"),
            active: false,
            agent_format: Some(AgentFormat::Markdown),
        };
        let toml = Component {
            name: "demo".into(),
            kind: ComponentKind::Agent,
            source: PathBuf::from("demo.toml"),
            active: false,
            agent_format: Some(AgentFormat::CodexToml),
        };

        assert_eq!(
            NativeAdapter(AgentKind::Codex)
                .destination(&toml, InstallScope::Project, project)
                .unwrap(),
            PathBuf::from("/work/.codex/agents/demo.toml")
        );
        assert_eq!(
            NativeAdapter(AgentKind::Claude)
                .destination(&markdown, InstallScope::Project, project)
                .unwrap(),
            PathBuf::from("/work/.claude/agents/demo.md")
        );
        assert_eq!(
            NativeAdapter(AgentKind::Cursor)
                .destination(&markdown, InstallScope::Project, project)
                .unwrap(),
            PathBuf::from("/work/.cursor/agents/demo.md")
        );
        assert_eq!(
            NativeAdapter(AgentKind::Gemini)
                .destination(&markdown, InstallScope::Project, project)
                .unwrap(),
            PathBuf::from("/work/.gemini/agents/demo.md")
        );
        assert!(!NativeAdapter(AgentKind::Codex).supports(&markdown));
        assert!(!NativeAdapter(AgentKind::Claude).supports(&toml));
        assert!(!NativeAdapter(AgentKind::Cursor).supports(&toml));
        assert!(!NativeAdapter(AgentKind::Gemini).supports(&toml));
        assert!(!NativeAdapter(AgentKind::Copilot).supports(&toml));
    }
}
