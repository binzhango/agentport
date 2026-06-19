use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    Codex,
    Claude,
    Copilot,
}

impl AgentKind {
    pub const ALL: [Self; 3] = [Self::Codex, Self::Claude, Self::Copilot];

    pub fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude Code",
            Self::Copilot => "GitHub Copilot",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallScope {
    Global,
    Project,
}

impl InstallScope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Project => "project",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    Plugin,
    Skill,
    Command,
    Agent,
    Hook,
    Mcp,
}

#[derive(Debug, Clone)]
pub struct Component {
    pub name: String,
    pub kind: ComponentKind,
    pub source: PathBuf,
    pub active: bool,
}

#[derive(Debug, Clone)]
pub struct Artifact {
    pub id: String,
    pub name: String,
    pub root: PathBuf,
    pub components: Vec<Component>,
    pub codex_plugin: Option<CodexPlugin>,
}

#[derive(Debug, Clone)]
pub struct CodexPlugin {
    pub name: String,
    pub marketplace: Option<String>,
    pub marketplace_root: Option<PathBuf>,
    pub has_hooks: bool,
}

impl Artifact {
    pub fn summary(&self) -> String {
        if self.codex_plugin.is_some() {
            let hooks = self
                .codex_plugin
                .as_ref()
                .is_some_and(|plugin| plugin.has_hooks);
            return if hooks {
                "Codex plugin (includes hooks)".into()
            } else {
                "Codex plugin".into()
            };
        }
        if self
            .components
            .iter()
            .any(|component| component.kind == ComponentKind::Hook)
        {
            return "standalone hooks package".into();
        }
        let skills = self
            .components
            .iter()
            .filter(|component| {
                matches!(
                    component.kind,
                    ComponentKind::Skill | ComponentKind::Command
                )
            })
            .count();
        let extras = self.components.len().saturating_sub(skills);
        format!("{skills} skill(s), {extras} extra component(s)")
    }
}

#[derive(Debug, Clone)]
pub struct DetectedAgent {
    pub kind: AgentKind,
    pub evidence: Vec<String>,
    pub home: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledFile {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledTarget {
    pub agent: AgentKind,
    pub scope: InstallScope,
    pub files: Vec<InstalledFile>,
    #[serde(default)]
    pub native_plugins: Vec<InstalledPlugin>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledPlugin {
    pub selector: String,
    pub marketplace: String,
    pub marketplace_source: String,
    pub plugin_owned: bool,
    pub marketplace_owned: bool,
    pub snapshot: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedInstallation {
    pub id: String,
    pub package: String,
    pub source: String,
    pub revision: Option<String>,
    pub installed_at_unix: u64,
    pub active_content_approved: bool,
    pub targets: Vec<InstalledTarget>,
}

#[derive(Debug, Clone)]
pub enum PlannedOperation {
    CopyDirectory {
        from: PathBuf,
        to: PathBuf,
    },
    CopyFile {
        from: PathBuf,
        to: PathBuf,
    },
    InstallCodexPlugin {
        plugin: String,
        marketplace: String,
        marketplace_source: String,
        snapshot_from: Option<PathBuf>,
        snapshot_to: Option<PathBuf>,
        standalone_hook: bool,
        revision: Option<String>,
    },
}

impl PlannedOperation {
    pub fn display(&self) -> String {
        match self {
            Self::CopyDirectory { to, .. } | Self::CopyFile { to, .. } => to.display().to_string(),
            Self::InstallCodexPlugin {
                plugin,
                marketplace,
                marketplace_source,
                ..
            } => {
                format!(
                    "codex plugin add {plugin}@{marketplace} (marketplace: {marketplace_source})"
                )
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct TargetPlan {
    pub agent: AgentKind,
    pub scope: InstallScope,
    pub operations: Vec<PlannedOperation>,
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct InstallPlan {
    pub package: String,
    pub source: String,
    pub revision: Option<String>,
    pub active_content_approved: bool,
    pub targets: Vec<TargetPlan>,
    pub warnings: Vec<String>,
}
