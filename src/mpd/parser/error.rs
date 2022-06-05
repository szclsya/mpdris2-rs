use anyhow::{bail, Result};
use nom::{
    bytes::complete::{tag, take_till},
    character::complete::*,
    AsChar, Err, IResult, InputTakeAtPosition,
};
use thiserror::Error;

#[derive(Error, Debug)]
#[error("MPD respond with an error: {msg} (source)")]
pub struct MpdError {
    source: MpdErrorType,
    msg: String,
    // These are only meaningful when executing a command list
    command_list_no: usize,
    current_command: String,
}

/// See https://github.com/MusicPlayerDaemon/MPD/blob/master/src/protocol/Ack.hxx
#[derive(Error, Debug)]
pub enum MpdErrorType {
    #[error("Unknown error from MPD: {0}")]
    Unknown(usize),
    #[error("error 2: no_list")]
    NotList, // Whatever this is...
    #[error("bad argument")]
    BadArgument,
    #[error("bad password")]
    BadPassword,
    #[error("permission denied")]
    Permission,
    #[error("resource doesn't exist")]
    NoExist,
}

impl From<usize> for MpdErrorType {
    fn from(e: usize) -> Self {
        match e {
            1 => MpdErrorType::NotList,
            2 => MpdErrorType::BadArgument,
            3 => MpdErrorType::BadPassword,
            4 => MpdErrorType::Permission,
            50 => MpdErrorType::NoExist,
            _ => MpdErrorType::Unknown(e),
        }
    }
}

pub fn parse_error_line(i: &str) -> Result<MpdError> {
    let res = match parse_error_line_helper(i) {
        Ok(res) => res,
        Err(e) => match e {
            Err::Incomplete(_) => bail!("incomplete mpd error line"),
            Err::Error(e) => {
                let pos = i.len() - e.input.len();
                bail!("error at {pos} ({i})")
            }
            _ => bail!("internal error while parsing mpd error line"),
        },
    };

    let (_, (error_id, command_no, current_command, msg)) = res;
    let error_type: MpdErrorType = error_id.parse::<usize>()?.into();
    let res = MpdError {
        source: error_type,
        msg: msg.to_owned(),
        command_list_no: command_no.parse()?,
        current_command: current_command.to_owned(),
    };
    Ok(res)
}

fn parse_error_line_helper(i: &str) -> IResult<&str, (&str, &str, &str, &str)> {
    let (i, _) = tag("ACK")(i)?;
    let (i, _) = space1(i)?;

    // Parse [x@x] block
    let (i, _) = char('[')(i)?;
    let (i, error_id) = digit1(i)?;
    let (i, _) = char('@')(i)?;
    let (i, command_list_no) = digit1(i)?;
    let (i, _) = char(']')(i)?;

    // Parse current command block
    let (i, _) = char('{')(i)?;
    let (i, current_command) = command(i)?;
    let (i, _) = char('}')(i)?;

    // The rest of the line is the error message
    let (i, msg) = take_till(|c| c == '\n')(i)?;
    let (_, _) = tag("\n")(i)?;

    Ok(("", (error_id, msg, command_list_no, current_command)))
}

fn command(input: &str) -> IResult<&str, &str> {
    input.split_at_position_complete(|c| !is_command_char(c))
}

fn is_command_char(c: char) -> bool {
    c.is_alpha() || c == '_'
}
