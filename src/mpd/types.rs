use anyhow::{Context, Result, bail};
use bytes::Bytes;
use log::warn;
use std::mem::discriminant;
use std::path::PathBuf;
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::Duration,
};
use tokio::{sync::RwLock, task::JoinHandle};

use crate::types::{PlayerStateChange, SongMetadata};

// A list of fields + optional binary data
#[derive(Debug)]
pub struct MpdResponse {
    pub fields: Vec<(String, String)>,
    pub binary: Option<Bytes>,
}

impl MpdResponse {
    pub fn field_map(&self) -> HashMap<String, Vec<String>> {
        let mut res = HashMap::new();
        for (name, value) in &self.fields {
            res.entry(name.clone()).or_insert_with(|| vec![value.clone()]).push(value.to_owned());
        }
        res
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum MpdStateChanged {
    StoredPlaylist,
    CurrentPlaylist,
    Player,
    Mixer,
    Options,
    Unknown(String),
}

impl From<&str> for MpdStateChanged {
    fn from(i: &str) -> Self {
        use MpdStateChanged::*;
        match i {
            "stored_playlist" => StoredPlaylist,
            "playlist" => CurrentPlaylist,
            "player" => Player,
            "mixer" => Mixer,
            "options" => Options,
            _ => Unknown(i.to_owned()),
        }
    }
}

pub struct Mpdris2State {
    pub album_art_dir: PathBuf,
    pub album_art_cache: Arc<RwLock<VecDeque<(u64, u64)>>>,
    pub album_art_updating: Arc<RwLock<Option<JoinHandle<()>>>>,
    pub mpdstate: Arc<RwLock<MpdState>>,
}

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct MpdState {
    pub playback_state: MpdPlaybackState,
    pub loop_state: MpdLoopState,
    pub random: bool,
    pub volume: Option<u64>,
    pub playlist_id: Option<u64>,
    pub song: Option<u64>,
    pub song_id: Option<u64>,
    pub next_song: Option<u64>,
    pub next_song_id: Option<u64>,
    pub playlistlength: u64,

    pub current_song: Option<SongMetadata>,
    pub album_art: Option<PathBuf>,
}

impl MpdState {
    pub fn update_status(
        &mut self,
        mut status: HashMap<String, Vec<String>>,
    ) -> Result<Vec<PlayerStateChange>> {
        let mut missing_fields = Vec::new();
        let mut diffs = Vec::new();
        let mut get_or_complain = |name: &str| match status.get(name) {
            Some(c) => c[0].clone(),
            None => {
                missing_fields.push(name.to_string());
                String::new()
            }
        };
        let get_u64 = |name: &str| match status.get(name) {
            Some(c) => match c[0].parse::<u64>() {
                Ok(res) => Some(res),
                Err(e) => {
                    warn!("expect {name} to be u64, got {}, reason {e}", c[0]);
                    None
                }
            },
            None => None,
        };
        let song = get_u64("song");
        let song_id = get_u64("songid");
        if self.song_id != song_id || self.song != song {
            diffs.push(PlayerStateChange::Song);
        }
        self.song = song;
        self.song_id = song_id;

        let playlistlength = get_u64("playlistlength").unwrap_or_default();
        let playlist_id = get_u64("playlist");
        if self.playlistlength != playlistlength || self.playlist_id != playlist_id {
            diffs.push(PlayerStateChange::Tracklist);
        }
        self.playlistlength = playlistlength;
        self.playlist_id = playlist_id;

        let next_song = get_u64("nextsong");
        let next_song_id = get_u64("nextsongid");
        if self.next_song != next_song || self.next_song_id != next_song_id {
            diffs.push(PlayerStateChange::NextSong);
        }
        self.next_song = next_song;
        self.next_song_id = next_song_id;

        let volume = get_u64("volume");
        if self.volume != volume {
            diffs.push(PlayerStateChange::Volume);
        }
        self.volume = volume;

        let repeat = get_or_complain("repeat");
        let single = get_or_complain("single");
        let loop_state = MpdLoopState::from_mpd(&repeat, &single)?;
        if self.loop_state != loop_state {
            diffs.push(PlayerStateChange::Loop);
        }
        self.loop_state = loop_state;

        let random = mpd_num_to_bool(&get_or_complain("random"), "random")?;
        if self.random != random {
            diffs.push(PlayerStateChange::Shuffle);
        }
        self.random = random;

        let state = get_or_complain("state");
        let playback_state = if state == "play" || state == "pause" {
            let elapsed = status.remove("elapsed");
            let duration = status.remove("duration");
            let elapsed = if let Some(time) = elapsed {
                Some(Duration::from_secs_f64(time[0].parse()?))
            } else {
                None
            };
            let duration = if let Some(time) = duration {
                Some(Duration::from_secs_f64(time[0].parse()?))
            } else {
                None
            };

            let playing_state = MpdPlayingState { elapsed, duration };
            if state == "play" {
                MpdPlaybackState::Playing(playing_state)
            } else {
                MpdPlaybackState::Paused(playing_state)
            }
        } else {
            if !missing_fields.is_empty() {
                bail!("missing fields from MPD status: {}", missing_fields.join(", "));
            }
            MpdPlaybackState::Stopped
        };
        if discriminant(&playback_state) != discriminant(&self.playback_state) {
            diffs.push(PlayerStateChange::Playback);
        }
        self.playback_state = playback_state;

        Ok(diffs)
    }

    pub fn update_metadata(&mut self, mut metadata: HashMap<String, Vec<String>>) -> Result<bool> {
        let new = hashmap_to_song_metadata(&mut metadata)?;
        let diff = self.current_song != new;
        self.current_song = new;
        if diff {
            self.album_art = None;
        }
        Ok(diff)
    }
}

pub fn hashmap_to_song_metadata(
    src: &mut HashMap<String, Vec<String>>,
) -> Result<Option<SongMetadata>> {
    let uri = if let Some(mut uri) = src.remove("file") {
        uri.remove(0)
    } else {
        return Ok(None);
    };

    let id = if let Some(id_str) = src.remove("Id") {
        id_str[0].parse::<u64>().context("Failed to parse `Id` in currentsong")?
    } else {
        bail!("No `Id` in currentsong but file is present")
    };
    let duration = if let Some(value) = src.remove("duration") {
        let duration =
            value[0].parse::<f32>().context("Failed to parse `duration in currentsong")?;
        Some(Duration::from_secs_f32(duration))
    } else {
        None
    };

    let mut get_value = |key: &str| {
        if let Some(mut value) = src.remove(key) {
            if value.is_empty() {
                None
            } else {
                value.dedup();
                Some(value.remove(0))
            }
        } else {
            None
        }
    };
    let res = SongMetadata {
        uri,
        id,
        duration,
        name: get_value("Name"),
        title: get_value("Title"),
        album: get_value("Album"),
        album_artist: get_value("AlbumArtist"),
        artist: get_value("Artist"),
        composer: get_value("Composer"),
        genre: get_value("Genre"),
        disc: get_value("Disc"),
        track: get_value("Track"),
        comment: get_value("Comment"),
    };
    Ok(Some(res))
}

#[derive(PartialEq, Eq, Debug, Clone, Default)]
pub enum MpdPlaybackState {
    #[default]
    Stopped,
    Playing(MpdPlayingState),
    Paused(MpdPlayingState),
}

impl MpdPlaybackState {
    pub fn get_elapsed(&self) -> Option<Duration> {
        match self {
            Self::Playing(x) | Self::Paused(x) => x.elapsed,
            _ => None,
        }
    }
}

impl MpdPlaybackState {
    pub fn as_str(&self) -> &'static str {
        use MpdPlaybackState::*;
        match self {
            Playing(_) => "Playing",
            Paused(_) => "Paused",
            Stopped => "Stopped",
        }
    }
}

#[derive(PartialEq, Eq, Debug, Clone)]
pub struct MpdPlayingState {
    pub elapsed: Option<Duration>,
    pub duration: Option<Duration>,
}

#[derive(PartialEq, Eq, Debug, Clone, Default)]
pub enum MpdLoopState {
    #[default]
    None,
    Track,
    Playlist,
}

impl MpdLoopState {
    pub fn from_mpd(repeat: &str, single: &str) -> Result<Self> {
        use MpdLoopState::*;
        let repeat = mpd_num_to_bool(repeat, "repeat")?;
        let single = mpd_num_to_bool(single, "single")?;
        let res = if repeat && single {
            Track
        } else if repeat && !single {
            Playlist
        } else {
            None
        };
        Ok(res)
    }

    pub fn from_str(s: &str) -> Self {
        use MpdLoopState::*;
        match s {
            "Track" => Track,
            "Playlist" => Playlist,
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        use MpdLoopState::*;
        match self {
            None => "None",
            Track => "Track",
            Playlist => "Playlist",
        }
    }
}

fn mpd_num_to_bool(i: &str, field_name: &str) -> Result<bool> {
    let res = match i {
        "0" => false,
        "1" => true,
        _ => bail!("invalid field {field_name}: expect 0/1, got {i}"),
    };
    Ok(res)
}
