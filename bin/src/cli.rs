use clap::Parser;
use std::path::PathBuf;

mod run;

#[derive(Parser, Debug)]
#[clap(name = "network-watchtower", version, rename_all = "kebab-case")]
pub struct Opt {
    #[clap(subcommand)]
    command: Fuel,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Parser)]
pub enum Fuel {
    Run(run::Command),
}

fn init_environment() -> Option<PathBuf> {
    dotenvy::dotenv().ok()
}

pub async fn run_cli() -> anyhow::Result<()> {
    fuel_core_bin::cli::init_logging();
    if let Some(path) = init_environment() {
        let path = path.display();
        tracing::info!("Loading environment variables from {path}");
    }
    let opt = Opt::try_parse();

    match opt {
        Ok(opt) => match opt.command {
            Fuel::Run(command) => run::exec(command).await,
        },
        Err(e) => {
            // Prints the error and exits.
            e.exit()
        }
    }
}
