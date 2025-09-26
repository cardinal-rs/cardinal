pub mod cmd;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Parser)]
pub struct CmdRun {
    #[arg(long, short)]
    config: Vec<String>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Run(CmdRun),
}
