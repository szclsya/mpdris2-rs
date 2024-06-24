use std::path::PathBuf;

#[derive(Clone, Debug)]
pub enum PlayerStateChange {
    Playback,
    Loop,
    Shuffle,
    Volume,
    Song,
    NextSong,
    Tracklist,
}

#[derive(Clone, Debug)]
pub enum MpdConnectionConfig {
    Tcp(String),
    Socket(PathBuf),
}
