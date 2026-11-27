use super::{OBJECT_PATH, PlayerInterface, TracklistInterface};
use crate::{
    mpd::{MpdStateServer, types::MpdPlaybackState},
    types::PlayerStateChange,
};

use anyhow::Result;
use log::trace;
use std::sync::Arc;
use tokio::sync::broadcast::Receiver;
use zbus::{Connection, ObjectPath};

pub async fn notify_loop(
    c: &Connection,
    rx: &mut Receiver<Vec<PlayerStateChange>>,
    client: &Arc<MpdStateServer>,
) -> Result<()> {
    use PlayerStateChange::*;
    let player_iface_ref = c.object_server().interface::<_, PlayerInterface>(OBJECT_PATH).await?;
    let tracklist_iface_ref =
        c.object_server().interface::<_, TracklistInterface>(OBJECT_PATH).await?;

    loop {
        trace!("Waiting for MPD state change from org.mpris2.MediaPlayer2...");
        let signal = rx.recv().await;

        let player_iface = player_iface_ref.get_mut().await;
        let player_ctxt = player_iface_ref.signal_emitter();
        let tracklist_ctxt = tracklist_iface_ref.signal_emitter();
        if let Ok(states) = signal {
            for s in states {
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
                    Song | NextSong => {
                        player_iface.playback_status_changed(player_ctxt).await?;
                        player_iface.can_go_next_changed(player_ctxt).await?;
                    }
                    Seek(elapsed) => {
                        PlayerInterface::seeked(player_ctxt, elapsed.as_micros() as i64).await?;
                    }
                    Metadata | AlbumArt => {
                        player_iface.playback_status_changed(player_ctxt).await?;
                        player_iface.metadata_changed(player_ctxt).await?;
                    }
                    Tracklist => {
                        use super::tracklist::{extract_ids_from_metadata, get_current_playlist};
                        if let Ok(tracklist) = get_current_playlist(client.clone()).await {
                            let ids: Vec<ObjectPath<'_>> = tracklist
                                .iter()
                                .filter_map(|song| extract_ids_from_metadata(song).ok())
                                .collect();
                            let state = client.get_status();
                            let state = state.read().await;
                            let no_track =
                                ObjectPath::try_from("/org/mpris/MediaPlayer2/TrackList/NoTrack")
                                    .unwrap();
                            // MPD keeps song/songid set after a stop, so gate on the
                            // playback state: a stopped player has no current track.
                            let current_object_path =
                                if matches!(state.playback_state, MpdPlaybackState::Stopped) {
                                    no_track
                                } else {
                                    // Make sure the id is not stale
                                    state
                                        .song
                                        .and_then(|pos| ids.get(pos as usize))
                                        .cloned()
                                        .unwrap_or(no_track)
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
                }
            }
        }
    }
}
