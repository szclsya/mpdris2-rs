use super::utils::*;
/// Player interface (org.mpris.MediaPlayer2.Player) implementation
use crate::mpd::{types::*, MpdStateServer};

use async_std::sync::{Arc, Mutex, RwLock};
use log::{debug, error};
use std::{collections::HashMap, time::Duration};
use zbus::{dbus_interface, SignalContext};
use zvariant::{ObjectPath, Value};

pub struct PlayerInterface {
    mpdclient: Arc<Mutex<MpdStateServer>>,
    mpd_state: Arc<RwLock<MpdState>>,
}

impl PlayerInterface {
    pub async fn new(mpdclient: Arc<Mutex<MpdStateServer>>) -> Self {
        PlayerInterface {
            mpd_state: mpdclient.clone().lock().await.get_status(),
            mpdclient,
        }
    }
}

#[dbus_interface(name = "org.mpris.MediaPlayer2.Player")]
impl PlayerInterface {
    #[dbus_interface(name = "Play")]
    async fn play(&self, #[zbus(signal_context)] ctxt: SignalContext<'_>) {
        let mut client = self.mpdclient.lock().await;
        match client.issue_command("play").await {
            Ok(_) => {
                PlayerInterface::playback_status_changed(self, &ctxt)
                    .await
                    .ok();
                client.update_status().await.ok();
            }
            Err(e) => {
                error!("org.mpris.MediaPlayer2.Player.Play failed: {e}");
            }
        }
    }

    #[dbus_interface(name = "Pause")]
    async fn pause(&self, #[zbus(signal_context)] ctxt: SignalContext<'_>) {
        match self.mpdclient.lock().await.issue_command("pause 1").await {
            Ok(_) => {
                PlayerInterface::playback_status_changed(self, &ctxt)
                    .await
                    .ok();
            }
            Err(e) => {
                error!("org.mpris.MediaPlayer2.Player.Pause failed: {e}");
            }
        }
    }

    #[dbus_interface(name = "PlayPause")]
    async fn play_pause(&self, #[zbus(signal_context)] ctxt: SignalContext<'_>) {
        match self.mpdclient.lock().await.issue_command("pause").await {
            Ok(_) => {
                PlayerInterface::playback_status_changed(self, &ctxt)
                    .await
                    .ok();
            }
            Err(e) => {
                error!("org.mpris.MediaPlayer2.Player.Play failed: {e}");
            }
        }
    }

    #[dbus_interface(name = "Next")]
    async fn next(&self) {
        self.mpdclient.lock().await.issue_command("next").await.ok();
    }

    #[dbus_interface(name = "Previous")]
    async fn previous(&self, #[zbus(signal_context)] ctxt: SignalContext<'_>) {
        let state = self.mpd_state.read().await;
        let mut cmd = "previous";
        if let MpdPlaybackState::Playing(state) = &state.playback_state {
            if state.elapsed.as_secs_f32() > 3.0 {
                cmd = "seekcur 0";
            }
        }

        match self.mpdclient.lock().await.issue_command(cmd).await {
            Ok(_) => {
                if cmd == "seekcur 0" {
                    PlayerInterface::seeked(&ctxt, 0).await.ok();
                }
            }
            Err(e) => {
                error!("org.mpris.MediaPlayer2.Player.Previous failed: {e}");
            }
        }
    }

    #[dbus_interface(name = "Stop")]
    async fn stop(&self) {
        self.mpdclient.lock().await.issue_command("stop").await.ok();
    }

    #[dbus_interface(name = "Seek")]
    async fn seek(&self, #[zbus(signal_context)] ctxt: SignalContext<'_>, ms: i64) {
        let symbol = if ms > 0 { '+' } else { '-' };
        let t = Duration::from_micros(ms.unsigned_abs());
        let cmd = format!("seekcur {symbol}{}", t.as_secs());
        if let Err(e) = self.mpdclient.lock().await.issue_command(&cmd).await {
            error!("org.mpris.MediaPlayer2.Player.Seek failed: {}", e);
        } else {
            PlayerInterface::seeked(&ctxt, ms).await.ok();
        }
    }

