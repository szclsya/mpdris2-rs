use super::{types, types::MpdState, MpdClient};
///
use crate::interfaces::MprisStateChange;

use anyhow::{bail, format_err, Result};
use async_broadcast::{broadcast, InactiveReceiver, Receiver, Sender};
use async_std::{
    fs,
    fs::File,
    io::{BufWriter, WriteExt},
    path::PathBuf,
    sync::{Arc, Mutex, RwLock},
    task,
};
use log::{debug, error};
use std::{mem::discriminant, time::Duration};

const IDLE_CMD: &str = "idle stored_playlist playlist player mixer options";
const PING_INTERVAL: Duration = Duration::from_secs(55);

pub struct MpdStateServer {
    query_client: Arc<Mutex<MpdClient>>,

    mpris_event_tx: Sender<MprisStateChange>,
    _mpris_event_rx: InactiveReceiver<MprisStateChange>,

    // State caches
    state: Arc<RwLock<types::MpdState>>,
}

impl MpdStateServer {
    pub async fn init(address: &str, port: usize) -> Result<Self> {
        // Set up query client
        let mut query_client = MpdClient::new(address, port).await?;

        let initial_state = query_client.issue_command("status").await?;
        let mut initial_state = MpdState::from(initial_state.field_map(), None)?;
        if let Ok(album_art_path) = update_album_art(&mut query_client).await {
            initial_state.album_art = Some(album_art_path);
        }
        let state = Arc::new(RwLock::new(initial_state));

        // Regularly ping to maintain connection
        let query_client = Arc::new(Mutex::new(query_client));
        let qc2 = query_client.clone();
        task::spawn(async move {
            loop {
                let mut client = qc2.lock().await;
                if let Err(e) = client.issue_command("ping").await {
                    error!("ping failed: {}", e);
                    client.reconnect_until_success().await;
                }
                drop(client);
                task::sleep(PING_INTERVAL).await;
            }
        });

        // Create a client that receive MPD state change
        let (mpris_event_tx, mpris_event_rx) = broadcast(50);
        let mpris_event_rx = mpris_event_rx.deactivate();
        let mut idle_client = MpdClient::new(address, port).await?;
        let s2 = state.clone();
        let tx = mpris_event_tx.clone();
        task::spawn(async move {
            loop {
                let res = idle(&mut idle_client, &s2, &tx).await;
                if let Err(e) = res {
                    error!("idle failed, attempting reconnect: {e}");
                    idle_client.reconnect_until_success().await;
                }
            }
        });

        let res = MpdStateServer {
            query_client,
            mpris_event_tx,
            _mpris_event_rx: mpris_event_rx,
            state,
        };
        Ok(res)
    }

    pub fn get_mpris_event_rx(&self) -> Receiver<MprisStateChange> {
        self.mpris_event_tx.new_receiver()
    }

    pub fn get_status(&self) -> Arc<RwLock<MpdState>> {
        self.state.clone()
    }

