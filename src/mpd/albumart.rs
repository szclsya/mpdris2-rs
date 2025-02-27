use super::{
    types::{MpdState, Mpdris2State},
    MpdClient,
};
use crate::types::PlayerStateChange;

use anyhow::{bail, format_err, Result};
use log::{debug, error, trace, warn};
use std::{
    collections::{HashMap, VecDeque},
    hash::Hasher,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::{
    fs,
    fs::File,
    io::{AsyncWriteExt, BufWriter},
    sync::broadcast::Sender,
    sync::Mutex,
    time::sleep,
};
use tokio_util::sync::CancellationToken;

const ALBUM_ART_CACHE_SIZE: usize = 20;

pub async fn update_album_art(
    c: &mut MpdClient,
    state: &mut MpdState,
    album_art_cache: &mut VecDeque<u64>,
) -> Result<bool> {
    let uri = if let Some(uri) = &state.file {
        uri
    } else {
        bail!("No `file` in currentsong!");
    };

    let hash = hash_metadata(&state.current_song);
    let path = hash_to_album_art_path(hash);
    // Check if we already have this already
    if album_art_cache.contains(&hash) {
        // Just give them the filename and we'd be good
        if path.is_file() {
            state.album_art = Some(path);
            return Ok(true);
        }
    }

    // Not in cache, fetch it
    let mut success = false;
    for cmd in ["readpicture", "albumart"] {
        match mpd_binary_to_file(c, "readpicture", uri, &path).await {
            Ok(true) => {
                success = true;
                break;
            }
            Ok(false) => (),
            Err(e) => debug!("Can't read album art with {cmd}: {e}"),
        };
    }

    if success {
        // Update cache
        album_art_cache.push_back(hash);
        if album_art_cache.len() > ALBUM_ART_CACHE_SIZE {
            let hash = album_art_cache.pop_front().unwrap();
            let old_art_path = hash_to_album_art_path(hash);
            if let Err(e) = fs::remove_file(&old_art_path).await {
                warn!("Failed to remove old album art at {}: {e}", old_art_path.display());
            }
        }
        // Update new album art
        state.album_art = Some(path);
        Ok(true)
    } else {
        Ok(false)
    }
}

// Update album art multiple times
// This is for Internet radios, which might stream their albums later
pub async fn repeated_update_album_art(
    query_client: Arc<Mutex<MpdClient>>,
    state: Arc<Mpdris2State>,
    retry: u8,
    tx: Sender<PlayerStateChange>,
) {
    // Check current state
    if let Some(token) = state.album_art_updating.read().await.as_ref() {
        token.cancel();
    }

    let cancel = CancellationToken::new();
    *state.album_art_updating.write().await = Some(cancel.clone());

    let retrieve_interval = 1000;
    let mut i = 1;
    loop {
        trace!("Running repeated album art update: {i}/{retry}");
        if i == retry {
            break;
        } else {
            i += 1;
        }

        tokio::select! {
            _ = cancel.cancelled() => { trace!("Repeated album art update cancelled"); break},
            _ = sleep(Duration::from_millis(retrieve_interval)) => (),
        };

        // Try to update album art
        let mut c = query_client.lock().await;
        let mut mpdstate = state.mpdstate.write().await;
        let mut album_art_cache = state.album_art_cache.write().await;
        match update_album_art(&mut c, &mut mpdstate, &mut album_art_cache).await {
            Ok(true) => {
                // Declare we just updated album art
                if let Err(e) = tx.send(PlayerStateChange::CurrentSong) {
                    error!("Failed to broadcast delayed update album art update: {e}");
                }
                // Not breaking here since album art in ICY stream might come later,
                // so we just try multiple times
            }
            Ok(false) => {
                // Do nothing, try again next time
                continue;
            }
            Err(e) => {
                debug!("Failed to retrieve new album art: {e} in attempt {retry}.");
            }
        }
    }

    *state.album_art_updating.write().await = None;
    trace!("Repeated album art update done.");
}

// Calculate a hash for a song
fn hash_metadata(metadata: &HashMap<String, Vec<String>>) -> u64 {
    let mut hasher = twox_hash::XxHash32::with_seed(123);
    for (k, v) in metadata {
        hasher.write(k.as_bytes());
        for v in v {
            hasher.write(v.as_bytes());
        }
    }
    hasher.finish()
}

fn hash_to_album_art_path(hash: u64) -> PathBuf {
    let pic_dir = match dirs::runtime_dir() {
        Some(path) => path,
        None => PathBuf::from("/tmp"),
    }
    .join("mpd/album_art/");
    let s = base32::encode(base32::Alphabet::Z, &hash.to_le_bytes());
    pic_dir.join(s)
}

async fn prepare_album_art_file(path: &PathBuf) -> Result<BufWriter<File>> {
    if let Some(pic_dir) = path.parent() {
        if !pic_dir.is_dir() {
            fs::create_dir_all(&pic_dir).await?;
        }
    }
    if path.is_file() {
        fs::remove_file(&path).await?;
    }
    debug!("Creating new album art file at {}", path.display());
    let file = BufWriter::new(File::create(&path).await?);
    Ok(file)
}

async fn mpd_binary_to_file(
    c: &mut MpdClient,
    cmd: &str,
    uri: &str,
    path: &PathBuf,
) -> Result<bool> {
    let resp = c.issue_command(&format!("{cmd} \"{uri}\" 0")).await?;
    let fields = resp.field_map();
    let mut offset: u64 = 0;
    if fields.contains_key("binary") {
        let size = &fields.get("size").ok_or_else(|| format_err!("bad mpd response: no size"))?[0];
        let binary_size = &fields.get("binary").unwrap()[0];
        let mut pic_file = prepare_album_art_file(path).await?;
        pic_file.write_all(&resp.binary.unwrap()).await?;
        if size != binary_size {
            offset += binary_size.parse::<u64>()?;
            loop {
                // Read the remaining parts
                let cmd = format!("{cmd} \"{uri}\" {offset}");
                let resp = c.issue_command(&cmd).await?;
                let size: u64 =
                    fields.get("size").ok_or_else(|| format_err!("bad mpd response: no size"))?[0]
                        .parse()?;
                let binary_size: u64 = fields.get("binary").unwrap()[0].parse()?;
                pic_file.write_all(&resp.binary.unwrap()).await?;
                if binary_size + offset >= size {
                    // We've read all of them
                    break;
                }
                offset += binary_size;
            }
        }
        Ok(true)
    } else {
        Ok(false)
    }
}
