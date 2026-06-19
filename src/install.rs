use crate::adapters::{AgentAdapter, NativeAdapter};
use crate::model::{
    AgentKind, Artifact, InstallPlan, InstallScope, InstalledFile, InstalledTarget,
    ManagedInstallation, PlannedOperation, TargetPlan,
};
use crate::state::StateStore;
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct InstallRequest {
    pub artifact: Artifact,
    pub source: String,
    pub revision: Option<String>,
    pub targets: Vec<(AgentKind, InstallScope)>,
    pub project: PathBuf,
    pub approve_active: bool,
}

pub fn build_plan(request: &InstallRequest, store: &StateStore) -> Result<InstallPlan> {
    if request.targets.is_empty() {
        bail!("select at least one target agent");
    }
    let managed: HashSet<PathBuf> = store
        .list()?
        .into_iter()
        .flat_map(|installation| installation.targets)
        .flat_map(|target| target.files)
        .map(|file| file.path)
        .collect();
    let mut warnings = Vec::new();
    let mut targets = Vec::new();

    for (agent, scope) in &request.targets {
        let adapter = NativeAdapter(*agent);
        let mut operations = Vec::new();
        let mut skipped = Vec::new();
        if *agent == AgentKind::Codex {
            let plugin = request.artifact.codex_plugin.as_ref();
            let standalone_hook = plugin.is_none()
                && request
                    .artifact
                    .components
                    .iter()
                    .any(|c| c.kind == crate::model::ComponentKind::Hook);
            if plugin.is_some() || standalone_hook {
                if *scope != InstallScope::Global {
                    skipped.push("Codex plugins and hooks are global-only".into());
                } else {
                    let active = standalone_hook
                        || request
                            .artifact
                            .components
                            .iter()
                            .any(|c| c.kind == crate::model::ComponentKind::Plugin && c.active);
                    if active && !request.approve_active {
                        skipped
                            .push("plugin hooks/scripts/MCP: active content not approved".into());
                    } else {
                        let plugin_name = plugin
                            .map(|p| p.name.clone())
                            .unwrap_or_else(|| format!("{}-hooks", request.artifact.id));
                        let direct_remote = plugin.and_then(|p| p.marketplace.as_ref()).is_some()
                            && github_repo_root(&request.source).is_some();
                        let source_hash = short_hash(&format!(
                            "{}:{:?}:{}",
                            request.source, request.revision, plugin_name
                        ));
                        let generated_marketplace =
                            format!("agentport-{}-{source_hash}", slug(&plugin_name));
                        let (marketplace, marketplace_source, snapshot_from, snapshot_to) =
                            if direct_remote {
                                (
                                    plugin.unwrap().marketplace.clone().unwrap(),
                                    github_repo_root(&request.source).unwrap(),
                                    None,
                                    None,
                                )
                            } else {
                                let destination = store
                                    .root()
                                    .join("codex-marketplaces")
                                    .join(&generated_marketplace);
                                (
                                    generated_marketplace,
                                    destination.display().to_string(),
                                    Some(request.artifact.root.clone()),
                                    Some(destination),
                                )
                            };
                        operations.push(PlannedOperation::InstallCodexPlugin {
                            plugin: plugin_name,
                            marketplace,
                            marketplace_source,
                            snapshot_from,
                            snapshot_to,
                            standalone_hook,
                            revision: request.revision.clone(),
                        });
                    }
                }
                targets.push(TargetPlan {
                    agent: *agent,
                    scope: *scope,
                    operations,
                    skipped,
                });
                continue;
            }
        }
        for component in &request.artifact.components {
            if component.kind == crate::model::ComponentKind::Plugin {
                continue;
            }
            if component.active && !request.approve_active {
                skipped.push(format!(
                    "{} ({:?}): active content not approved",
                    component.name, component.kind
                ));
                continue;
            }
            if !adapter.supports(component) {
                skipped.push(format!(
                    "{} ({:?}): unsupported by {}",
                    component.name,
                    component.kind,
                    agent.label()
                ));
                continue;
            }
            let Some(destination) = adapter.destination(component, *scope, &request.project) else {
                skipped.push(format!(
                    "{} ({:?}): no safe standalone destination",
                    component.name, component.kind
                ));
                continue;
            };
            if destination.exists() && !managed.contains(&destination) {
                bail!(
                    "refusing to overwrite unmanaged path {}",
                    destination.display()
                );
            }
            if destination.exists() {
                bail!(
                    "{} is already managed; uninstall it before reinstalling",
                    destination.display()
                );
            }
            let operation = if component.source.is_dir() {
                PlannedOperation::CopyDirectory {
                    from: component.source.clone(),
                    to: destination,
                }
            } else if matches!(component.kind, crate::model::ComponentKind::Command) {
                PlannedOperation::CopyFile {
                    from: component.source.clone(),
                    to: destination.join("SKILL.md"),
                }
            } else {
                PlannedOperation::CopyFile {
                    from: component.source.clone(),
                    to: destination,
                }
            };
            operations.push(operation);
        }
        if operations.is_empty() {
            warnings.push(format!(
                "No compatible selected components for {} ({})",
                agent.label(),
                scope.label()
            ));
        }
        targets.push(TargetPlan {
            agent: *agent,
            scope: *scope,
            operations,
            skipped,
        });
    }

    Ok(InstallPlan {
        package: request.artifact.name.clone(),
        source: request.source.clone(),
        revision: request.revision.clone(),
        active_content_approved: request.approve_active,
        targets,
        warnings,
    })
}

