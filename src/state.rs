use crate::model::ManagedInstallation;
use anyhow::{Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    schema_version: u32,
    installations: Vec<ManagedInstallation>,
}

#[derive(Debug, Clone)]
pub struct StateStore {
    root: PathBuf,
}

impl StateStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn discover() -> Result<Self> {
        let root = dirs::data_dir()
            .or_else(|| dirs::home_dir().map(|home| home.join(".local/share")))
            .context("could not determine a user data directory")?
            .join("agentport");
        Ok(Self::new(root))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn list(&self) -> Result<Vec<ManagedInstallation>> {
        Ok(self.load()?.installations)
    }

    pub fn add(&self, installation: ManagedInstallation) -> Result<()> {
        self.mutate(|state| {
            state.installations.push(installation);
            Ok(())
        })
    }

    pub fn remove(&self, id: &str) -> Result<Option<ManagedInstallation>> {
        let mut removed = None;
        self.mutate(|state| {
            if let Some(index) = state.installations.iter().position(|item| item.id == id) {
                removed = Some(state.installations.remove(index));
            }
            Ok(())
        })?;
        Ok(removed)
    }

    pub fn transfer_marketplace_ownership(
        &self,
        excluding_id: &str,
        marketplace: &str,
    ) -> Result<bool> {
        let mut transferred = false;
        self.mutate(|state| {
            if let Some(plugin) = state
                .installations
                .iter_mut()
                .filter(|item| item.id != excluding_id)
                .flat_map(|item| &mut item.targets)
                .flat_map(|target| &mut target.native_plugins)
                .find(|plugin| plugin.marketplace == marketplace)
            {
                plugin.marketplace_owned = true;
                transferred = true;
            }
            Ok(())
        })?;
        Ok(transferred)
    }

    fn load(&self) -> Result<State> {
        let path = self.root.join("state.json");
        if !path.exists() {
            return Ok(State {
                schema_version: 2,
                installations: Vec::new(),
            });
        }
        serde_json::from_slice(&fs::read(&path)?)
            .with_context(|| format!("parse {}", path.display()))
    }

    fn mutate(&self, operation: impl FnOnce(&mut State) -> Result<()>) -> Result<()> {
        fs::create_dir_all(&self.root)?;
        let lock_path = self.root.join("state.lock");
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(lock_path)?;
        lock.lock_exclusive().context("lock Agentport state")?;
        let mut state = self.load()?;
        operation(&mut state)?;
        state.schema_version = 2;
        let bytes = serde_json::to_vec_pretty(&state)?;
        let temp_path = self.root.join("state.json.tmp");
        let final_path = self.root.join("state.json");
        let mut temp = File::create(&temp_path)?;
        temp.write_all(&bytes)?;
        temp.sync_all()?;
        fs::rename(&temp_path, &final_path)?;
        FileExt::unlock(&lock)?;
        Ok(())
    }
}
