mod config;
mod mpd;
mod plugins;
mod types;

const RETRY_INTERVAL: Duration = Duration::from_secs(5);

use anyhow::Result;
use colored::Colorize;
use fern::colors::{Color, ColoredLevelConfig};
use futures_util::stream::StreamExt;
use log::{debug, error, info};
use signal_hook::consts::signal::{SIGINT, SIGQUIT, SIGTERM};
use signal_hook_tokio::Signals;
use std::{sync::Arc, time::Duration};
use tokio::{runtime, sync::Mutex, time::sleep};

fn main() {
    let rt = match runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            error!("Cannot initialize tokio runtime: {e}");
            std::process::exit(1);
        }
    };
    if let Err(err) = rt.block_on(try_main()) {
        println!("{:>6} {err}", "ERROR".red());
        err.chain().skip(1).for_each(|cause| {
            println!("{} {}", "DUE TO".yellow(), cause);
        });
    }
}

async fn try_main() -> Result<()> {
    let args: config::Args = argh::from_env();
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
                sleep(RETRY_INTERVAL).await;
            }
        }
    };

    let mpd_state_server = Arc::new(Mutex::new(mpd_state_server));

    // Always need MPRIS2
    let (connection, _notifier_task) = plugins::mpris2::start(mpd_state_server.clone()).await?;

    // Set up notification relay, if requested
    let _notification_task = if !args.no_notification {
        info!("Notification enabled, starting notification sender...");
        let task = plugins::fdo_notification::start(&connection, mpd_state_server.clone()).await?;
        Some(task)
    } else {
        info!("Notification disabled.");
        None
    };

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
