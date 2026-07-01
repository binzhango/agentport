use crate::model::{AgentFormat, Artifact, CodexPlugin, Component, ComponentKind};
use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub trait ArtifactScanner {
    fn scan(&self, root: &Path) -> Result<Vec<Artifact>>;
}

#[derive(Default)]
pub struct DefaultScanner;

impl ArtifactScanner for DefaultScanner {
    fn scan(&self, root: &Path) -> Result<Vec<Artifact>> {
        if let Some(artifact) = discover_harness_package(root) {
            return Ok(vec![artifact]);
        }

        let mut roots = discover_artifact_roots(root)?;
        if roots.is_empty() && contains_components(root) {
            roots.push(root.to_path_buf());
        }

        let mut artifacts = Vec::new();
        let mut seen = HashSet::new();
        for artifact_root in roots {
            let canonical = artifact_root
                .canonicalize()
                .unwrap_or_else(|_| artifact_root.clone());
            if !seen.insert(canonical) {
                continue;
            }
            validate_manifests(&artifact_root)?;
            let components = discover_components(&artifact_root)?;
            if components.is_empty() {
                continue;
            }
            let name = manifest_name(&artifact_root).unwrap_or_else(|| {
                artifact_root
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("package")
                    .to_owned()
            });
            let codex_plugin = codex_plugin_metadata(root, &artifact_root, &name)?;
            for component in components
                .iter()
                .filter(|component| component.kind == ComponentKind::Agent)
            {
                artifacts.push(Artifact {
                    id: format!("{}--agent--{}", slug(&name), slug(&component.name)),
                    name: component.name.clone(),
                    root: component.source.clone(),
                    components: vec![component.clone()],
                    codex_plugin: None,
                });
            }
            if codex_plugin.is_some() {
                for component in components.iter().filter(|component| {
                    matches!(
                        component.kind,
                        ComponentKind::Skill | ComponentKind::Command
                    )
                }) {
                    artifacts.push(Artifact {
                        id: format!("{}--{}", slug(&name), slug(&component.name)),
                        name: component.name.clone(),
                        root: component.source.clone(),
                        components: vec![component.clone()],
                        codex_plugin: None,
                    });
                }
            }
            let grouped_components = components
                .into_iter()
                .filter(|component| {
                    component.kind != ComponentKind::Agent
                        && (codex_plugin.is_none()
                            || !matches!(
                                component.kind,
                                ComponentKind::Skill | ComponentKind::Command
                            ))
                })
                .collect::<Vec<_>>();
            if grouped_components.is_empty() && codex_plugin.is_none() {
                continue;
            }
            artifacts.push(Artifact {
                id: slug(&name),
                name,
                root: artifact_root,
                components: grouped_components,
                codex_plugin,
            });
        }

        if artifacts.is_empty()
            && let Some(artifact) = discover_direct_subagent_package(root)?
        {
            artifacts.push(artifact);
        }

        if artifacts.is_empty() {
            // A directory containing only direct skill children is a collection.
            for entry in fs::read_dir(root).context("scan source directory")? {
                let path = entry?.path();
                if path.join("SKILL.md").is_file() {
                    let name = path
                        .file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or("skill");
                    artifacts.push(Artifact {
                        id: slug(name),
                        name: name.to_owned(),
                        root: path.clone(),
                        components: vec![component_for_skill(&path)],
                        codex_plugin: None,
                    });
                }
            }
        }
        artifacts.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(artifacts)
    }
}

