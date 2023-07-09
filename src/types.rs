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