    pub async fn update_status(&mut self) -> Result<()> {
        let mut c = self.query_client.lock().await;
        update_status(&mut c, &self.state, &self.mpris_event_tx).await?;
        Ok(())
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
}

async fn idle(
    c: &mut MpdClient,
    state: &Arc<RwLock<MpdState>>,
    tx: &Sender<MprisStateChange>,
) -> Result<()> {
    debug!("Entering idle...");
    let res = c.issue_command(IDLE_CMD).await?;
    debug!("Idle interrupted");

    for (name, field) in res.fields {
        if name == "changed" {
            match field.as_str() {
                "stored_playlist" => (),
                "playlist" => {
                    tx.broadcast(MprisStateChange::Tracklist).await?;
                }
                "player" | "mixer" | "options" => update_status(c, state, tx).await?,
                _ => (),
            }
        }
    }

    Ok(())
}

async fn update_status(
    c: &mut MpdClient,
    state: &Arc<RwLock<types::MpdState>>,
    tx: &Sender<MprisStateChange>,
) -> Result<()> {
    let new_status = c.issue_command("status").await?;
    let mut new = if new_status.fields.iter().any(|(name, _)| name == "song") {
        let metadata = c.issue_command("currentsong").await?.field_map();
        MpdState::from(new_status.field_map(), Some(metadata))?
    } else {
        MpdState::from(new_status.field_map(), None)?
    };
    let old = state.read().await;

    // Send notification
    if discriminant(&new.playback_state) != discriminant(&old.playback_state) {
        tx.broadcast(MprisStateChange::Playback).await?;
    }
    if new.loop_state != old.loop_state {
        tx.broadcast(MprisStateChange::Loop).await?;
    }
    if new.random != old.random {
        tx.broadcast(MprisStateChange::Shuffle).await?;
    }
    if new.song != old.song {
        match update_album_art(c).await {
            Ok(new_path) => {
                new.album_art = Some(new_path);
                if let Some(path) = &old.album_art {
                    if path.is_file().await {
                        fs::remove_file(path).await?;
                    }
                }
            }
            Err(e) => {
                error!("Failed to update album art: {}", e);
            }
        }
        tx.broadcast(MprisStateChange::Song).await?;
    } else {
        new.album_art = old.album_art.clone();
    }
    if new.next_song != old.next_song {
        tx.broadcast(MprisStateChange::NextSong).await?;
    }
    if new.volume != old.volume {
        tx.broadcast(MprisStateChange::Volume).await?;
    }
    drop(old);

    *state.write().await = new;
    Ok(())
}

pub async fn update_album_art(c: &mut MpdClient) -> Result<PathBuf> {
    // Find out song URI
    let resp = c.issue_command("currentsong").await?;
    let uri = match resp.field_map().remove("file") {
        Some(mut uri) => uri.remove(0),
        None => bail!("invalid MPD response: no current song uri"),
    };
    let id = match resp.field_map().remove("Id") {
        Some(mut id) => id.remove(0),
        None => bail!("invalid MPD response: no current song ID"),
    };

    let pic_dir = PathBuf::from("/tmp/mpd/album_art/");
    if !pic_dir.is_dir().await {
        async_std::fs::create_dir_all(&pic_dir).await?;
    }
    let pic_path = pic_dir.join(id);
    if pic_path.is_file().await {
        fs::remove_file(&pic_path).await?;
    }
    let mut pic_file = BufWriter::new(File::create(&pic_path).await?);

    // Try integrated art first
    let resp = c.issue_command(&format!("readpicture \"{uri}\" 0")).await?;
    let fields = resp.field_map();
    let mut offset: u64 = 0;
    if fields.contains_key("binary") {
        let size = &fields
            .get("size")
            .ok_or_else(|| format_err!("bad mpd response: no size"))?[0];
        let binary_size = &fields.get("binary").unwrap()[0];
        pic_file.write_all(&resp.binary.unwrap()).await?;
        if size != binary_size {
            offset += binary_size.parse::<u64>()?;
            loop {
                // Read the remaining parts
                let cmd = format!("readpicture \"{uri}\" {offset}");
                let resp = c.issue_command(&cmd).await?;
                let size: u64 = fields
                    .get("size")
                    .ok_or_else(|| format_err!("bad mpd response: no size"))?[0]
                    .parse()?;
                let binary_size: u64 = fields.get("binary").unwrap()[0].parse()?;
                pic_file.write_all(&resp.binary.unwrap()).await?;
                if binary_size + offset >= size {
                    break;
                } else {
                    offset += binary_size;
                }
            }
        }
        debug!(
            "Album art updated from embedded image at {}",
            pic_path.display()
        );
    } else if let Ok(resp) = c.issue_command(&format!("albumart \"{uri}\" 0")).await {
        // Try cover.jpg instead
        let fields = resp.field_map();
        let mut offset: u64 = 0;
        if fields.contains_key("binary") {
            let size = &fields
                .get("size")
                .ok_or_else(|| format_err!("bad mpd response: no size"))?[0];
            let binary_size = &fields.get("binary").unwrap()[0];
            pic_file.write_all(&resp.binary.unwrap()).await?;
            if size != binary_size {
                offset += binary_size.parse::<u64>()?;
                loop {
                    // Read the remaining parts
                    let cmd = format!("albumart {uri} {offset}");
                    let resp = c.issue_command(&cmd).await?;
                    let size: u64 = fields
                        .get("size")
                        .ok_or_else(|| format_err!("bad mpd response: no size"))?[0]
                        .parse()?;
                    let binary_size: u64 = fields.get("binary").unwrap()[0].parse()?;
                    pic_file.write_all(&resp.binary.unwrap()).await?;
                    if binary_size + offset >= size {
                        break;
                    } else {
                        offset += binary_size;
                    }
                }
                debug!(
                    "Album art updated from folder cover file at {}",
                    pic_path.display()
                );
            }
        } else {
            debug!("No album art found")
        }
    }
    pic_file.flush().await?;
    debug!("Album art update finished");
    Ok(pic_path)
}