    #[dbus_interface(signal)]
    async fn seeked(signal_ctxt: &SignalContext<'_>, position: i64) -> zbus::Result<()>;

    #[dbus_interface(name = "SetPosition")]
    async fn set_position(
        &self,
        #[zbus(signal_context)] ctxt: SignalContext<'_>,
        track_id: ObjectPath<'_>,
        position: i64,
    ) {
        let state = self.mpd_state.read().await;
        let song = state.song.map(|(_, id)| id);
        if song == object_path_to_id(&track_id) {
            let pos = Duration::from_micros(position as u64);
            let cmd = format!("seekcur {}", pos.as_secs());
            if let Err(e) = self.mpdclient.lock().await.issue_command(&cmd).await {
                error!("org.mpris.MediaPlayer2.Player.SetPosition failed: {}", e);
            } else {
                PlayerInterface::seeked(&ctxt, position).await.ok();
            }
        } else {
            debug!("Wrong song object id: {}", track_id);
        }
    }

    #[dbus_interface(name = "OpenUri")]
    async fn open_uri(&self, uri: &str) {
        let cmd = format!("add {}", uri);
        self.mpdclient.lock().await.issue_command(&cmd).await.ok();
    }

    #[dbus_interface(property, name = "PlaybackStatus")]
    async fn playback_status(&self) -> String {
        self.mpd_state.read().await.playback_state.to_string()
    }

    #[dbus_interface(property, name = "LoopStatus")]
    async fn loop_status(&self) -> String {
        self.mpd_state.read().await.loop_state.to_string()
    }

    #[dbus_interface(property, name = "LoopStatus")]
    async fn set_loop_status(&self, state: String) {
        let commands = match MpdLoopState::from_str(&state) {
            MpdLoopState::None => ["repeat 0", "single 0"],
            MpdLoopState::Track => ["repeat 1", "single 1"],
            MpdLoopState::Playlist => ["repeat 1", "single 0"],
        };
        for cmd in commands {
            self.mpdclient.lock().await.issue_command(cmd).await.ok();
        }
    }

    #[dbus_interface(property, name = "Rate")]
    async fn rate(&self) -> f64 {
        1.0
    }

    #[dbus_interface(property, name = "Rate")]
    async fn set_rate(&self, _: f64) {}

    #[dbus_interface(property, name = "Shuffle")]
    async fn shuffle(&self) -> bool {
        self.mpd_state.read().await.random
    }

    #[dbus_interface(property, name = "Shuffle")]
    async fn set_shuffle(&self, shuffle: bool) {
        let cmd = if shuffle { "random 1" } else { "random 0" };
        self.mpdclient.lock().await.issue_command(cmd).await.ok();
    }

    #[dbus_interface(property, name = "Metadata")]
    async fn metadata(&self) -> HashMap<String, Value<'_>> {
        let state = self.mpd_state.read().await;
        let mut res = if let Some(metadata) = state.current_song.clone() {
            match to_mpris_metadata(metadata) {
                Ok(res) => res,
                Err(e) => {
                    error!("org.mpris.MediaPlayer2.Player.Metadata failed: {}", e);
                    HashMap::new()
                }
            }
        } else {
            HashMap::new()
        };

        let state = self.mpd_state.read().await;
        if let Some(art) = &state.album_art {
            res.insert(
                "mpris:artUrl".to_owned(),
                Value::new(format!("file://{}", art.display())),
            );
        }
        res
    }

    #[dbus_interface(property, name = "Volume")]
    async fn volume(&self) -> f64 {
        if let Some(vol) = self.mpd_state.read().await.volume {
            vol as f64 / 100.0
        } else {
            100.0
        }
    }

    #[dbus_interface(property, name = "Volume")]
    async fn set_volume(&self, volume: f64) {
        if self.mpd_state.read().await.volume == None {
            // No mixer, can't change volume
            return
        }

        let mut volume = (volume * 100.0).floor();
        if volume < 0.0 {
            volume = 0.0;
        }
        let volume = volume as u64;
        let cmd = format!("volume {volume}");
        self.mpdclient.lock().await.issue_command(&cmd).await.ok();
    }

    #[dbus_interface(property, name = "Position")]
    async fn position(&self) -> i64 {
        use MpdPlaybackState::*;

        self.mpdclient.lock().await.update_status().await.ok();
        let elapsed = match &self.mpd_state.read().await.playback_state {
            Playing(s) | Paused(s) => s.elapsed,
            Stopped => Duration::new(0, 0),
        };
        elapsed.as_micros() as i64
    }

    #[dbus_interface(property, name = "MinimumRate")]
    async fn minimum_rate(&self) -> f64 {
        1.0
    }

    #[dbus_interface(property, name = "MaximumRate")]
    async fn maximum_rate(&self) -> f64 {
        1.0
    }

    #[dbus_interface(property, name = "CanGoNext")]
    async fn can_go_next(&self) -> bool {
        let status = self.mpd_state.read().await;
        status.next_song.is_some() || status.loop_state == MpdLoopState::Playlist
    }

    #[dbus_interface(property, name = "CanGoPrevious")]
    async fn can_go_previous(&self) -> bool {
        true
    }

    #[dbus_interface(property, name = "CanPlay")]
    async fn can_play(&self) -> bool {
        let status = self.mpd_state.read().await;
        !matches!(status.playback_state, MpdPlaybackState::Stopped) || status.playlistlength != 0
    }

    #[dbus_interface(property, name = "CanPause")]
    async fn can_pause(&self) -> bool {
        true
    }

    #[dbus_interface(property, name = "CanSeek")]
    async fn can_seek(&self) -> bool {
        true
    }

    #[dbus_interface(property, name = "CanControl")]
    async fn can_control(&self) -> bool {
        true
    }
}