fn discover_artifact_roots(root: &Path) -> Result<Vec<PathBuf>> {
    let mut roots = Vec::new();
    if has_plugin_manifest(root) || root.join("SKILL.md").is_file() {
        roots.push(root.to_path_buf());
    }

    for marketplace in [
        root.join(".agents/plugins/marketplace.json"),
        root.join(".claude-plugin/marketplace.json"),
        root.join(".codex-plugin/marketplace.json"),
        root.join(".github/plugin/marketplace.json"),
        root.join("marketplace.json"),
    ] {
        if !marketplace.is_file() {
            continue;
        }
        let value: Value = serde_json::from_slice(&fs::read(&marketplace)?)
            .with_context(|| format!("parse {}", marketplace.display()))?;
        if let Some(plugins) = value.get("plugins").and_then(Value::as_array) {
            let mut ids = HashSet::new();
            for plugin in plugins {
                let name = plugin
                    .get("name")
                    .and_then(Value::as_str)
                    .context("marketplace plugin is missing a string name")?;
                if !ids.insert(name) {
                    anyhow::bail!("duplicate plugin id '{name}' in {}", marketplace.display());
                }
                if let Some(source) = marketplace_plugin_path(plugin) {
                    if !source.starts_with("./") {
                        anyhow::bail!("local marketplace source must start with './': {source}");
                    }
                    let candidate = root.join(source);
                    let canonical_root = root.canonicalize()?;
                    let canonical = candidate.canonicalize().with_context(|| {
                        format!(
                            "marketplace plugin path does not exist: {}",
                            candidate.display()
                        )
                    })?;
                    if !canonical.starts_with(&canonical_root) {
                        anyhow::bail!("marketplace plugin path escapes its root: {source}");
                    }
                    roots.push(candidate);
                }
            }
        }
    }

    for folder in ["plugins", "plugin"] {
        let directory = root.join(folder);
        if directory.is_dir() {
            for entry in fs::read_dir(directory)? {
                let path = entry?.path();
                if path.is_dir() && contains_components(&path) {
                    roots.push(path);
                }
            }
        }
    }
    Ok(roots)
}

fn marketplace_plugin_path(plugin: &Value) -> Option<&str> {
    let source = plugin.get("source")?;
    source
        .as_str()
        .or_else(|| source.get("path").and_then(Value::as_str))
}

fn codex_plugin_metadata(
    source_root: &Path,
    artifact_root: &Path,
    name: &str,
) -> Result<Option<CodexPlugin>> {
    let manifest = artifact_root.join(".codex-plugin/plugin.json");
    if !manifest.is_file() {
        return Ok(None);
    }
    let mut marketplace_name = None;
    let mut marketplace_root = None;
    for relative in [
        ".agents/plugins/marketplace.json",
        ".codex-plugin/marketplace.json",
        "marketplace.json",
    ] {
        let path = source_root.join(relative);
        if !path.is_file() {
            continue;
        }
        let value: Value = serde_json::from_slice(&fs::read(&path)?)?;
        let plugins = value
            .get("plugins")
            .and_then(Value::as_array)
            .context("marketplace plugins must be an array")?;
        if plugins
            .iter()
            .any(|p| p.get("name").and_then(Value::as_str) == Some(name))
        {
            marketplace_name = value
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    source_root
                        .file_name()
                        .and_then(|v| v.to_str())
                        .map(str::to_owned)
                });
            marketplace_root = Some(source_root.to_path_buf());
            break;
        }
    }
    Ok(Some(CodexPlugin {
        name: name.to_owned(),
        marketplace: marketplace_name,
        marketplace_root,
        has_hooks: artifact_root.join("hooks/hooks.json").is_file()
            || artifact_root.join("hooks.json").is_file(),
    }))
}

fn contains_components(path: &Path) -> bool {
    has_plugin_manifest(path)
        || path.join("SKILL.md").is_file()
        || path.join("skills").is_dir()
        || path.join("commands").is_dir()
        || path.join("agents").is_dir()
        || path.join(".claude/agents").is_dir()
        || path.join(".cursor/agents").is_dir()
        || path.join(".gemini/agents").is_dir()
        || path.join(".codex/agents").is_dir()
        || path.join("codex/agents").is_dir()
        || is_harness_package(path)
        || path.join("hooks.json").is_file()
        || path.join("hooks/hooks.json").is_file()
        || path.join(".mcp.json").is_file()
}

