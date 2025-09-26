use cardinal_cli::cmd::run::run_cmd;
use cardinal_cli::{Cli, Command};
use cardinal_errors::CardinalError;
use clap::Parser;

fn main() {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cmd: Result<(), CardinalError> = match cli.command {
        None => Ok(()),
        Some(cmd) => match cmd {
            Command::Run(run_options) => run_cmd(run_options),
        },
    };

    if let Err(e) = cmd {
        eprintln!("{}", e);
        std::process::exit(1);
    }
}
