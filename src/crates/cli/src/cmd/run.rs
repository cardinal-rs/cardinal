use crate::CmdRun;
use cardinal::Cardinal;
use cardinal_errors::CardinalError;

pub fn run_cmd(cmd: CmdRun) -> Result<(), CardinalError> {
    let cardinal = Cardinal::from_paths(&cmd.config)?;
    cardinal.run()?;
    Ok(())
}
