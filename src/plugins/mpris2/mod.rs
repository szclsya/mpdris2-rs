mod notifier;
pub mod player;
mod root;
pub mod tracklist;
mod utils;

const BUS_NAME: &str = "org.mpris.MediaPlayer2.mpd";
const OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";

use crate::mpd::MpdStateServer;
use notifier::notify_loop;
use player::PlayerInterface;
use root::RootInterface;
use tracklist::TracklistInterface;

use anyhow::{Context, Result};
use log::error;
use std::sync::Arc;
use tokio::{spawn, sync::Mutex, task::JoinHandle};
use zbus::{Connection, ConnectionBuilder};

pub async fn start(
    mpd_state_server: Arc<Mutex<MpdStateServer>>,
) -> Result<(Connection, JoinHandle<()>)> {
    let root_interface = RootInterface::default();
    let player_interface = PlayerInterface::new(mpd_state_server.clone()).await;
    let tracklist_interface = TracklistInterface::new(mpd_state_server.clone());

    let connection = ConnectionBuilder::session().context("Failed to connect to D-Bus session bus. Is $DBUS_SESSION_BUS_ADDRESS set to the correct address?")?
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, root_interface)?
        .serve_at(OBJECT_PATH, player_interface)?
        .serve_at(OBJECT_PATH, tracklist_interface)?
        .build()
        .await?;

    let connection2 = connection.clone();
    let client = mpd_state_server.clone();
    let mut rx = mpd_state_server.lock().await.get_mpd_event_rx();

    let notifier = spawn(async move {
        loop {
            if let Err(e) = notify_loop(&connection2, &mut rx, &client).await {
                error!("D-Bus property change notifier dead, restarting. Reason: {e}");
            }
        }
    });

    Ok((connection, notifier))
}
