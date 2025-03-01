use super::{
    albumart::*,
    types::{self, hashmap_to_song_metadata, MpdState, Mpdris2State},
    MpdClient,
};
use crate::types::{MpdConnectionConfig, PlayerStateChange, SongMetadata};

use anyhow::Result;
use log::{debug, error, trace, warn};
use std::{
    collections::{HashMap, VecDeque},
    mem::discriminant,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::{
    spawn,
    sync::broadcast::{channel, Receiver, Sender},
    sync::{Mutex, RwLock},
    task,
    time::sleep,
};

const IDLE_CMD: &str = "idle player mixer options";
const PING_INTERVAL: Duration = Duration::from_secs(55);

pub struct MpdStateServer {
    query_client: Arc<Mutex<MpdClient>>,
    _ping_task: task::JoinHandle<()>,
    _idle_task: task::JoinHandle<()>,

    mpd_event_tx: Sender<Vec<PlayerStateChange>>,

    // State caches
    state: Arc<Mpdris2State>,
}

impl MpdStateServer {
    pub async fn init(connection_config: MpdConnectionConfig) -> Result<Self> {
        let connection_config = Arc::new(connection_config);
        // Set up query client
        let mut query_client = MpdClient::new(connection_config.clone()).await?;
        let album_art_dir = match dirs::runtime_dir() {
            Some(path) => path,
            None => PathBuf::from("/tmp"),
        }
        .join("mpd/album_art/");
        let mut album_art_cache = VecDeque::new();
        let album_art_updating = Arc::new(RwLock::new(None));

        let init_state = query_client.issue_command("status").await?.field_map();
        let init_meta = query_client.issue_command("currentsong").await?.field_map();
        let mut initial_state = MpdState::from(init_state, init_meta)?;
        if let Err(e) = update_album_art(
            &mut query_client,
            &mut initial_state,
            &album_art_dir,
            &mut album_art_cache,
        )
        .await
        {
            warn!("Can't retrieve initial album art: {e}");
        }
        let mpdstate = Arc::new(RwLock::new(initial_state));
        let album_art_cache = Arc::new(RwLock::new(album_art_cache));
        let state =
            Arc::new(Mpdris2State { album_art_dir, album_art_cache, album_art_updating, mpdstate });

        // Regularly ping to maintain connection
        let query_client = Arc::new(Mutex::new(query_client));
        let qc2 = query_client.clone();
        let _ping_task = spawn(async move {
            loop {
                let mut client = qc2.lock().await;
                if let Err(e) = client.issue_command("ping").await {
                    error!("ping failed: {}", e);
                    client.reconnect_until_success().await;
                }
                drop(client);
                sleep(PING_INTERVAL).await;
            }
        });

        // Create a client that receive MPD state change
        let (mpd_event_tx, _) = channel(10);
        let mut idle_client = MpdClient::new(connection_config.clone()).await?;
        let qc2 = query_client.clone();
        let s2 = state.clone();
        let tx = mpd_event_tx.clone();
        let _idle_task = spawn(async move {
            loop {
                let res = idle(&mut idle_client, &s2, qc2.clone(), &tx).await;
                if let Err(e) = res {
                    error!("idle failed, attempting reconnect: {e}");
                    idle_client.reconnect_until_success().await;
                }
            }
        });

        let res = MpdStateServer { query_client, _ping_task, _idle_task, mpd_event_tx, state };
        Ok(res)
    }

    pub fn get_mpd_event_rx(&self) -> Receiver<Vec<PlayerStateChange>> {
        self.mpd_event_tx.subscribe()
    }

    pub fn get_status(&self) -> Arc<RwLock<MpdState>> {
        self.state.mpdstate.clone()
    }

    pub async fn update_status(&mut self) -> Result<()> {
        let mut c = self.query_client.lock().await;
        update_status(&mut c, self.query_client.clone(), &self.state, &self.mpd_event_tx, "")
            .await?;
        Ok(())
    }

    pub async fn get_track(&mut self, id: u64) -> Result<Option<SongMetadata>> {
        let response = self.issue_command(&format!("playlistid {id}")).await?;
        let mut hashmap: HashMap<String, Vec<String>> =
            response.fields.into_iter().map(|d| (d.0, vec![d.1])).collect();
        let metadata = hashmap_to_song_metadata(&mut hashmap)?;
        Ok(metadata)
    }

    pub async fn get_playlist(&mut self) -> Result<Vec<SongMetadata>> {
        let mut res = Vec::new();

        let response = self.issue_command("playlistinfo").await?;
        let mut buf = HashMap::new();
        for (key, value) in response.fields {
            if key == "file" && !buf.is_empty() {
                // We've just entered a new song, parse previous buffer to a song metadata
                let metadata = hashmap_to_song_metadata(&mut buf)?;
                if let Some(metadata) = metadata {
                    res.push(metadata);
                }
            }
            buf.entry(key.clone()).or_insert_with(|| vec![value.clone()]).push(value.clone());
        }

        Ok(res)
    }

    pub async fn issue_command(&self, cmd: &str) -> Result<types::MpdResponse> {
        let mut client = self.query_client.lock().await;
        let resp = client.issue_command(cmd).await;
        match resp {
            Ok(resp) => Ok(resp),
            Err(e) => {
                error!("Error executing command: {e}");
                client.reconnect_until_success().await;
                client.issue_command(cmd).await
            }
        }
    }

    pub async fn ready(&self) -> Result<()> {
        use PlayerStateChange::*;

        let mut client = self.query_client.lock().await;
        let tx = &self.mpd_event_tx;
        update_status(&mut client, self.query_client.clone(), &self.state, tx, "player").await?;

        let state_changes = vec![Playback, Loop, Shuffle, Volume, Song, NextSong, Tracklist];
        tx.send(state_changes)?;
        Ok(())
    }

    pub async fn cleanup(&self) -> Result<()> {
        if self.state.album_art_dir.is_dir() {
            debug!("Cleaning up album art folder");
            tokio::fs::remove_dir_all(&self.state.album_art_dir).await?;
        }
        Ok(())
    }
}

async fn idle(
    c: &mut MpdClient,
    state: &Arc<Mpdris2State>,
    query_client: Arc<Mutex<MpdClient>>,
    tx: &Sender<Vec<PlayerStateChange>>,
) -> Result<()> {
    trace!("Entering idle...");
    let res = c.issue_command(IDLE_CMD).await?;
    trace!("Idle interrupted");

    for (name, field) in res.fields {
        if name == "changed" {
            debug!("Idle interrupted by {}", field.as_str());
            match field.as_str() {
                "player" | "mixer" | "options" => {
                    update_status(c, query_client.clone(), state, tx, &field).await?;
                }
                unknown => {
                    debug!("Unhandled event from mpd: {unknown}");
                }
            }
        }
    }

    Ok(())
}

async fn update_status(
    c: &mut MpdClient,
    query_client: Arc<Mutex<MpdClient>>,
    state: &Arc<types::Mpdris2State>,
    tx: &Sender<Vec<PlayerStateChange>>,
    subsystem: &str,
) -> Result<()> {
    let new_status = c.issue_command("status").await?;
    let new_metadata = c.issue_command("currentsong").await?.field_map();
    let mut new = MpdState::from(new_status.field_map(), new_metadata)?;
    let old = state.mpdstate.read().await.clone();

    let mut delayed_update = false;
    if let Some(new_metadata) = &new.current_song {
        if new.song != old.song {
            debug!("Updating cover due to new song id");
            if let Some(handle) = state.album_art_updating.read().await.as_ref() {
                handle.abort()
            }
            let mut album_art_cache = state.album_art_cache.write().await;
            update_album_art(c, &mut new, &state.album_art_dir, &mut album_art_cache).await?;
        } else if new_metadata.name.is_some() && subsystem == "player" {
            if new.current_song != old.current_song {
                debug!("Updating cover due to new ICY tag changed");
                delayed_update = true;
                tokio::task::spawn(repeated_update_album_art(
                    query_client,
                    state.clone(),
                    3,
                    tx.clone(),
                ));
            } else {
                new.album_art = old.album_art;
            }
        } else {
            new.album_art = old.album_art;
        }
    }

    // Write changes before broadcasting, so that receivers will have the latest state
    *state.mpdstate.write().await = new;

    // Compare && send state changes
    let new = state.mpdstate.read().await;
    let mut changed = Vec::new();
    if discriminant(&new.playback_state) != discriminant(&old.playback_state) {
        changed.push(PlayerStateChange::Playback);
    }
    if new.loop_state != old.loop_state {
        changed.push(PlayerStateChange::Loop);
    }
    if new.random != old.random {
        changed.push(PlayerStateChange::Shuffle);
    }
    if new.song_id != old.song_id && !delayed_update {
        changed.push(PlayerStateChange::Song);
    }
    if new.next_song != old.next_song {
        changed.push(PlayerStateChange::NextSong);
    }
    if new.volume != old.volume {
        changed.push(PlayerStateChange::Volume);
    }
    if new.song == old.song
        && new.playlistlength == old.playlistlength
        && new.current_song != old.current_song
        && !delayed_update
    {
        changed.push(PlayerStateChange::CurrentSong);
    }

    if !changed.is_empty() {
        tx.send(changed)?;
    }
    Ok(())
}
