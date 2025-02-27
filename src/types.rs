use std::path::PathBuf;

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

#[derive(Clone, Debug)]
pub enum MpdConnectionConfig {
    Tcp(String),
    Socket(PathBuf),
}
