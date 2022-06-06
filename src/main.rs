mod interfaces;
mod mpd;

const BUS_NAME: &str = "org.mpris.MediaPlayer2.mpd";
const OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";

use anyhow::Result;
use async_std::{
    prelude::*,
    sync::{Arc, Mutex},
    task,
};
use colored::Colorize;
use fern::colors::{Color, ColoredLevelConfig};
use log::{error, info};
use signal_hook::consts::signal::*;
use signal_hook_async_std::Signals;
use zbus::{export::futures_util::SinkExt, ConnectionBuilder};

fn main() -> Result<()> {
    setup_logger()?;

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
    let mpd_state_server = mpd::MpdStateServer::init("127.0.0.1", 6600).await?;

    let mpris_event_rx = mpd_state_server.get_mpris_event_rx();
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
    let rx2 = mpris_event_rx.clone();
    task::spawn(async move {
        loop {
            if let Err(e) = interfaces::notify_loop(c2.clone(), rx2.clone(), client2.clone()).await
            {
                error!("Interface property change notifier dead, restarting: {e}");
            }
        }
    });

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

fn setup_logger() -> Result<()> {
    let colors = ColoredLevelConfig::new()
        .error(Color::Red)
        .warn(Color::Yellow)
        .info(Color::Blue);

    fern::Dispatch::new()
        .format(move |out, message, record| {
            out.finish(format_args!(
                "{:>5} [{}] {}",
                colors.color(record.level()),
                record.target(),
                message
            ))
        })
        .level(log::LevelFilter::Debug)
        .chain(std::io::stdout())
        .apply()?;
    Ok(())
}