fn has_plugin_manifest(path: &Path) -> bool {
    [
        path.join("plugin.json"),
        path.join(".claude-plugin/plugin.json"),
        path.join(".codex-plugin/plugin.json"),
        path.join(".github/plugin.json"),
    ]
    .iter()
    .any(|manifest| manifest.is_file())
}

fn validate_manifests(root: &Path) -> Result<()> {
    for path in [
        root.join("plugin.json"),
        root.join(".claude-plugin/plugin.json"),
        root.join(".codex-plugin/plugin.json"),
        root.join(".github/plugin.json"),
    ] {
        if !path.is_file() {
            continue;
        }
        let value: Value = serde_json::from_slice(&fs::read(&path)?)
            .with_context(|| format!("parse plugin manifest {}", path.display()))?;
        if !value.is_object() {
            anyhow::bail!(
                "plugin manifest {} must contain a JSON object",
                path.display()
            );
        }
        if value.get("name").is_some_and(|name| !name.is_string()) {
            anyhow::bail!("plugin manifest {} has a non-string name", path.display());
        }
    }
    Ok(())
}

fn manifest_name(root: &Path) -> Option<String> {
    for path in [
        root.join(".claude-plugin/plugin.json"),
        root.join(".codex-plugin/plugin.json"),
        root.join("plugin.json"),
        root.join(".github/plugin.json"),
    ] {
        let Ok(bytes) = fs::read(path) else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        if let Some(name) = value.get("name").and_then(Value::as_str) {
            return Some(name.to_owned());
        }
    }
    None
}

fn discover_components(root: &Path) -> Result<Vec<Component>> {
    let mut components = BTreeMap::<(String, String), Component>::new();
    if root.join(".codex-plugin/plugin.json").is_file() {
        let name = manifest_name(root).unwrap_or_else(|| "plugin".into());
        components.insert(
            ("plugin".into(), name.clone()),
            Component {
                name,
                kind: ComponentKind::Plugin,
                source: root.to_path_buf(),
                active: contains_executable_content(root)
                    || root.join("hooks/hooks.json").is_file()
                    || root.join("hooks.json").is_file()
                    || root.join(".mcp.json").is_file(),
                agent_format: None,
            },
        );
    }
    if root.join("SKILL.md").is_file() {
        let component = component_for_skill(root);
        components.insert(("skill".into(), component.name.clone()), component);
    }

    let skills = root.join("skills");
    if skills.is_dir() {
        for entry in fs::read_dir(&skills)? {
            let path = entry?.path();
            if path.is_dir() && path.join("SKILL.md").is_file() {
                let component = component_for_skill(&path);
                components.insert(("skill".into(), component.name.clone()), component);
            }
        }
    }

    let commands = root.join("commands");
    if commands.is_dir() {
        for entry in fs::read_dir(commands)? {
            let path = entry?.path();
            if path.extension().and_then(|value| value.to_str()) == Some("md") {
                let name = file_stem(&path);
                components.insert(
                    ("command".into(), name.clone()),
                    Component {
                        name,
                        kind: ComponentKind::Command,
                        source: path,
                        active: false,
                        agent_format: None,
                    },
                );
            }
        }
    }

    discover_agent_files(
        root,
        &mut components,
        &[
            ("agents", "md", AgentFormat::Markdown),
            (".claude/agents", "md", AgentFormat::Markdown),
            (".cursor/agents", "md", AgentFormat::Markdown),
            (".gemini/agents", "md", AgentFormat::Markdown),
            (".codex/agents", "toml", AgentFormat::CodexToml),
            ("codex/agents", "toml", AgentFormat::CodexToml),
        ],
    )?;

    for hook in [root.join("hooks.json"), root.join("hooks/hooks.json")] {
        if hook.is_file() {
            let name = format!(
                "{}-hooks",
                root.file_name()
                    .and_then(|v| v.to_str())
                    .unwrap_or("package")
            );
            components.insert(
                ("hook".into(), name.clone()),
                Component {
                    name,
                    kind: ComponentKind::Hook,
                    source: hook,
                    active: true,
                    agent_format: None,
                },
            );
        }
    }
    for mcp in [root.join(".mcp.json"), root.join(".github/mcp.json")] {
        if mcp.is_file() {
            components.insert(
                ("mcp".into(), "mcp-servers".into()),
                Component {
                    name: "mcp-servers".into(),
                    kind: ComponentKind::Mcp,
                    source: mcp,
                    active: true,
                    agent_format: None,
                },
            );
        }
    }
    Ok(components.into_values().collect())
}