pub fn execute_plan(plan: &InstallPlan, store: &StateStore) -> Result<ManagedInstallation> {
    execute_plan_with_runner(plan, store, &ProcessCodexRunner)
}

pub trait CodexCommandRunner {
    fn run(&self, args: &[String]) -> Result<String>;
}

pub struct ProcessCodexRunner;
impl CodexCommandRunner for ProcessCodexRunner {
    fn run(&self, args: &[String]) -> Result<String> {
        let output = Command::new("codex")
            .args(args)
            .output()
            .context("run codex CLI")?;
        if !output.status.success() {
            bail!(
                "codex {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

pub fn execute_plan_with_runner(
    plan: &InstallPlan,
    store: &StateStore,
    runner: &dyn CodexCommandRunner,
) -> Result<ManagedInstallation> {
    if plan
        .targets
        .iter()
        .all(|target| target.operations.is_empty())
    {
        bail!("the install plan contains no file operations");
    }
    let mut committed_roots = Vec::new();
    let mut created_plugins: Vec<(String, String, bool, bool)> = Vec::new();
    let mut installed_targets = Vec::new();

    let execution = (|| -> Result<()> {
        for target in &plan.targets {
            let mut files = Vec::new();
            let mut native_plugins = Vec::new();
            for operation in &target.operations {
                match operation {
                    PlannedOperation::CopyDirectory { from, to } => {
                        if to.exists() {
                            bail!("destination appeared during installation: {}", to.display());
                        }
                        atomic_copy_directory(from, to)?;
                        committed_roots.push(to.clone());
                        files.extend(hash_destination(to)?);
                    }
                    PlannedOperation::CopyFile { from, to } => {
                        if to.exists() {
                            bail!("destination appeared during installation: {}", to.display());
                        }
                        atomic_copy_file(from, to)?;
                        committed_roots.push(to.clone());
                        files.extend(hash_destination(to)?);
                    }
                    PlannedOperation::InstallCodexPlugin {
                        plugin,
                        marketplace,
                        marketplace_source,
                        snapshot_from,
                        snapshot_to,
                        standalone_hook,
                        revision,
                    } => {
                        if let (Some(from), Some(to)) = (snapshot_from, snapshot_to) {
                            if to.exists() {
                                bail!("managed marketplace already exists: {}", to.display());
                            }
                            create_local_marketplace(
                                from,
                                to,
                                plugin,
                                marketplace,
                                *standalone_hook,
                            )?;
                            committed_roots.push(to.clone());
                            files.extend(hash_destination(to)?);
                        }
                        let listing = codex_plugin_listing(runner)?;
                        let selector = format!("{plugin}@{marketplace}");
                        let plugin_preexisting = listing
                            .iter()
                            .any(|p| p.selector == selector && p.installed);
                        let marketplace_preexisting = listing.iter().any(|p| {
                            p.marketplace == *marketplace
                                && same_source(&p.marketplace_source, marketplace_source)
                        });
                        if listing.iter().any(|p| {
                            p.marketplace == *marketplace
                                && !same_source(&p.marketplace_source, marketplace_source)
                        }) {
                            bail!(
                                "Codex marketplace '{marketplace}' already points to a different source"
                            );
                        }
                        if !marketplace_preexisting {
                            let mut args = vec![
                                "plugin".into(),
                                "marketplace".into(),
                                "add".into(),
                                marketplace_source.clone(),
                            ];
                            if snapshot_to.is_none()
                                && let Some(rev) = revision
                            {
                                args.extend(["--ref".into(), rev.clone()]);
                            }
                            args.push("--json".into());
                            runner.run(&args)?;
                        }
                        if !plugin_preexisting
                            && let Err(error) = runner.run(&[
                                "plugin".into(),
                                "add".into(),
                                selector.clone(),
                                "--json".into(),
                            ])
                        {
                            if !marketplace_preexisting {
                                let _ = runner.run(&[
                                    "plugin".into(),
                                    "marketplace".into(),
                                    "remove".into(),
                                    marketplace.clone(),
                                ]);
                            }
                            return Err(error.context("install Codex plugin"));
                        }
                        let verified = codex_plugin_listing(runner)?
                            .iter()
                            .any(|p| p.selector == selector && p.installed);
                        if !verified {
                            if !plugin_preexisting {
                                let _ = runner.run(&[
                                    "plugin".into(),
                                    "remove".into(),
                                    selector.clone(),
                                ]);
                            }
                            if !marketplace_preexisting {
                                let _ = runner.run(&[
                                    "plugin".into(),
                                    "marketplace".into(),
                                    "remove".into(),
                                    marketplace.clone(),
                                ]);
                            }
                            bail!("Codex did not report {selector} as installed");
                        }
                        created_plugins.push((
                            selector.clone(),
                            marketplace.clone(),
                            !plugin_preexisting,
                            !marketplace_preexisting,
                        ));
                        native_plugins.push(crate::model::InstalledPlugin {
                            selector,
                            marketplace: marketplace.clone(),
                            marketplace_source: marketplace_source.clone(),
                            plugin_owned: !plugin_preexisting,
                            marketplace_owned: !marketplace_preexisting,
                            snapshot: snapshot_to.clone(),
                        });
                    }
                }
            }
            installed_targets.push(InstalledTarget {
                agent: target.agent,
                scope: target.scope,
                files,
                native_plugins,
            });
        }
        Ok(())
    })();

    if let Err(error) = execution {
        for path in committed_roots.iter().rev() {
            let _ = remove_path(path);
        }
        for (selector, marketplace, plugin_owned, marketplace_owned) in created_plugins.iter().rev()
        {
            if *plugin_owned {
                let _ = runner.run(&["plugin".into(), "remove".into(), selector.clone()]);
            }
            if *marketplace_owned {
                let _ = runner.run(&[
                    "plugin".into(),
                    "marketplace".into(),
                    "remove".into(),
                    marketplace.clone(),
                ]);
            }
        }
        return Err(error.context("installation rolled back"));
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut id_hasher = Sha256::new();
    id_hasher.update(plan.package.as_bytes());
    id_hasher.update(plan.source.as_bytes());
    id_hasher.update(now.to_le_bytes());
    let id = format!(
        "{}-{}",
        slug(&plan.package),
        hex_bytes(&id_hasher.finalize())
    )
    .chars()
    .take(32)
    .collect();
    let installation = ManagedInstallation {
        id,
        package: plan.package.clone(),
        source: plan.source.clone(),
        revision: plan.revision.clone(),
        installed_at_unix: now,
        active_content_approved: plan.active_content_approved,
        targets: installed_targets,
    };
    if let Err(error) = store.add(installation.clone()) {
        for (selector, marketplace, plugin_owned, marketplace_owned) in created_plugins.iter().rev()
        {
            if *plugin_owned {
                let _ = runner.run(&["plugin".into(), "remove".into(), selector.clone()]);
            }
            if *marketplace_owned {
                let _ = runner.run(&[
                    "plugin".into(),
                    "marketplace".into(),
                    "remove".into(),
                    marketplace.clone(),
                ]);
            }
        }
        for path in committed_roots.iter().rev() {
            let _ = remove_path(path);
        }
        return Err(error.context("could not record installation; installed files rolled back"));
    }
    Ok(installation)
}

#[derive(Default)]
struct ListedPlugin {
    selector: String,
    marketplace: String,
    marketplace_source: String,
    installed: bool,
}
fn codex_plugin_listing(runner: &dyn CodexCommandRunner) -> Result<Vec<ListedPlugin>> {
    let text = runner.run(&[
        "plugin".into(),
        "list".into(),
        "--available".into(),
        "--json".into(),
    ])?;
    let value: serde_json::Value =
        serde_json::from_str(&text).context("parse codex plugin list JSON")?;
    let mut result = Vec::new();
    for key in ["installed", "available"] {
        for item in value
            .get(key)
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            let marketplace = item
                .get("marketplaceName")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned();
            result.push(ListedPlugin {
                selector: item
                    .get("pluginId")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_owned(),
                marketplace,
                marketplace_source: item
                    .get("marketplaceSource")
                    .and_then(|v| v.get("source"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_owned(),
                installed: item
                    .get("installed")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(key == "installed"),
            });
        }
    }
    Ok(result)
}

fn same_source(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    let canonical = |v: &str| fs::canonicalize(v).ok();
    canonical(left).is_some() && canonical(left) == canonical(right)
}

fn create_local_marketplace(
    from: &Path,
    to: &Path,
    plugin: &str,
    marketplace: &str,
    standalone_hook: bool,
) -> Result<()> {
    let plugin_to = to.join("plugins").join(plugin);
    copy_directory(from, &plugin_to)?;
    if standalone_hook {
        let source_hook = plugin_to.join("hooks.json");
        if !source_hook.is_file() {
            bail!("standalone hook package is missing hooks.json");
        }
        fs::create_dir_all(plugin_to.join("hooks"))?;
        fs::copy(&source_hook, plugin_to.join("hooks/hooks.json"))?;
        fs::create_dir_all(plugin_to.join(".codex-plugin"))?;
        fs::write(
            plugin_to.join(".codex-plugin/plugin.json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"name": plugin, "version": "0.0.0", "description": "Agentport-managed standalone hooks"}),
            )?,
        )?;
    }
    fs::create_dir_all(to.join(".agents/plugins"))?;
    fs::write(
        to.join(".agents/plugins/marketplace.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"name": marketplace, "plugins": [{"name": plugin, "source": {"source": "local", "path": format!("./plugins/{plugin}")}, "policy": {"installation": "AVAILABLE", "authentication": "ON_INSTALL"}}]}),
        )?,
    )?;
    Ok(())
}

fn github_repo_root(source: &str) -> Option<String> {
    let url = url::Url::parse(source).ok()?;
    if url.host_str()? != "github.com" {
        return None;
    }
    let segments: Vec<_> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    if segments.len() != 2 {
        return None;
    }
    Some(format!(
        "{}/{}",
        segments[0],
        segments[1].trim_end_matches(".git")
    ))
}

fn short_hash(value: &str) -> String {
    let mut h = Sha256::new();
    h.update(value);
    hex_bytes(&h.finalize())[..10].to_owned()
}

#[derive(Debug, Default)]
pub struct UninstallReport {
    pub removed: Vec<PathBuf>,
    pub preserved: Vec<PathBuf>,
}

pub fn uninstall(store: &StateStore, id: &str) -> Result<UninstallReport> {
    uninstall_with_runner(store, id, &ProcessCodexRunner)
}

pub fn uninstall_with_runner(
    store: &StateStore,
    id: &str,
    runner: &dyn CodexCommandRunner,
) -> Result<UninstallReport> {
    let installation = store
        .list()?
        .into_iter()
        .find(|installation| installation.id == id)
        .with_context(|| format!("no managed installation with id '{id}'"))?;
    let mut report = UninstallReport::default();
    for target in &installation.targets {
        for plugin in &target.native_plugins {
            if plugin.plugin_owned {
                runner.run(&["plugin".into(), "remove".into(), plugin.selector.clone()])?;
            }
            let used_elsewhere = store
                .list()?
                .iter()
                .filter(|i| i.id != installation.id)
                .flat_map(|i| &i.targets)
                .flat_map(|t| &t.native_plugins)
                .any(|p| p.marketplace == plugin.marketplace);
            if plugin.marketplace_owned && !used_elsewhere {
                runner.run(&[
                    "plugin".into(),
                    "marketplace".into(),
                    "remove".into(),
                    plugin.marketplace.clone(),
                ])?;
            } else if plugin.marketplace_owned && used_elsewhere {
                store.transfer_marketplace_ownership(&installation.id, &plugin.marketplace)?;
            }
        }
        for file in &target.files {
            if !file.path.exists() {
                continue;
            }
            if hash_file(&file.path)? == file.sha256 {
                fs::remove_file(&file.path)?;
                report.removed.push(file.path.clone());
                remove_empty_parents(&file.path, &target.files);
            } else {
                report.preserved.push(file.path.clone());
            }
        }
    }
    store.remove(id)?;
    Ok(report)
}

fn atomic_copy_directory(from: &Path, to: &Path) -> Result<()> {
    let parent = to.parent().context("destination has no parent")?;
    fs::create_dir_all(parent)?;
    let stage = parent.join(format!(".agentport-stage-{}", unique_suffix()));
    copy_directory(from, &stage)?;
    fs::rename(&stage, to).with_context(|| format!("commit {}", to.display()))?;
    Ok(())
}

fn atomic_copy_file(from: &Path, to: &Path) -> Result<()> {
    let parent = to.parent().context("destination has no parent")?;
    fs::create_dir_all(parent)?;
    let stage = parent.join(format!(".agentport-stage-{}", unique_suffix()));
    fs::copy(from, &stage)?;
    fs::rename(&stage, to).with_context(|| format!("commit {}", to.display()))?;
    Ok(())
}

fn copy_directory(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in WalkDir::new(from).follow_links(false) {
        let entry = entry?;
        let relative = entry.path().strip_prefix(from)?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        let output = to.join(relative);
        if entry.file_type().is_symlink() {
            bail!(
                "source contains a symbolic link: {}",
                entry.path().display()
            );
        }
        if entry.file_type().is_dir() {
            fs::create_dir_all(&output)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), &output)?;
        }
    }
    Ok(())
}

fn hash_destination(path: &Path) -> Result<Vec<InstalledFile>> {
    if path.is_file() {
        return Ok(vec![InstalledFile {
            path: path.to_path_buf(),
            sha256: hash_file(path)?,
        }]);
    }
    let mut files = Vec::new();
    for entry in WalkDir::new(path)
        .into_iter()
        .filter_map(|entry| entry.ok())
    {
        if entry.file_type().is_file() {
            files.push(InstalledFile {
                path: entry.path().to_path_buf(),
                sha256: hash_file(entry.path())?,
            });
        }
    }
    Ok(files)
}

fn hash_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex_bytes(&hasher.finalize()))
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn remove_path(path: &Path) -> Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)?;
    } else if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn remove_empty_parents(path: &Path, managed_files: &[InstalledFile]) {
    let common_root = managed_files
        .iter()
        .filter_map(|file| file.path.parent())
        .fold(None::<PathBuf>, |common, parent| {
            Some(match common {
                None => parent.to_path_buf(),
                Some(mut value) => {
                    while !parent.starts_with(&value) && value.pop() {}
                    value
                }
            })
        });
    let Some(root) = common_root else { return };
    let mut current = path.parent();
    while let Some(directory) = current {
        if !directory.starts_with(&root) || directory == root.parent().unwrap_or(&root) {
            break;
        }
        if fs::remove_dir(directory).is_err() {
            break;
        }
        if directory == root {
            break;
        }
        current = directory.parent();
    }
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{nanos}", std::process::id())
}

fn slug(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Component, ComponentKind};
    use std::cell::RefCell;

