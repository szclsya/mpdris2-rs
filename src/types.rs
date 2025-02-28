use std::{path::PathBuf, hash::{Hash, Hasher}, time::Duration};
use twox_hash::XxHash3_64;

#[derive(Clone, Debug)]
pub enum PlayerStateChange {
    Playback,
    Loop,
    Shuffle,
    Volume,
    Song,
    NextSong,
    // If the current song's metadata is changed
    // Happens when a Internet radios station changes metadata
    CurrentSong,
    AlbumArt,
    Tracklist,
}

#[derive(Hash, Debug, Clone, PartialEq, Eq)]
pub struct SongMetadata {
    pub uri: String,
    pub id: u64,
    pub duration: Option<Duration>,
    // `name` is AFAIK exclusively used by ICY streams
    pub name: Option<String>,
    pub title: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub artist: Option<String>,
    pub composer: Option<String>,
    pub genre: Option<String>,
    pub disc: Option<String>,
    pub track: Option<String>,
    pub comment: Option<String>,
}

impl SongMetadata {
    pub fn xxhash3_64(&self) -> u64 {
        let mut hasher = XxHash3_64::with_seed(crate::XXHASH3_64_SEED);
        self.hash(&mut hasher);
        hasher.finish()
    }
}

#[derive(Clone, Debug)]
pub enum MpdConnectionConfig {
    Tcp(String),
    Socket(PathBuf),
}