fn component_for_skill(path: &Path) -> Component {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("skill")
        .to_owned();
    Component {
        name,
        kind: ComponentKind::Skill,
        source: path.to_path_buf(),
        active: contains_executable_content(path),
        agent_format: None,
    }
}

fn discover_agent_files(
    root: &Path,
    components: &mut BTreeMap<(String, String), Component>,
    directories: &[(&str, &str, AgentFormat)],
) -> Result<()> {
    for (directory, extension, format) in directories {
        let agents = root.join(directory);
        if !agents.is_dir() {
            continue;
        }
        for entry in fs::read_dir(agents)? {
            let path = entry?.path();
            if path.extension().and_then(|value| value.to_str()) == Some(*extension) {
                let name = file_stem(&path).trim_end_matches(".agent").to_owned();
                components.insert(
                    (format!("agent-{:?}", format).to_lowercase(), name.clone()),
                    component_for_agent(path, *format),
                );
            }
        }
    }
    Ok(())
}

fn discover_direct_subagent_package(root: &Path) -> Result<Option<Artifact>> {
    let mut candidates = Vec::new();
    for entry in fs::read_dir(root).context("scan source directory")? {
        let path = entry?.path();
        if !path.is_file() || is_documentation_file(&path) {
            continue;
        }
        match path.extension().and_then(|value| value.to_str()) {
            Some("md") if looks_like_markdown_subagent(&path)? => {
                candidates.push(component_for_agent(path, AgentFormat::Markdown));
            }
            Some("toml") if looks_like_codex_subagent(&path)? => {
                candidates.push(component_for_agent(path, AgentFormat::CodexToml));
            }
            _ => {}
        }
    }
    if candidates.len() != 1 {
        return Ok(None);
    }
    let component = candidates.remove(0);
    Ok(Some(Artifact {
        id: format!("agent--{}", slug(&component.name)),
        name: component.name.clone(),
        root: component.source.clone(),
        components: vec![component],
        codex_plugin: None,
    }))
}

fn discover_harness_package(root: &Path) -> Option<Artifact> {
    if !is_harness_package(root) {
        return None;
    }
    let name = harness_agent_name(root);
    let component = Component {
        name: name.clone(),
        kind: ComponentKind::Agent,
        source: root.to_path_buf(),
        active: contains_executable_content(root),
        agent_format: Some(AgentFormat::Harness),
    };
    Some(Artifact {
        id: format!("harness--{}", slug(&name)),
        name,
        root: root.to_path_buf(),
        components: vec![component],
        codex_plugin: None,
    })
}

fn is_harness_package(path: &Path) -> bool {
    path.join("AGENTS.md").is_file()
        && path.join(".harness").is_dir()
        && path.join(".harness/Skills").is_dir()
}

fn harness_agent_name(root: &Path) -> String {
    fs::read_to_string(root.join("AGENTS.md"))
        .ok()
        .and_then(|content| frontmatter_field(&content, "name"))
        .unwrap_or_else(|| "harness".to_owned())
}

