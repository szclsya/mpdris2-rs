use anyhow::{Context, Result, bail};
use thiserror::Error;

#[derive(Error, Debug)]
#[error("MPD error {code}: {msg}")]
pub struct AckError {
    // We just return the code rather than replicate the enum here
    // See <https://github.com/MusicPlayerDaemon/MPD/blob/master/src/protocol/Ack.hxx>
    pub code: usize,
    msg: String,
    // These are only meaningful when executing a command list
    command_list_no: usize,
    current_command: String,
}

// nom-style parsing primitives
fn expect<'a>(i: &'a str, t: &str) -> Result<&'a str> {
    i.strip_prefix(t).with_context(|| format!("expected {t:?} in mpd line {i:?}"))
}

fn take_till<'a>(i: &'a str, pat: &str) -> Result<(&'a str, &'a str)> {
    let (captured, rest) =
        i.split_once(pat).with_context(|| format!("missing {pat:?} in mpd line {i:?}"))?;
    Ok((rest, captured))
}

/// Parses a single newline-terminated MPD response line into (name, value).
pub fn parse_line(i: &str) -> Result<(&str, &str)> {
    let (i, name) = take_till(i, ": ")?;
    if !name.chars().all(|c| c.is_ascii_alphabetic() || c == '_' || c == '-') {
        bail!("invalid field name {name:?} in mpd line");
    }
    let (_, value) = take_till(i, "\n")?;
    Ok((name, value))
}

/// Parses a single newline-terminated ACK line, e.g. `ACK [50@0] {load} database is locked\n`
pub fn parse_error_line(i: &str) -> Result<AckError> {
    let i = expect(i, "ACK [")?;
    let (i, code) = take_till(i, "@")?;
    let (i, command_list_no) = take_till(i, "] {")?;
    let (i, current_command) = take_till(i, "}")?;
    let (_, msg) = take_till(i, "\n")?;

    Ok(AckError {
        code: code.parse()?,
        msg: msg.trim().to_owned(),
        command_list_no: command_list_no.parse()?,
        current_command: current_command.to_owned(),
    })
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
