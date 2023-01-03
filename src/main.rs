mod config;
mod mpd;
mod plugins;
mod types;

const RETRY_INTERVAL: Duration = Duration::from_secs(5);

use anyhow::Result;
use async_std::{
    prelude::*,
    sync::{Arc, Mutex},
    task,
};
use clap::Parser;
use colored::Colorize;
use fern::colors::{Color, ColoredLevelConfig};
use log::{debug, error, info};
use signal_hook::consts::signal::{SIGTERM, SIGINT, SIGQUIT};
use signal_hook_async_std::Signals;
use std::time::Duration;

fn main() {
    // We don't really need multiple worker thread to pass some music info
    std::env::set_var("ASYNC_STD_THREAD_COUNT", "1");

    if let Err(err) = async_std::task::block_on(try_main()) {
        println!("{:>6} {err}", "ERROR".red());
        err.chain().skip(1).for_each(|cause| {
            println!("{} {}", "DUE TO".yellow(), cause);
        });
    }
}

async fn try_main() -> Result<()> {
    let args = config::Args::parse();
    setup_logger(args.verbose)?;

    let mut first_retry = true;
    let mpd_state_server = loop {
        match mpd::MpdStateServer::init(&args.host, args.port).await {
            Ok(c) => break c,
            Err(e) => {
                if first_retry {
                    error!("Failed to connect to MPD server: {e}. Will try again every 5 secs...");
                    first_retry = false;
                } else {
                    debug!("Retry failed.");
                }
                task::sleep(RETRY_INTERVAL).await;
            }
        }
    };

    let mpd_state_server = Arc::new(Mutex::new(mpd_state_server));

    // Always need MPRIS2
    let connection = plugins::mpris2::start(mpd_state_server.clone()).await?;

    // Set up notification relay, if requested
    if !args.no_notification {
        info!("Notification enabled, starting notification sender...");
        plugins::fdo_notification::start(&connection, mpd_state_server.clone()).await?;
    }

    // Broadcast MPD server state change
    mpd_state_server.lock().await.ready().await?;

    // Now everything is set-up, wait for an exit signal
    info!("Service started.");

    let mut signals = Signals::new([SIGTERM, SIGINT, SIGQUIT])?;
    let handle = signals.handle();
    if let Some(_signal) = signals.next().await {
        info!("Exit signal received, closing D-Bus connection");
        handle.close();
    }

    Ok(())
}

fn setup_logger(debug: u8) -> Result<()> {
    let colors = ColoredLevelConfig::new()
        .error(Color::Red)
        .warn(Color::Yellow)
        .info(Color::Blue);

    let level = match debug {
        1 => log::LevelFilter::Debug,
        2 => log::LevelFilter::Trace,
        _ => log::LevelFilter::Info,
    };

    fern::Dispatch::new()
        .format(move |out, message, record| {
            out.finish(format_args!(
                "{:>5} [{}] {}",
                colors.color(record.level()),
                record.target(),
                message
            ));
        })
        .level(level)
        .chain(std::io::stdout())
        .apply()?;
    Ok(())
}