fn frontmatter_field(markdown: &str, key: &str) -> Option<String> {
    let rest = markdown.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    for line in rest[..end].lines() {
        if let Some((field, value)) = line.split_once(':')
            && field.trim() == key
        {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    None
}

fn component_for_agent(path: PathBuf, format: AgentFormat) -> Component {
    let name = file_stem(&path).trim_end_matches(".agent").to_owned();
    Component {
        name,
        kind: ComponentKind::Agent,
        source: path,
        active: false,
        agent_format: Some(format),
    }
}

fn is_documentation_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        name.as_str(),
        "readme.md" | "changelog.md" | "contributing.md" | "security.md" | "license.md"
    )
}

fn looks_like_markdown_subagent(path: &Path) -> Result<bool> {
    let text = fs::read_to_string(path)?;
    let mut lines = text.lines();
    if lines.next() != Some("---") {
        return Ok(false);
    }
    let mut has_name = false;
    let mut has_description = false;
    for line in lines {
        if line == "---" {
            break;
        }
        has_name |= line.trim_start().starts_with("name:");
        has_description |= line.trim_start().starts_with("description:");
    }
    Ok(has_name && has_description)
}

fn looks_like_codex_subagent(path: &Path) -> Result<bool> {
    let text = fs::read_to_string(path)?;
    Ok(text.contains("developer_instructions") && text.contains("description"))
}

fn contains_executable_content(path: &Path) -> bool {
    const ACTIVE_EXTENSIONS: &[&str] = &[
        "sh", "bash", "zsh", "py", "js", "mjs", "ts", "rb", "pl", "exe",
    ];
    WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .any(|entry| {
            entry.file_type().is_file()
                && entry
                    .path()
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|extension| ACTIVE_EXTENSIONS.contains(&extension))
        })
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("component")
        .to_owned()
}

