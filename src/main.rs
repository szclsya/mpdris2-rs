mod interfaces;
mod mpd;

const BUS_NAME: &str = "org.mpris.MediaPlayer2.mpd2";
const OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";

use anyhow::Result;
use async_std::{
    sync::{Arc, Mutex},
    task,
    prelude::*
};
use zbus::{ ConnectionBuilder, export::futures_util::SinkExt };
use fern::colors::{Color, ColoredLevelConfig};
use signal_hook::consts::signal::*;
use signal_hook_async_std::Signals;
use log::info;

#[async_std::main]
async fn main() -> Result<()> {
    setup_logger()?;
    
    try_main().await?;
    Ok(())
}

async fn try_main() -> Result<()> {
    let mpd_state_server = mpd::MpdStateServer::init("127.0.0.1", 6600).await?;

    let mpris_event_rx = mpd_state_server.get_mpris_event_rx();
    let mpd_state_server = Arc::new(Mutex::new(mpd_state_server));
    let root_interface = interfaces::root::RootInterface::default();
    let player_interface =
        interfaces::player::PlayerInterface::new(mpd_state_server.clone()).await;

    let mut connection = ConnectionBuilder::session()?
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, root_interface)?
        .serve_at(OBJECT_PATH, player_interface)?
        .build()
        .await?;

    // Register state change updater
    let c2 = connection.clone();
    task::spawn(async move {
        interfaces::player::notify_changed(c2, mpris_event_rx.clone())
            .await
            .ok()
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
                "{} [{}] {}",
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
