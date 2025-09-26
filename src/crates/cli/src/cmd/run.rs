use crate::CmdRun;
use cardinal_errors::CardinalError;
use cardinal_rs::Cardinal;

pub fn run_cmd(cmd: CmdRun) -> Result<(), CardinalError> {
    let cardinal = Cardinal::from_paths(&cmd.config)?;
    cardinal.run()?;
    Ok(())
}
