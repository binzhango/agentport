use crate::model::{Artifact, CodexPlugin, Component, ComponentKind};
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
            artifacts.push(Artifact {
                id: slug(&name),
                name,
                root: artifact_root,
                components,
                codex_plugin,
            });
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
                    },
                );
            }
        }
    }

    let agents = root.join("agents");
    if agents.is_dir() {
        for entry in fs::read_dir(agents)? {
            let path = entry?.path();
            if path.extension().and_then(|value| value.to_str()) == Some("md") {
                let name = file_stem(&path).trim_end_matches(".agent").to_owned();
                components.insert(
                    ("agent".into(), name.clone()),
                    Component {
                        name,
                        kind: ComponentKind::Agent,
                        source: path,
                        active: false,
                    },
                );
            }
        }
    }

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
    }
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
