/// A simple MPD client implementation
use super::{parse_error_line, parse_line, types::MpdResponse};
use crate::types::MpdConnectionConfig;

use anyhow::{bail, Context, Result};
use log::{debug, error, info, trace};
use std::sync::Arc;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, BufWriter},
    net::{tcp, unix, TcpStream, UnixStream},
    time::sleep,
};

enum MpdConnection {
    Tcp((BufReader<tcp::OwnedReadHalf>, BufWriter<tcp::OwnedWriteHalf>)),
    Socket((BufReader<unix::OwnedReadHalf>, BufWriter<unix::OwnedWriteHalf>)),
}

impl MpdConnection {
    pub async fn connect(config: &MpdConnectionConfig) -> Result<Self> {
        let res = match config {
            MpdConnectionConfig::Tcp(s) => {
                let stream = TcpStream::connect(s)
                    .await
                    .context(format!("Cannot connect to MPD server with TCP at {s}"))?;
                let (r, w) = stream.into_split();
                let (r, w) = (BufReader::new(r), BufWriter::new(w));
                MpdConnection::Tcp((r, w))
            }
            MpdConnectionConfig::Socket(path) => {
                let stream = UnixStream::connect(path).await.context(format!(
                    "Cannot connect to MPD server with socket at {}",
                    path.display()
                ))?;
                let (r, w) = stream.into_split();
                let (r, w) = (BufReader::new(r), BufWriter::new(w));
                MpdConnection::Socket((r, w))
            }
        };

        Ok(res)
    }

    pub async fn read_line(&mut self, buf: &mut String) -> std::io::Result<usize> {
        match self {
            MpdConnection::Tcp((r, _)) => r.read_line(buf).await,
            MpdConnection::Socket((r, _)) => r.read_line(buf).await,
        }
    }

    pub async fn read_exact(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            MpdConnection::Tcp((r, _)) => r.read_exact(buf).await,
            MpdConnection::Socket((r, _)) => r.read_exact(buf).await,
        }
    }

    // Come with flush!
    pub async fn write_all(&mut self, src: &[u8]) -> std::io::Result<()> {
        match self {
            MpdConnection::Tcp((_, w)) => {
                w.write_all(src).await?;
                w.flush().await?;
            }
            MpdConnection::Socket((_, w)) => {
                w.write_all(src).await?;
                w.flush().await?;
            }
        }
        Ok(())
    }
}

pub struct MpdClient {
    config: Arc<MpdConnectionConfig>,
    connection: MpdConnection,
}

impl MpdClient {
    pub async fn new(config: Arc<MpdConnectionConfig>) -> Result<Self> {
        let connection = MpdConnection::connect(config.as_ref()).await?;
        let mut res = MpdClient { config, connection };

        // Read version info
        let mut hello = String::new();
        res.connection.read_line(&mut hello).await?;
        // Increase binary chunk size
        res.issue_command("binarylimit 524288").await?;

        Ok(res)
    }

    async fn reconnect(&mut self) -> Result<()> {
        // Create a new connection based on the config we saved
        let mut new_connection = MpdConnection::connect(self.config.as_ref()).await?;
        let mut hello = String::new();
        new_connection.read_line(&mut hello).await?;
        self.connection = new_connection;
        // Increase binary chunk size
        self.issue_command("binarylimit 524288").await?;
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
        trace!("Issuing command to MPD: {}", cmd);
        let mut real_cmd = cmd.to_owned();
        real_cmd.push('\n');

        self.connection.write_all(real_cmd.as_bytes()).await?;

        let resp = self.read_response().await?;
        trace!("Command {} returned", cmd);
        Ok(resp)
    }

    async fn read_response(&mut self) -> Result<MpdResponse> {
        let mut fields: Vec<(String, String)> = Vec::new();
        let mut binary: Option<Vec<u8>> = None;

        let mut buf = String::new();
        loop {
            self.connection.read_line(&mut buf).await?;
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
                self.connection.read_exact(res.as_mut_slice()).await?;
                binary = Some(res);
                // Read newline
                let mut newline = [0];
                self.connection.read_exact(&mut newline).await?;
                // Read the last `OK` message
                let mut buf = String::new();
                self.connection.read_line(&mut buf).await?;
                if !buf.starts_with("OK") {
                    bail!("Expecting OK after binary chunk, got {}", buf);
                }
                break;
            }
            buf.clear();
        }

        Ok(MpdResponse { fields, binary })
    }
}