    struct FakeRunner {
        outputs: RefCell<Vec<String>>,
    }
    impl CodexCommandRunner for FakeRunner {
        fn run(&self, _args: &[String]) -> Result<String> {
            Ok(self.outputs.borrow_mut().remove(0))
        }
    }

    #[test]
    fn uninstall_preserves_modified_file() {
        let temp = tempfile::tempdir().unwrap();
        let state = StateStore::new(temp.path().join("state"));
        let installed = temp.path().join("installed/SKILL.md");
        fs::create_dir_all(installed.parent().unwrap()).unwrap();
        fs::write(&installed, "before").unwrap();
        let installation = ManagedInstallation {
            id: "demo".into(),
            package: "demo".into(),
            source: "local".into(),
            revision: None,
            installed_at_unix: 0,
            active_content_approved: false,
            targets: vec![InstalledTarget {
                agent: AgentKind::Codex,
                scope: InstallScope::Global,
                files: vec![InstalledFile {
                    path: installed.clone(),
                    sha256: hash_file(&installed).unwrap(),
                }],
                native_plugins: vec![],
            }],
        };
        state.add(installation).unwrap();
        fs::write(&installed, "changed").unwrap();
        let report = uninstall(&state, "demo").unwrap();
        assert_eq!(report.preserved, vec![installed]);
    }

