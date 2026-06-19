mod tui;

use agentport::StateStore;
use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "agentport",
    version,
    about = "Install agent skills and plugins across AI coding agents"
)]
struct Cli {
    /// GitHub URL, local directory, ZIP, or tar.gz to install.
    source: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// List Agentport-managed installations.
    List,
    /// Safely uninstall a managed package.
    Uninstall {
        /// Installation ID or package name to preselect.
        package: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let store = StateStore::discover()?;
    match cli.command {
        Some(Command::List) => {
            let installations = store.list()?;
            if installations.is_empty() {
                println!("No Agentport-managed installations.");
            } else {
                for installation in installations {
                    println!(
                        "{}\t{}\t{}\t{} target(s)",
                        installation.id,
                        installation.package,
                        installation.source,
                        installation.targets.len()
                    );
                }
            }
            Ok(())
        }
        Some(Command::Uninstall { package }) => tui::run_uninstall(package, &store),
        None => tui::run_installer(cli.source, &store),
    }
}
