mod interfaces;
mod mpd;
mod notification;
use notification::NotificationRelay;
mod config;
use config::Config;

const BUS_NAME: &str = "org.mpris.MediaPlayer2.mpd";
const OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";

use anyhow::{Context, Result};
use async_std::{
    fs::File,
    prelude::*,
    sync::{Arc, Mutex},
    task,
};
use colored::Colorize;
use fern::colors::{Color, ColoredLevelConfig};
use log::{error, info};
use signal_hook::consts::signal::*;
use signal_hook_async_std::Signals;
use std::{path::PathBuf, time::Duration};
use zbus::{export::futures_util::SinkExt, ConnectionBuilder};

fn main() -> Result<()> {
    // We don't really need multiple worker thread to pass some music info
    std::env::set_var("ASYNC_STD_THREAD_COUNT", "1");

    if let Err(err) = async_std::task::block_on(try_main()) {
        println!("{:>6} {err}", "ERROR".red());
        err.chain().skip(1).for_each(|cause| {
            println!("{} {}", "DUE TO".yellow(), cause);
        });
    }

    Ok(())
}

async fn try_main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let config: Config = if args.len() >= 2 {
        let config_path = PathBuf::from(&args[1]);
        let mut config_file = File::open(&config_path)
            .await
            .context("Failed to open config file")?;
        let mut config_content = String::new();
        config_file
            .read_to_string(&mut config_content)
            .await
            .context("Failed to read config file")?;
        toml::from_str(&config_content).context("Failed to parse config file")?
    } else {
        Config::default()
    };
    setup_logger(config.debug)?;

    let mpd_state_server = mpd::MpdStateServer::init(&config.host, config.port).await?;

    let mpd_state_server = Arc::new(Mutex::new(mpd_state_server));
    let root_interface = interfaces::RootInterface::default();
    let player_interface = interfaces::PlayerInterface::new(mpd_state_server.clone()).await;
    let tracklist_interface = interfaces::TracklistInterface::new(mpd_state_server.clone());

    let mut connection = ConnectionBuilder::session()?
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, root_interface)?
        .serve_at(OBJECT_PATH, player_interface)?
        .serve_at(OBJECT_PATH, tracklist_interface)?
        .build()
        .await?;

    // Register state change updater
    let c2 = connection.clone();
    let client2 = mpd_state_server.clone();
    let mut rx2 = mpd_state_server.lock().await.get_mpris_event_rx();
    task::spawn(async move {
        loop {
            if let Err(e) = interfaces::notify_loop(&c2, &mut rx2, &client2).await {
                error!("D-Bus property change notifier dead, restarting. Reason: {e}");
            }
        }
    });

    // Set up notification relay, if requested
    if config.notification {
        let mut notification =
            NotificationRelay::new(&connection, mpd_state_server.clone()).await?;
        task::spawn(async move {
            loop {
                if let Err(e) = notification.send_notification_on_event().await {
                    error!("NotificationRelay dead, restarting. Reason: {e}");
                    task::sleep(Duration::from_secs(5)).await;
                }
            }
        });
    }

    // Now everything is set-up, wait for an exit signal
    info!("Server started");

    let mut signals = Signals::new(&[SIGTERM, SIGINT, SIGQUIT])?;
    let handle = signals.handle();
    if let Some(_signal) = signals.next().await {
        info!("Exit signal received, closing D-Bus connection");
        connection.close().await?;
        handle.close();
    }

    Ok(())
}

fn setup_logger(debug: bool) -> Result<()> {
    let colors = ColoredLevelConfig::new()
        .error(Color::Red)
        .warn(Color::Yellow)
        .info(Color::Blue);

    let level = if debug { log::LevelFilter::Debug } else { log::LevelFilter::Info };

    fern::Dispatch::new()
        .format(move |out, message, record| {
            out.finish(format_args!(
                "{:>5} [{}] {}",
                colors.color(record.level()),
                record.target(),
                message
            ))
        })
        .level(level)
        .chain(std::io::stdout())
        .apply()?;
    Ok(())
}