    #[test]
    fn plan_excludes_unapproved_active_skill() {
        let temp = tempfile::tempdir().unwrap();
        let skill = temp.path().join("demo");
        fs::create_dir(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "demo").unwrap();
        let request = InstallRequest {
            artifact: Artifact {
                id: "demo".into(),
                name: "demo".into(),
                root: skill.clone(),
                components: vec![Component {
                    name: "demo".into(),
                    kind: ComponentKind::Skill,
                    source: skill,
                    active: true,
                }],
                codex_plugin: None,
            },
            source: "local".into(),
            revision: None,
            targets: vec![(AgentKind::Codex, InstallScope::Project)],
            project: temp.path().join("project"),
            approve_active: false,
        };
        let plan = build_plan(&request, &StateStore::new(temp.path().join("state"))).unwrap();
        assert!(plan.targets[0].operations.is_empty());
    }

    #[test]
    fn project_install_and_uninstall_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let skill = temp.path().join("source/demo");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "demo").unwrap();
        fs::write(skill.join("reference.md"), "reference").unwrap();
        let state = StateStore::new(temp.path().join("state"));
        let project = temp.path().join("project");
        let request = InstallRequest {
            artifact: Artifact {
                id: "demo".into(),
                name: "demo".into(),
                root: skill.clone(),
                components: vec![Component {
                    name: "demo".into(),
                    kind: ComponentKind::Skill,
                    source: skill,
                    active: false,
                }],
                codex_plugin: None,
            },
            source: "local".into(),
            revision: None,
            targets: vec![(AgentKind::Codex, InstallScope::Project)],
            project: project.clone(),
            approve_active: false,
        };
        let plan = build_plan(&request, &state).unwrap();
        let installed = execute_plan(&plan, &state).unwrap();
        let destination = project.join(".agents/skills/demo");
        assert!(destination.join("SKILL.md").is_file());
        assert!(destination.join("reference.md").is_file());
        let report = uninstall(&state, &installed.id).unwrap();
        assert_eq!(report.removed.len(), 2);
        assert!(!destination.exists());
        assert!(state.list().unwrap().is_empty());
    }

    #[test]
    fn codex_plugin_plan_uses_native_global_operation() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("plugin");
        fs::create_dir_all(root.join(".codex-plugin")).unwrap();
        fs::write(root.join(".codex-plugin/plugin.json"), r#"{"name":"demo"}"#).unwrap();
        let artifact = Artifact {
            id: "demo".into(),
            name: "demo".into(),
            root: root.clone(),
            components: vec![Component {
                name: "demo".into(),
                kind: ComponentKind::Plugin,
                source: root,
                active: false,
            }],
            codex_plugin: Some(crate::model::CodexPlugin {
                name: "demo".into(),
                marketplace: None,
                marketplace_root: None,
                has_hooks: false,
            }),
        };
        let request = InstallRequest {
            artifact,
            source: "local".into(),
            revision: None,
            targets: vec![(AgentKind::Codex, InstallScope::Global)],
            project: temp.path().into(),
            approve_active: false,
        };
        let plan = build_plan(&request, &StateStore::new(temp.path().join("state"))).unwrap();
        assert!(matches!(
            plan.targets[0].operations[0],
            PlannedOperation::InstallCodexPlugin { .. }
        ));
    }

    #[test]
    fn codex_plugin_project_scope_is_skipped() {
        let temp = tempfile::tempdir().unwrap();
        let artifact = Artifact {
            id: "demo".into(),
            name: "demo".into(),
            root: temp.path().into(),
            components: vec![Component {
                name: "demo".into(),
                kind: ComponentKind::Plugin,
                source: temp.path().into(),
                active: false,
            }],
            codex_plugin: Some(crate::model::CodexPlugin {
                name: "demo".into(),
                marketplace: None,
                marketplace_root: None,
                has_hooks: false,
            }),
        };
        let request = InstallRequest {
            artifact,
            source: "local".into(),
            revision: None,
            targets: vec![(AgentKind::Codex, InstallScope::Project)],
            project: temp.path().into(),
            approve_active: false,
        };
        let plan = build_plan(&request, &StateStore::new(temp.path().join("state"))).unwrap();
        assert!(plan.targets[0].operations.is_empty());
        assert!(plan.targets[0].skipped[0].contains("global-only"));
    }

    #[test]
    fn parses_codex_plugin_list_json() {
        let runner = FakeRunner { outputs: RefCell::new(vec![r#"{"installed":[{"pluginId":"demo@market","marketplaceName":"market","installed":true,"marketplaceSource":{"source":"/tmp/market"}}],"available":[]}"#.into()]) };
        let plugins = codex_plugin_listing(&runner).unwrap();
        assert_eq!(plugins[0].selector, "demo@market");
        assert_eq!(plugins[0].marketplace_source, "/tmp/market");
        assert!(plugins[0].installed);
    }

    #[test]
    fn wraps_standalone_hooks_as_local_plugin() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("market");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("hooks.json"), r#"{"hooks":{"Stop":[]}}"#).unwrap();
        create_local_marketplace(&source, &destination, "demo-hooks", "agentport-demo", true)
            .unwrap();
        assert!(
            destination
                .join("plugins/demo-hooks/.codex-plugin/plugin.json")
                .is_file()
        );
        assert!(
            destination
                .join("plugins/demo-hooks/hooks/hooks.json")
                .is_file()
        );
        let value: serde_json::Value = serde_json::from_slice(
            &fs::read(destination.join(".agents/plugins/marketplace.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(value["name"], "agentport-demo");
        assert_eq!(
            value["plugins"][0]["source"]["path"],
            "./plugins/demo-hooks"
        );
    }

    #[test]
    fn formats_digest_bytes_as_lowercase_hex() {
        assert_eq!(hex_bytes(&[0x00, 0x0f, 0xa5, 0xff]), "000fa5ff");
    }
}
