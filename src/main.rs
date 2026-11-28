mod config;
mod mpd;
mod plugins;
mod types;
use types::MpdConnectionConfig;

const RETRY_INTERVAL: Duration = Duration::from_secs(5);

use anyhow::Result;
use colored::Colorize;
use futures_util::stream::StreamExt;
use log::{Level, LevelFilter, Metadata, Record, debug, error, info};
use signal_hook::consts::signal::{SIGINT, SIGQUIT, SIGTERM};
use signal_hook_tokio::Signals;
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::{runtime, time::sleep};

const DEFAULT_MPD_HOST: &str = "localhost:6600";
// ASCII code of "mpdris2-rs" added together
const XXHASH3_64_SEED: u64 = 979;

fn main() {
    #[cfg(feature = "tokio-console")]
    console_subscriber::init();

    let rt = match runtime::Builder::new_current_thread().enable_io().enable_time().build() {
        Ok(rt) => rt,
        Err(e) => {
            error!("Cannot initialize tokio runtime: {e}");
            std::process::exit(1);
        }
    };
    if let Err(err) = rt.block_on(try_main()) {
        eprintln!("{:>6} {err}", "ERROR".red());
        err.chain().skip(1).for_each(|cause| {
            eprintln!("{} {}", "DUE TO".yellow(), cause);
        });
    }
}

async fn try_main() -> Result<()> {
    let args: config::Args = argh::from_env();
    let level = if args.trace {
        LevelFilter::Trace
    } else if args.verbose {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    };
    log::set_boxed_logger(Box::new(SimpleLogger(level))).map(|()| log::set_max_level(level))?;

    // Configure how to connect to MPD
    let connection_config = match &args.host {
        Some(s) => {
            info!("Connecting to specified MPD server: {s}");
            parse_host_string(s)
        }
        None => {
            if let Ok(s) = std::env::var("MPD_HOST") {
                info!("Connecting to MPD_HOST: {s}");
                parse_host_string(&s)
            } else {
                info!("Connecting to default MPD server: {DEFAULT_MPD_HOST}");
                MpdConnectionConfig::Tcp(DEFAULT_MPD_HOST.to_owned())
            }
        }
    };

    let mut first_retry = true;
    let mpd_state_server = loop {
        match mpd::MpdStateServer::init(connection_config.clone()).await {
            Ok(c) => break c,
            Err(e) => {
                if first_retry {
                    error!("Failed to connect to MPD server: {e}. Will try again every 5 secs...");
                    first_retry = false;
                } else {
                    debug!("Retry failed.");
                }
                sleep(RETRY_INTERVAL).await;
            }
        }
    };

    mpd_state_server.full_update_status().await?;
    let mpd_state_server = Arc::new(mpd_state_server);

    // Always need MPRIS2
    let (connection, _notifier_task) = plugins::mpris2::start(mpd_state_server.clone()).await?;

    // Set up notification relay, if requested
    let _notification_task = if !args.no_notification {
        info!("Notification enabled, starting notification sender...");
        let task =
            plugins::fdo_notification::start(&connection, mpd_state_server.clone(), args).await?;
        Some(task)
    } else {
        info!("Notification disabled.");
        None
    };

    // Broadcast MPD server state change
    mpd_state_server.ready().await?;

    // Now everything is set-up, wait for an exit signal
    info!("Service started.");
    let mut signals = Signals::new([SIGTERM, SIGINT, SIGQUIT])?;
    let handle = signals.handle();
    if let Some(_signal) = signals.next().await {
        info!("Exit signal received, closing D-Bus connection");
        handle.close();
        mpd_state_server.cleanup().await?;
    }

    Ok(())
}

struct SimpleLogger(LevelFilter);

impl log::Log for SimpleLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.0
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let level = match record.level() {
            Level::Error => "ERROR".red(),
            Level::Warn => "WARN".yellow(),
            Level::Info => "INFO".blue(),
            Level::Debug => "DEBUG".cyan(),
            Level::Trace => "TRACE".white(),
        };
        let dest: &mut dyn std::io::Write = match record.level() {
            Level::Error => &mut std::io::stderr().lock(),
            _ => &mut std::io::stdout().lock(),
        };
        let _ = writeln!(dest, "{:>5} [{}] {}", level, record.target(), record.args());
    }

    fn flush(&self) {}
}

fn parse_host_string(s: &str) -> MpdConnectionConfig {
    if s.starts_with('@') {
        // Abstract socket - @ prefix will be stripped when connecting
        MpdConnectionConfig::AbstractSocket(s.to_owned())
    } else if s.starts_with('/') {
        // UNIX socket
        let path = PathBuf::from(s);
        MpdConnectionConfig::Socket(path)
    } else {
        MpdConnectionConfig::Tcp(s.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_host_string_tcp() {
        assert!(matches!(
            parse_host_string("localhost:6600"),
            MpdConnectionConfig::Tcp(s) if s == "localhost:6600"
        ));

        assert!(matches!(
            parse_host_string("192.168.1.1:6600"),
            MpdConnectionConfig::Tcp(s) if s == "192.168.1.1:6600"
        ));
    }

    #[test]
    fn test_parse_host_string_socket() {
        assert!(matches!(
            parse_host_string("/var/run/mpd/socket"),
            MpdConnectionConfig::Socket(path) if path == PathBuf::from("/var/run/mpd/socket")
        ));

        assert!(matches!(
            parse_host_string("/tmp/mpd.sock"),
            MpdConnectionConfig::Socket(path) if path == PathBuf::from("/tmp/mpd.sock")
        ));
    }

    #[test]
    fn test_parse_host_string_abstract_socket() {
        assert!(matches!(
            parse_host_string("@mpd_socket"),
            MpdConnectionConfig::AbstractSocket(s) if s == "@mpd_socket"
        ));

        assert!(matches!(
            parse_host_string("@/tmp/mpd"),
            MpdConnectionConfig::AbstractSocket(s) if s == "@/tmp/mpd"
        ));
    }
}
