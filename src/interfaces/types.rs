#[derive(Clone)]
pub enum MprisStateChange {
    Playback,
    Loop,
    Shuffle,
    Volume,
    Song,
    NextSong,
    Tracklist,
}
