use anyhow::{Result, bail};
use nom::{
    AsChar, Err, IResult, Input,
    bytes::complete::{tag, take_till},
    character::complete::{char, digit1, space1},
};
use thiserror::Error;

#[derive(Error, Debug)]
#[error("MPD error {code}: {msg}")]
pub struct MpdError {
    // We just return the code rather than replicate the enum here
    // See <https://github.com/MusicPlayerDaemon/MPD/blob/master/src/protocol/Ack.hxx>
    pub code: usize,
    msg: String,
    // These are only meaningful when executing a command list
    command_list_no: usize,
    current_command: String,
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
            Err::Failure(e) => bail!("internal error while parsing mpd error line: {e}"),
        },
    };

    let (_, (error_id, command_no, current_command, msg)) = res;
    let res = MpdError {
        code: error_id.parse()?,
        msg: msg.trim().to_owned(),
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

    // The command block is separated from the error code block by a space
    let (i, _) = space1(i)?;

    // Parse current command block
    let (i, _) = char('{')(i)?;
    let (i, current_command) = command(i)?;
    let (i, _) = char('}')(i)?;

    // The rest of the line is the error message
    let (i, msg) = take_till(|c| c == '\n')(i)?;
    let (_, _) = tag("\n")(i)?;

    Ok(("", (error_id, command_list_no, current_command, msg)))
}

fn command(input: &str) -> IResult<&str, &str> {
    input.split_at_position_complete(|c| !is_command_char(c))
}

fn is_command_char(c: char) -> bool {
    c.is_alpha() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_error_line() {
        let cases: &[(&str, usize, usize, &str, &str)] = &[
            ("ACK [50@0] {load} database is locked\n", 50, 0, "load", "database is locked"),
            // MPD's doc/protocol.rst example: `play` is the 2nd command (0-indexed).
            ("ACK [2@1] {play} Bad song index\n", 2, 1, "play", "Bad song index"),
            // `{}` is legal: AllCommands.cxx rejects unknown commands before
            // SetCommand has run (src/command/AllCommands.cxx).
            ("ACK [5@0] {} unknown command \"foo\"\n", 5, 0, "", "unknown command \"foo\""),
        ];
        for (line, code, command_list_no, command, msg) in cases {
            let e = parse_error_line(line).unwrap_or_else(|e| panic!("{line}: {e}"));
            assert_eq!(
                (e.code, e.command_list_no, e.current_command.as_str(), e.msg.as_str()),
                (*code, *command_list_no, *command, *msg),
                "{line}"
            );
        }
    }

    #[test]
    fn test_display_and_invalid_lines() {
        let e = parse_error_line("ACK [2@0] {play} Bad song id\n").unwrap();
        assert_eq!(e.to_string(), "MPD error 2: Bad song id");
        assert!(parse_error_line("volume: 50\n").is_err(), "not an ACK line");
        assert!(parse_error_line("ACK [2@0] {add} bad").is_err(), "needs a trailing newline");
    }
}
