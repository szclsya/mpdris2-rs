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

use anyhow::Result;
use async_dup::{Arc, Mutex};
use log::error;
use smol::{spawn, Task};
use zbus::{Connection, ConnectionBuilder};

pub async fn start(mpd_state_server: Arc<Mutex<MpdStateServer>>) -> Result<(Connection, Task<()>)> {
    let root_interface = RootInterface::default();
    let player_interface = PlayerInterface::new(mpd_state_server.clone()).await;
    let tracklist_interface = TracklistInterface::new(mpd_state_server.clone());

    let connection = ConnectionBuilder::session()?
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, root_interface)?
        .serve_at(OBJECT_PATH, player_interface)?
        .serve_at(OBJECT_PATH, tracklist_interface)?
        .build()
        .await?;

    let connection2 = connection.clone();
    let client = mpd_state_server.clone();
    let mut rx = mpd_state_server.lock().get_mpris_event_rx();

    let notifier = spawn(async move {
        loop {
            if let Err(e) = notify_loop(&connection2, &mut rx, &client).await {
                error!("D-Bus property change notifier dead, restarting. Reason: {e}");
            }
        }
    });

    Ok((connection, notifier))
}