fn slug(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            output.push(character.to_ascii_lowercase());
        } else if !output.ends_with('-') {
            output.push('-');
        }
    }
    output.trim_matches('-').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_multi_skill_repository() {
        let temp = tempfile::tempdir().unwrap();
        for name in ["one", "two"] {
            let path = temp.path().join("skills").join(name);
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("SKILL.md"), format!("---\nname: {name}\n---\n")).unwrap();
        }
        let artifacts = DefaultScanner.scan(temp.path()).unwrap();
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].components.len(), 2);
    }

    #[test]
    fn marks_scripted_skill_active() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("SKILL.md"), "hello").unwrap();
        fs::write(temp.path().join("run.sh"), "echo hi").unwrap();
        let artifacts = DefaultScanner.scan(temp.path()).unwrap();
        assert!(artifacts[0].components[0].active);
    }

    #[test]
    fn scans_marketplace_plugins_individually() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".claude-plugin")).unwrap();
        fs::write(
            temp.path().join(".claude-plugin/marketplace.json"),
            r#"{"plugins":[{"name":"one","source":"./plugins/one"},{"name":"two","source":"./plugins/two"}]}"#,
        )
        .unwrap();
        for name in ["one", "two"] {
            let path = temp
                .path()
                .join("plugins")
                .join(name)
                .join("skills")
                .join(name);
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("SKILL.md"), format!("---\nname: {name}\n---\n")).unwrap();
        }
        let artifacts = DefaultScanner.scan(temp.path()).unwrap();
        assert_eq!(artifacts.len(), 2);
        assert_eq!(artifacts[0].name, "one");
        assert_eq!(artifacts[1].name, "two");
    }

    #[test]
    fn rejects_malformed_plugin_manifest() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("skills/demo")).unwrap();
        fs::write(temp.path().join("skills/demo/SKILL.md"), "demo").unwrap();
        fs::write(temp.path().join("plugin.json"), "not json").unwrap();
        assert!(DefaultScanner.scan(temp.path()).is_err());
    }

    #[test]
    fn scans_codex_marketplace_object_source() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".agents/plugins")).unwrap();
        fs::create_dir_all(temp.path().join("plugins/demo/.codex-plugin")).unwrap();
        fs::write(
            temp.path().join(".agents/plugins/marketplace.json"),
            r#"{"name":"local-demo","plugins":[{"name":"demo","source":{"source":"local","path":"./plugins/demo"}}]}"#,
        ).unwrap();
        fs::write(
            temp.path().join("plugins/demo/.codex-plugin/plugin.json"),
            r#"{"name":"demo","version":"1.0.0"}"#,
        )
        .unwrap();
        let artifacts = DefaultScanner.scan(temp.path()).unwrap();
        assert_eq!(artifacts.len(), 1);
        let plugin = artifacts[0].codex_plugin.as_ref().unwrap();
        assert_eq!(plugin.name, "demo");
        assert_eq!(plugin.marketplace.as_deref(), Some("local-demo"));
        assert!(
            artifacts[0]
                .components
                .iter()
                .any(|c| c.kind == ComponentKind::Plugin)
        );
    }

    #[test]
    fn exposes_codex_plugin_skills_as_standalone_artifacts() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".codex-plugin")).unwrap();
        fs::create_dir_all(temp.path().join("skills/ponytail")).unwrap();
        fs::create_dir_all(temp.path().join("skills/ponytail-review")).unwrap();
        fs::write(
            temp.path().join(".codex-plugin/plugin.json"),
            r#"{"name":"ponytail"}"#,
        )
        .unwrap();
        fs::write(temp.path().join("skills/ponytail/SKILL.md"), "ponytail").unwrap();
        fs::write(
            temp.path().join("skills/ponytail-review/SKILL.md"),
            "review",
        )
        .unwrap();

        let artifacts = DefaultScanner.scan(temp.path()).unwrap();

        assert_eq!(artifacts.len(), 3);
        assert_eq!(
            artifacts
                .iter()
                .filter(|artifact| artifact.codex_plugin.is_some())
                .count(),
            1
        );
        let standalone = artifacts
            .iter()
            .filter(|artifact| artifact.codex_plugin.is_none())
            .collect::<Vec<_>>();
        assert_eq!(standalone.len(), 2);
        assert!(standalone.iter().all(|artifact| {
            artifact.components.len() == 1 && artifact.components[0].kind == ComponentKind::Skill
        }));
        let plugin = artifacts
            .iter()
            .find(|artifact| artifact.codex_plugin.is_some())
            .unwrap();
        assert!(
            plugin
                .components
                .iter()
                .all(|component| component.kind != ComponentKind::Skill)
        );
    }

    #[test]
    fn scans_codex_toml_subagents_as_individual_artifacts() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".codex/agents")).unwrap();
        for name in ["harness-util", "reviewer"] {
            fs::write(
                temp.path()
                    .join(".codex/agents")
                    .join(format!("{name}.toml")),
                format!(
                    "name = \"{name}\"\ndescription = \"demo\"\ndeveloper_instructions = \"demo\""
                ),
            )
            .unwrap();
        }

        let artifacts = DefaultScanner.scan(temp.path()).unwrap();

        assert_eq!(artifacts.len(), 2);
        assert!(artifacts.iter().all(|artifact| {
            artifact.components.len() == 1
                && artifact.components[0].kind == ComponentKind::Agent
                && artifact.components[0].agent_format == Some(AgentFormat::CodexToml)
        }));
    }

    #[test]
    fn scans_markdown_subagents_as_individual_artifacts() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("agents")).unwrap();
        for name in ["planner", "tester"] {
            fs::write(
                temp.path().join("agents").join(format!("{name}.md")),
                format!("---\nname: {name}\ndescription: demo\n---\nDemo"),
            )
            .unwrap();
        }

        let artifacts = DefaultScanner.scan(temp.path()).unwrap();

        assert_eq!(artifacts.len(), 2);
        assert!(artifacts.iter().all(|artifact| {
            artifact.components.len() == 1
                && artifact.components[0].kind == ComponentKind::Agent
                && artifact.components[0].agent_format == Some(AgentFormat::Markdown)
        }));
    }

    #[test]
    fn scans_tool_native_subagent_folders() {
        let temp = tempfile::tempdir().unwrap();
        for folder in [".claude/agents", ".cursor/agents", ".gemini/agents"] {
            fs::create_dir_all(temp.path().join(folder)).unwrap();
            fs::write(
                temp.path().join(folder).join(format!(
                    "{}.md",
                    folder.split('/').next().unwrap().trim_start_matches('.')
                )),
                "---\nname: demo\ndescription: demo\n---\nDemo",
            )
            .unwrap();
        }
        fs::create_dir_all(temp.path().join("codex/agents")).unwrap();
        fs::write(
            temp.path().join("codex/agents/harness.toml"),
            "name = \"harness\"\ndescription = \"demo\"\ndeveloper_instructions = \"demo\"",
        )
        .unwrap();

        let artifacts = DefaultScanner.scan(temp.path()).unwrap();

        assert_eq!(artifacts.len(), 4);
        assert_eq!(
            artifacts
                .iter()
                .filter(
                    |artifact| artifact.components[0].agent_format == Some(AgentFormat::Markdown)
                )
                .count(),
            3
        );
        assert_eq!(
            artifacts
                .iter()
                .filter(
                    |artifact| artifact.components[0].agent_format == Some(AgentFormat::CodexToml)
                )
                .count(),
            1
        );
    }

    #[test]
    fn scans_harness_repo_as_one_agent_package() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "# harness_util\n").unwrap();
        fs::write(temp.path().join("AGENTS.md"), "# Harness agent\n").unwrap();
        fs::create_dir_all(temp.path().join(".harness/Skills")).unwrap();
        for name in ["coding-skill", "expert-reviewer", "unit-test-write"] {
            fs::write(
                temp.path()
                    .join(".harness/Skills")
                    .join(format!("{name}.md")),
                format!("# {name}\n\nHarness procedure without frontmatter."),
            )
            .unwrap();
        }

        let artifacts = DefaultScanner.scan(temp.path()).unwrap();

        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].name, "harness");
        assert_eq!(artifacts[0].summary(), "Harness agent package");
        assert_eq!(artifacts[0].components.len(), 1);
        assert_eq!(artifacts[0].components[0].kind, ComponentKind::Agent);
        assert_eq!(
            artifacts[0].components[0].agent_format,
            Some(AgentFormat::Harness)
        );
        assert_eq!(artifacts[0].components[0].source, temp.path());
    }

    #[test]
    fn harness_agent_name_can_come_from_agents_frontmatter() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("AGENTS.md"),
            "---\nname: delivery-harness\n---\n# Harness agent\n",
        )
        .unwrap();
        fs::create_dir_all(temp.path().join(".harness/Skills")).unwrap();

        let artifacts = DefaultScanner.scan(temp.path()).unwrap();

        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].name, "delivery-harness");
    }

    #[test]
    fn scans_direct_single_file_markdown_subagent_repo() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "# demo").unwrap();
        fs::write(
            temp.path().join("harness_util.md"),
            "---\nname: harness_util\ndescription: Harness helper\n---\nDemo",
        )
        .unwrap();

        let artifacts = DefaultScanner.scan(temp.path()).unwrap();

        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].name, "harness_util");
        assert_eq!(
            artifacts[0].components[0].agent_format,
            Some(AgentFormat::Markdown)
        );
    }

    #[test]
    fn rejects_marketplace_path_escape() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("root");
        fs::create_dir_all(root.join(".agents/plugins")).unwrap();
        fs::create_dir_all(parent.path().join("outside/.codex-plugin")).unwrap();
        fs::write(
            parent.path().join("outside/.codex-plugin/plugin.json"),
            r#"{"name":"bad"}"#,
        )
        .unwrap();
        fs::write(root.join(".agents/plugins/marketplace.json"), r#"{"name":"bad","plugins":[{"name":"bad","source":{"source":"local","path":"./../outside"}}]}"#).unwrap();
        assert!(DefaultScanner.scan(&root).is_err());
    }
}
