use super::{MprisStateChange, PlayerInterface, TracklistInterface};
use crate::mpd::MpdStateServer;

use anyhow::Result;
use async_broadcast::Receiver;
use async_std::sync::{Arc, Mutex};
use log::debug;
use zbus::Connection;
use zvariant::ObjectPath;

pub async fn notify_loop(
    c: &Connection,
    rx: &mut Receiver<MprisStateChange>,
    client: &Arc<Mutex<MpdStateServer>>,
) -> Result<()> {
    use MprisStateChange::*;
    let player_iface_ref = c
        .object_server()
        .interface::<_, PlayerInterface>(crate::OBJECT_PATH)
        .await?;
    let tracklist_iface_ref = c
        .object_server()
        .interface::<_, TracklistInterface>(crate::OBJECT_PATH)
        .await?;

    loop {
        debug!("Waiting for MPD state change from org.mpris2.MediaPlayer2...");
        let signal = rx.recv().await;

        let player_iface = player_iface_ref.get_mut().await;
        let player_ctxt = player_iface_ref.signal_context();
        let tracklist_ctxt = tracklist_iface_ref.signal_context();
        if let Ok(s) = signal {
            match s {
                Playback => {
                    player_iface.playback_status_changed(player_ctxt).await?;
                }
                Loop => {
                    player_iface.loop_status_changed(player_ctxt).await?;
                    player_iface.can_go_next_changed(player_ctxt).await?;
                }
                Shuffle => {
                    player_iface.shuffle_changed(player_ctxt).await?;
                }
                Volume => {
                    player_iface.volume_changed(player_ctxt).await?;
                }
                Song => {
                    player_iface.metadata_changed(player_ctxt).await?;
                    player_iface.playback_status_changed(player_ctxt).await?;
                    player_iface.can_go_next_changed(player_ctxt).await?;
                }
                Tracklist => {
                    use super::tracklist::{extract_ids_from_metadata, get_current_playlist};
                    if let Ok(tracklist) = get_current_playlist(client.clone()).await {
                        let ids: Vec<ObjectPath<'_>> = tracklist
                            .iter()
                            .filter_map(|song| extract_ids_from_metadata(song).ok())
                            .collect();

                        let current_object_path = if ids.is_empty() {
                            ObjectPath::try_from("/org/mpris/MediaPlayer2/TrackList/NoTrack")
                                .unwrap()
                        } else {
                            let client = client.lock().await;
                            let state = client.get_status();
                            let state = state.read().await;
                            let current_pos = state.song.unwrap_or((0, 0)).0;
                            ids.get(current_pos as usize).unwrap().clone()
                        };

                        TracklistInterface::track_list_replaced(
                            tracklist_ctxt,
                            ids,
                            current_object_path,
                        )
                        .await
                        .ok();
                    }
                }
                NextSong => (),
            }
        }
    }
}
