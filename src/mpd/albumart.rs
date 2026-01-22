use super::{MpdClient, types::Mpdris2State};
use crate::types::PlayerStateChange;

use anyhow::{Result, format_err};
use log::{debug, error, trace, warn};
use std::{
    hash::Hasher,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    fs,
    fs::File,
    io::{AsyncWriteExt, BufWriter},
    sync::Mutex,
    sync::broadcast::Sender,
    time::sleep,
};
use twox_hash::XxHash3_64;

const ALBUM_ART_CACHE_SIZE: usize = 20;

pub async fn update_album_art(
    query_client: Arc<Mutex<MpdClient>>,
    state: Arc<Mpdris2State>,
) -> Result<Option<u64>> {
    let mpdstate = state.mpdstate.read().await;
    let current_song = mpdstate.current_song.clone();
    drop(mpdstate);

    let album_art_dir = &state.album_art_dir;
    let mut album_art_cache = state.album_art_cache.write().await;

    let Some(metadata) = current_song else {
        return Ok(None);
    };
    let uri = &metadata.uri;

    let name_hash = metadata.xxhash3_64();
    let path = hash_to_album_art_path(album_art_dir, name_hash);
    // Check if we already have this already
    for (name_hash_l, pic_hash_l) in album_art_cache.iter() {
        if *name_hash_l == name_hash && path.is_file() {
            trace!("Providing album art from cache");
            state.mpdstate.write().await.album_art = Some(path);
            return Ok(Some(*pic_hash_l));
        }
    }

    trace!("Requested album art not in cache, fetching...");
    let mut new_pic_hash = None;
    for cmd in ["readpicture", "albumart"] {
        let mut c = query_client.lock().await;
        match mpd_binary_to_file(&mut c, cmd, uri, &path).await {
            Ok(Some(pic_hash)) => {
                new_pic_hash = Some(pic_hash);
                break;
            }
            Ok(None) => (),
            Err(e) => debug!("Can't read album art with {cmd}: {e}"),
        };
    }

    if let Some(pic_hash) = new_pic_hash {
        // Update cache
        album_art_cache.push_back((name_hash, pic_hash));
        if album_art_cache.len() > ALBUM_ART_CACHE_SIZE {
            let (old_name_hash, _) = album_art_cache.pop_front().unwrap();
            let old_art_path = hash_to_album_art_path(album_art_dir, old_name_hash);
            if let Err(e) = fs::remove_file(&old_art_path).await {
                warn!("Failed to remove old album art at {}: {e}", old_art_path.display());
            }
        }
        // Update new album art
        state.mpdstate.write().await.album_art = Some(path);
        Ok(Some(pic_hash))
    } else {
        Ok(None)
    }
}

// Update album art multiple times
// This is for Internet radios, which might stream their albums later
pub async fn repeated_update_album_art(
    query_client: Arc<Mutex<MpdClient>>,
    state: Arc<Mpdris2State>,
    retry: u8,
    tx: Sender<Vec<PlayerStateChange>>,
) {
    // Check current state
    if let Some(token) = state.album_art_updating.read().await.as_ref() {
        token.abort();
    }

    let retrieve_interval = 1000;
    let mut last_pic_hash = None;
    let mut i = 1;
    loop {
        trace!("Running repeated album art update: {i}/{retry}");
        if i == retry {
            break;
        }
        i += 1;

        sleep(Duration::from_millis(retrieve_interval)).await;

        // Try to update album art
        match update_album_art(query_client.clone(), state.clone()).await {
            Ok(Some(pic_hash)) => {
                if last_pic_hash == Some(pic_hash) {
                    trace!("Same picture, doing nothing");
                    last_pic_hash = Some(pic_hash);
                    continue;
                }
                // Declare we just updated album art
                if let Err(e) = tx.send(vec![PlayerStateChange::AlbumArt]) {
                    error!("Failed to broadcast delayed update album art update: {e}");
                }

                last_pic_hash = Some(pic_hash);
                // Not breaking here since album art in ICY stream might come later,
                // so we just try multiple times
            }
            Ok(None) => {
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

fn hash_to_album_art_path(base_dir: &Path, hash: u64) -> PathBuf {
    let s = base32::encode(base32::Alphabet::Z, &hash.to_le_bytes());
    base_dir.join(s)
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
) -> Result<Option<u64>> {
    let resp = c.issue_command(&format!("{cmd} \"{uri}\" 0")).await?;
    let fields = resp.field_map();
    let mut offset: u64 = 0;
    if fields.contains_key("binary") {
        let mut hasher = XxHash3_64::with_seed(crate::XXHASH3_64_SEED);
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
                let bytes = resp.binary.unwrap();
                pic_file.write_all(&bytes).await?;
                hasher.write(&bytes);
                if binary_size + offset >= size {
                    // We've read all of them
                    break;
                }
                offset += binary_size;
            }
        }
        let hash = hasher.finish();
        Ok(Some(hash))
    } else {
        Ok(None)
    }
}
