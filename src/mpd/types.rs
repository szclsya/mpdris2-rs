use anyhow::{bail, Result};
use log::warn;
use std::path::PathBuf;
use std::{collections::HashMap, fmt::Display, time::Duration};

// A list of fields + optional binary data
#[derive(Debug)]
pub struct MpdResponse {
    pub fields: Vec<(String, String)>,
    pub binary: Option<Vec<u8>>,
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

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MpdState {
    pub playback_state: MpdPlaybackState,
    pub loop_state: MpdLoopState,
    pub random: bool,
    pub volume: Option<u64>,
    pub playlist_id: Option<u64>,
    pub song: Option<u64>,
    pub song_id: Option<u64>,
    pub next_song: Option<(u64, u64)>,
    pub playlistlength: u64,

    pub current_song: Option<HashMap<String, Vec<String>>>,
    pub album_art: Option<PathBuf>,
}

impl MpdState {
    pub fn from(
        mut status: HashMap<String, Vec<String>>,
        metadata: Option<HashMap<String, Vec<String>>>,
    ) -> Result<Self> {
        let mut missing_fields = Vec::new();
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

        let playlistlength = get_u64("playlistlength");
        let song = get_u64("song");
        let song_id = get_u64("songid");
        let playlist_id = get_u64("playlist");
        let next_song = get_u64("nextsong");
        let next_song_id = get_u64("nextsongid");
        let volume = get_u64("volume");
        let state = get_or_complain("state");
        let repeat = get_or_complain("repeat");
        let single = get_or_complain("single");
        let random = get_or_complain("random");
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

        let next_song = if let (Some(next_song), Some(next_song_id)) = (next_song, next_song_id) {
            Some((next_song, next_song_id))
        } else {
            None
        };

        let res = MpdState {
            playback_state,
            loop_state: MpdLoopState::from_mpd(&repeat, &single)?,
            random: mpd_num_to_bool(&random, "random")?,
            volume,
            playlist_id,
            song,
            song_id,
            next_song,
            playlistlength: playlistlength.unwrap_or(0),
            current_song: metadata,
            album_art: None,
        };

        Ok(res)
    }
}

#[derive(PartialEq, Eq, Debug, Clone)]
pub enum MpdPlaybackState {
    Playing(MpdPlayingState),
    Paused(MpdPlayingState),
    Stopped,
}

impl Display for MpdPlaybackState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use MpdPlaybackState::*;
        let s = match self {
            Playing(_) => "Playing",
            Paused(_) => "Paused",
            Stopped => "Stopped",
        };
        f.write_str(s)
    }
}

#[derive(PartialEq, Eq, Debug, Clone)]
pub struct MpdPlayingState {
    pub elapsed: Option<Duration>,
    pub duration: Option<Duration>,
}

#[derive(PartialEq, Eq, Debug, Clone)]
pub enum MpdLoopState {
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
            "None" => None,
            "Track" => Track,
            "Playlist" => Playlist,
            _ => None,
        }
    }
}

impl Display for MpdLoopState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use MpdLoopState::*;
        let s = match self {
            None => "None",
            Track => "Track",
            Playlist => "Playlist",
        };
        f.write_str(s)
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
