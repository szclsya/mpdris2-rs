/// A simple MPD client implementation
use super::{parse_error_line, parse_line, types::MpdResponse};

use anyhow::{bail, Context, Result};
use log::{debug, error, info};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, BufWriter},
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpStream,
    },
    time::sleep,
};

pub struct MpdClient {
    reader: BufReader<OwnedReadHalf>,
    writer: BufWriter<OwnedWriteHalf>,

    // MPD info
    ip: String,
    port: u32,
}

impl MpdClient {
    pub async fn new(ip: &str, port: u32) -> Result<Self> {
        let stream = TcpStream::connect(format!("{}:{}", ip, port))
            .await
            .context(format!("Cannot connect to MPD server at {ip}:{port}"))?;
        let (r, w) = stream.into_split();
        let mut reader = BufReader::new(r);
        let writer = BufWriter::new(w);
        // Read version info
        let mut hello = String::new();
        reader.read_line(&mut hello).await?;

        Ok(MpdClient {
            ip: ip.to_owned(),
            port,
            reader,
            writer,
        })
    }

    async fn reconnect(&mut self) -> Result<()> {
        let stream = TcpStream::connect(format!("{}:{}", self.ip, self.port))
            .await
            .context(format!(
                "Cannot reconnect to MPD server at {}:{}",
                self.ip, self.port
            ))?;
        let (r, w) = stream.into_split();
        self.reader = BufReader::new(r);
        self.writer = BufWriter::new(w);

        let mut hello = String::new();
        self.reader.read_line(&mut hello).await?;

        Ok(())
    }

    pub async fn reconnect_until_success(&mut self) {
        error!("MPD connection broken, attempting reconnect...");
        let mut first_retry = true;
        loop {
            match self.reconnect().await {
                Ok(_) => {
                    info!("Reconnect success.");
                    break;
                }
                Err(e) => {
                    if first_retry {
                        error!("Reconnect failed: {}", e);
                        error!("Will reattempt every 5s...");
                        first_retry = false;
                    } else {
                        debug!("Reconnect failed");
                    }

                    sleep(crate::RETRY_INTERVAL).await;
                }
            }
        }
    }

    /// Issue command to MPD server and wait for response.
    /// Returns when response has been received and parsed.
    pub async fn issue_command(&mut self, cmd: &str) -> Result<MpdResponse> {
        debug!("Issuing command to MPD: {}", cmd);
        let mut real_cmd = cmd.to_owned();
        real_cmd.push('\n');

        self.writer.write_all(real_cmd.as_bytes()).await?;
        self.writer.flush().await?;

        let resp = read_response(&mut self.reader).await?;
        debug!("Command {} returned", cmd);
        Ok(resp)
    }
}

async fn read_response(r: &mut BufReader<OwnedReadHalf>) -> Result<MpdResponse> {
    let mut fields: Vec<(String, String)> = Vec::new();
    let mut binary: Option<Vec<u8>> = None;

    let mut buf = String::new();
    loop {
        r.read_line(&mut buf).await?;
        if buf.starts_with("OK") {
            // Response ends here
            break;
        } else if buf.starts_with("ACK") {
            // We encountered an error
            let e = parse_error_line(&buf)?;
            return Err(anyhow::Error::from(e));
        }

        // It's a normal line. Parse it.
        let (name, value) = parse_line(&buf)?;
        fields.push((name.to_owned(), value.to_owned()));

        if name == "binary" {
            // We are receiving a binary chunk
            let len: u64 = value.parse()?;
            let mut res = vec![0u8; len as usize];
            r.read_exact(res.as_mut_slice()).await?;
            binary = Some(res);
            // Read newline
            let mut newline = [0];
            r.read_exact(&mut newline).await?;
            // Read the last `OK` message
            let mut buf = String::new();
            r.read_line(&mut buf).await?;
            if !buf.starts_with("OK") {
                bail!("Expecting OK after binary chunk, got {}", buf);
            }
            break;
        }
        buf.clear();
    }

    Ok(MpdResponse { fields, binary })
}
