#![doc = include_str!("../README.md")]

pub mod adapters;
pub mod install;
pub mod model;
pub mod scanner;
pub mod source;
pub mod state;

pub use adapters::{AgentAdapter, detect_agents};
pub use install::{
    CodexCommandRunner, InstallRequest, build_plan, execute_plan, execute_plan_with_runner,
    uninstall, uninstall_with_runner,
};
pub use model::*;
pub use scanner::{ArtifactScanner, DefaultScanner};
pub use source::{DefaultSourceProvider, PreparedSource, SourceProvider};
pub use state::StateStore;
