use super::{
    MpdClient,
    albumart::*,
    types::{self, MpdState, Mpdris2State, hashmap_to_song_metadata},
};
use crate::types::{MpdConnectionConfig, PlayerStateChange, SongMetadata};

use anyhow::Result;
use log::{debug, error, trace};
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::{
    spawn,
    sync::broadcast::{Receiver, Sender, channel},
    sync::{Mutex, RwLock},
    task,
    time::sleep,
};

const IDLE_CMD: &str = "idle player mixer playlist options";
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
        let query_client = MpdClient::new(connection_config.clone()).await?;
        let album_art_dir = match dirs::runtime_dir() {
            Some(path) => path,
            None => PathBuf::from("/tmp"),
        }
        .join("mpd/album_art/");
        let album_art_cache = VecDeque::new();
        let album_art_updating = Arc::new(RwLock::new(None));
        let state = MpdState::default();
        let mpdstate = Arc::new(RwLock::new(state));
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
                    error!("ping failed: {e}");
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

    pub async fn update_status(&self) -> Result<()> {
        let new_status = self.query_client.lock().await.issue_command("status").await?.field_map();
        let _diffs = self.state.mpdstate.write().await.update_status(new_status)?;
        Ok(())
    }

    pub async fn full_update_status(&self) -> Result<()> {
        full_update_status(&self.query_client, &self.state, &self.mpd_event_tx, &[]).await?;
        Ok(())
    }

    pub async fn get_track(&self, id: u64) -> Result<Option<SongMetadata>> {
        let response = self.issue_command(&format!("playlistid {id}")).await?;
        let mut hashmap: HashMap<String, Vec<String>> =
            response.fields.into_iter().map(|d| (d.0, vec![d.1])).collect();
        let metadata = hashmap_to_song_metadata(&mut hashmap)?;
        Ok(metadata)
    }

    pub async fn get_playlist(&self) -> Result<Vec<SongMetadata>> {
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

        let tx = &self.mpd_event_tx;

        let state_changes = vec![Song, NextSong, Metadata];
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

    let mut subsystems: Vec<&str> = Vec::new();
    for field in &res.fields {
        if field.0 == "changed" {
            subsystems.push(&field.1);
        }
    }

    if !subsystems.is_empty() {
        trace!("Idle interrupted: {:?}", subsystems);
        full_update_status(&query_client, state, tx, &subsystems).await?;
    }
    Ok(())
}

async fn full_update_status(
    query_client: &Arc<Mutex<MpdClient>>,
    state: &Arc<types::Mpdris2State>,
    tx: &Sender<Vec<PlayerStateChange>>,
    subsystems: &[&str],
) -> Result<()> {
    let mut c = query_client.lock().await;
    let new_status = c.issue_command("status").await?.field_map();
    let new_metadata_map = c.issue_command("currentsong").await?.field_map();
    drop(c);

    let mut mpdstate = state.mpdstate.write().await;
    let mut diffs = mpdstate.update_status(new_status)?;
    let metadata_diff = mpdstate.update_metadata(new_metadata_map)?;
    let mut icy_stream = false;
    if let Some(metadata) = &mpdstate.current_song {
        icy_stream = metadata.name.is_some();
    }
    drop(mpdstate);

    // Determine if it's time for a album art update
    if metadata_diff {
        diffs.push(PlayerStateChange::Metadata);
        if icy_stream {
            debug!("Looks like this is an icecast stream");
            repeated_update_album_art(query_client.clone(), state.clone(), 3, tx.clone()).await;
        } else {
            update_album_art(query_client.clone(), state.clone()).await?;
        }
    }

    if !icy_stream
        && subsystems.contains(&"player")
        && !diffs.contains(&PlayerStateChange::Playback)
        && !diffs.contains(&PlayerStateChange::Song)
        && let Some(elapsed) = state.mpdstate.read().await.playback_state.get_elapsed()
    {
        diffs.push(PlayerStateChange::Seek(elapsed));
    }

    if !diffs.is_empty() {
        diffs.dedup();
        // It's okay if nobody receives it
        // This allows us to do first update before any plugin become active
        tx.send(diffs).ok();
    }
    Ok(())
}
