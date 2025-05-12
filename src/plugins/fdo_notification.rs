#![allow(clippy::too_many_arguments)]
use crate::mpd::{
    types::{MpdLoopState, MpdPlaybackState, MpdState},
    MpdStateServer,
};
/// Sending MPD activities as notifications
use crate::types::PlayerStateChange;

use anyhow::{bail, Result};
use futures::StreamExt;
use log::{debug, error, trace};
use std::{collections::HashMap, default::Default, sync::Arc};
use tokio::{
    spawn,
    sync::broadcast::Receiver,
    sync::{Mutex, RwLock},
    task::JoinHandle,
    time::{sleep, Duration, Instant},
};
use zbus::{proxy, proxy::SignalStream, Connection};
use zvariant::Value;

const DEFAULT_PLAYER_NAME: &str = "Music Player Daemon";
const DEFAULT_MPD_ICON_PATH: &str = "/usr/share/icons/hicolor/scalable/apps/mpd.svg";
const DEFUALT_NOTIFICATION_DURATION: u64 = 5;
// Maximum length of a segment of metadata one notification, like artist or radio station name
const MAX_SEGMENT_LEN: usize = 30;

#[proxy(interface = "org.freedesktop.Notifications", assume_defaults = true)]
trait Notifications {
    fn get_capabilities(&self) -> zbus::Result<Vec<String>>;

    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: &HashMap<&str, Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;

    #[zbus(signal)]
    fn notification_closed(&self, arg_1: u32, arg_2: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    fn action_invoked(&self, id: u32, action_key: &str) -> zbus::Result<()>;
}

struct LastNotification {
    id: u32,
    summary: String,
    body: String,
    album_art: Option<String>,
    time: Instant,
}

impl LastNotification {
    pub fn new() -> Self {
        // This is to make sure a notification get sent when notificaiton service starts
        let time = Instant::now().checked_sub(Duration::from_secs(60)).unwrap();
        LastNotification {
            id: 0,
            summary: String::new(),
            body: String::new(),
            album_art: None,
            time,
        }
    }
}

pub struct FdoNotificationRelay<'a> {
    proxy: NotificationsProxy<'a>,
    mpd_event_rx: Mutex<Receiver<Vec<PlayerStateChange>>>,
    state: Arc<RwLock<MpdState>>,
    client: Arc<Mutex<MpdStateServer>>,

    // Settings
    notification_timeout: Duration,
    // Rate-limit settings and internal variables
    notification_interval: Duration,
    notification_signals: Mutex<SignalStream<'a>>,
    last_notification: Mutex<LastNotification>,
    send_actions: bool,
    hints: HashMap<&'a str, Value<'a>>,
}

impl<'a> FdoNotificationRelay<'a> {
    pub async fn new(
        connection: &Connection,
        client: Arc<Mutex<MpdStateServer>>,
        notification_interval: Duration,
    ) -> Result<FdoNotificationRelay<'a>> {
        let proxy = NotificationsProxy::new(connection).await?;
        let notification_signals = proxy.0.receive_all_signals().await?;
        let c = client.lock().await;
        let mpd_event_rx = c.get_mpd_event_rx();
        let state = c.get_status();
        let mut hints = HashMap::new();
        hints.insert("urgency", Value::from(0));
        drop(c);

        // Ask server if they support actions
        let capabilities = proxy.get_capabilities().await?;
        let send_actions = capabilities.contains(&"actions".to_string());
        debug!("FdoNotification min interval set to {notification_interval:?}");
        let res = FdoNotificationRelay {
            proxy,
            mpd_event_rx: Mutex::new(mpd_event_rx),
            state,
            client,
            send_actions,
            notification_timeout: Duration::from_secs(DEFUALT_NOTIFICATION_DURATION),
            notification_signals: Mutex::new(notification_signals),
            last_notification: Mutex::new(LastNotification::new()),
            hints,
            notification_interval,
        };

        debug!("Notification timeout: {}", res.notification_timeout.as_secs());

