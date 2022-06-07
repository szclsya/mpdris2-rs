mod error;
pub use error::*;

use anyhow::{bail, Result};
use nom::{
    bytes::complete::{tag, take_till, take_while},
    AsChar, Err, IResult,
};

pub fn parse_line(i: &str) -> Result<(&str, &str)> {
    let res = match parse_line_helper(i) {
        Ok(res) => res,
        Err(e) => match e {
            Err::Incomplete(_) => bail!("incomplete mpd line"),
            Err::Error(e) => {
                let pos = i.len() - e.input.len();
                bail!(
                    "parse line failed at {}: {} ({})",
                    pos,
                    e.code.description(),
                    i
                )
            }
            Err::Failure(e) => bail!("internal error while parsing mpd line: {e}"),
        },
    };

    Ok(res.1)
}

fn parse_line_helper(i: &str) -> IResult<&str, (&str, &str)> {
    let (i, name) = take_while(is_field_name_char)(i)?;
    let (i, _) = tag(": ")(i)?;
    let (i, value) = take_till(|c| c == '\n')(i)?;
    let (i, _) = tag("\n")(i)?;

    Ok((i, (name, value)))
}

fn is_field_name_char(c: char) -> bool {
    c.is_alpha() || c == '_' || c == '-'
}