        Ok(res)
    }

    async fn handle_signals(&self) -> Result<()> {
        let mut signal_stream = self.notification_signals.lock().await;
        // Wait for a notification_close signal
        if let Some(signal) = signal_stream.next().await {
            // The signal name is stored in header.member
            let header = signal.header();
            let Some(name) = header.member() else {
                bail!("No name in signal from notification daemon, ignoring");
            };
            if name.as_str() == "NotificationClosed" {
                debug!("Last notification has been closed, resetting internal register.");
                let mut last = self.last_notification.lock().await;
                last.id = 0;
            } else if name.as_str() == "ActionInvoked" {
                // Read the real action
                let (_id, body): (u32, String) = signal.body().deserialize()?;
                let c = self.client.lock().await;
                match body.as_str() {
                    "play" => c.issue_command("play").await?,
                    "play-pause" => c.issue_command("pause").await?,
                    "next" => c.issue_command("next").await?,
                    _ => bail!("Invalid notification action"),
                };
            }
        }

        Ok(())
    }

    async fn send_notification_on_event(&self) -> Result<()> {
        use PlayerStateChange::*;
        loop {
            trace!("Waiting for MPD state change from NotificationRelay...");
            let events = self.mpd_event_rx.lock().await.recv().await?;
            trace!("New events from state server: {events:?}");
            for event in events {
                match event {
                    Playback | Song | CurrentSong | AlbumArt => {
                        self.send_notification().await?;
                        continue;
                    }
                    _ => (),
                }
            }
        }
    }

    async fn send_notification(&self) -> Result<()> {
        let mut last_notification = self.last_notification.lock().await;
        // Check if the last notification has expired, if the notification server hasn't notified us
        if last_notification.time.elapsed() > self.notification_timeout {
            debug!("Last notification has timed out without server notification. Resetting internal register.");
            last_notification.id = 0;
        }

        debug!(
            "Last notification sent on {:?}, we shouldn't be hitting rate limits",
            last_notification.time.elapsed()
        );

        let state = self.state.read().await;
        let playback_status = state.playback_state.to_string();
        let body = generate_body(&state);
        let album_art = state.album_art.clone().map(|p| format!("file://{}", p.display()));
        let can_next = state.next_song.is_some() || state.loop_state == MpdLoopState::Playlist;

        // Update last notification
        last_notification.summary.clone_from(&playback_status);
        last_notification.body.clone_from(&body);
        last_notification.album_art.clone_from(&album_art);

        if playback_status != last_notification.summary
            && body == last_notification.body
            && album_art == last_notification.album_art
        {
            debug!("Same notification, not sending");
            return Ok(());
        }

        // If last notification is sent within interval, ignore this message
        // This is a rate-limit measure to prevent this error:
        // org.freedesktop.Notifications.Error.ExcessNotificationGeneration
        // This is placed in the end so that even if we hit rate-limit,
        // last_notification is still correct
        if last_notification.time.elapsed() < self.notification_interval {
            debug!("Not sending notification due to rate-limit.");
            return Ok(());
        }

        let mut hints = self.hints.clone();
        if let Some(album_art) = &album_art {
            hints.insert("image-path", Value::from(album_art));
        } else {
            hints.insert("image-path", Value::from(DEFAULT_MPD_ICON_PATH));
        }
        let actions =
            if self.send_actions { generate_actions(&state.playback_state, can_next) } else { &[] };
        let notification_id = self
            .proxy
            .notify(
                DEFAULT_PLAYER_NAME,
                last_notification.id,
                DEFAULT_MPD_ICON_PATH,
                &playback_status,
                &body,
                actions,
                &hints,
                self.notification_timeout.as_millis() as i32,
            )
            .await?;

        last_notification.time = Instant::now();
        last_notification.id = notification_id;
        debug!("New notification id is {notification_id}");
        Ok(())
    }
}

pub async fn start(
    connection: &Connection,
    mpdclient: Arc<Mutex<MpdStateServer>>,
    notification_interval: f32,
) -> Result<Vec<JoinHandle<()>>> {
    let interval = Duration::from_secs_f32(notification_interval);
    let notification_relay =
        Arc::new(FdoNotificationRelay::new(connection, mpdclient, interval).await?);
    let nr2 = notification_relay.clone();
    let t1 = spawn(async move {
        loop {
            if let Err(e) = notification_relay.send_notification_on_event().await {
                error!("NotificationRelay dead, restarting: {e}");
                sleep(crate::RETRY_INTERVAL).await;
            }
        }
    });
    let t2 = spawn(async move {
        loop {
            if let Err(e) = nr2.handle_signals().await {
                error!("Error handling signal from notification daemon: {e}");
            }
        }
    });
    Ok(vec![t1, t2])
}

fn generate_body(state: &MpdState) -> String {
    if state.playback_state == MpdPlaybackState::Stopped {
        "Playback stopped".to_string()
    } else if let Some(metadata) = &state.current_song {
        if metadata.title.is_none() && metadata.artist.is_none() {
            metadata.uri.to_owned()
        } else {
            let title = metadata.title.as_deref().unwrap_or("Unknown Song");
            let mut res = format!("<b>{}</b>", escape_notification_str(title));
            if let Some(artist) = &metadata.artist {
                let artist = trim_display_str(artist, MAX_SEGMENT_LEN);
                res.push_str(&format!("\n{}", escape_notification_str(&artist)));
            }
            if let Some(album) = &metadata.album {
                let album = trim_display_str(album, MAX_SEGMENT_LEN);
                res.push_str(&format!("\n{}", escape_notification_str(&album)));
            }
            if let Some(name) = &metadata.name {
                let name = trim_display_str(name, MAX_SEGMENT_LEN);
                res.push_str(&format!("\n{}", escape_notification_str(&name)));
            }
            res
        }
    } else {
        todo!()
    }
}

fn generate_actions(playback_state: &MpdPlaybackState, can_next: bool) -> &[&'static str] {
    match playback_state {
        MpdPlaybackState::Stopped => &["play", "⏵"],
        MpdPlaybackState::Playing(_) => {
            if can_next {
                &["play-pause", "⏸", "next", "⏭"]
            } else {
                &["play-pause", "⏸"]
            }
        }
        MpdPlaybackState::Paused(_) => {
            if can_next {
                &["play-pause", "⏵", "next", "⏭"]
            } else {
                &["play-pause", "⏵"]
            }
        }
    }
}

fn trim_display_str(s: &str, max_len: usize) -> String {
    // Unicode might use multiple bytes for one character
    // Since we want string len from a human standpoint, use this instead
    let len = s.chars().count();

    if len > max_len {
        format!("{s:.width$}...", width = max_len - 3)
    } else {
        s.to_owned()
    }
}

fn escape_notification_str(s: &str) -> String {
    s.replace(&['<', '>', '/', '\"', '&'][..], "")
}
