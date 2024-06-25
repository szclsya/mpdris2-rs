#![allow(clippy::too_many_arguments)]
use crate::mpd::{
    types::{MpdPlaybackState, MpdState},
    MpdStateServer,
};
/// Sending MPD activities as notifications
use crate::types::PlayerStateChange;

use anyhow::Result;
use futures::StreamExt;
use log::{debug, error};
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
// Maxinum length of the title of the music
const MAX_TITLE_LEN: usize = 60;
// Maximum length of a segment of metadata one notification, like artist or radio station name
const MAX_SEGMENT_LEN: usize = 30;

#[proxy(interface = "org.freedesktop.Notifications", assume_defaults = true)]
trait Notifications {
    /// Call the org.freedesktop.Notifications.Notify D-Bus method
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

    #[dbus_proxy(signal)]
    fn notification_closed(&self, id: u32, reason: u32) -> zbus::Result<()>;
}

struct LastNotification {
    id: u32,
    time: Instant,
}

impl LastNotification {
    pub fn new() -> Self {
        LastNotification { id: 0, time: Instant::now() }
    }
}

pub struct FdoNotificationRelay<'a> {
    proxy: NotificationsProxy<'a>,
    mpd_event_rx: Mutex<Receiver<PlayerStateChange>>,
    state: Arc<RwLock<MpdState>>,

    // Settings
    mpd_icon: String,
    notification_timeout: Duration,
    // Rate-limit settings and internal variables
    notification_interval: Duration,
    notification_close_signal: Mutex<SignalStream<'a>>,
    last_notification: Mutex<LastNotification>,
    hints: HashMap<&'a str, Value<'a>>,
}

impl<'a> FdoNotificationRelay<'a> {
    pub async fn new(
        connection: &Connection,
        client: Arc<Mutex<MpdStateServer>>,
        notification_interval: Duration,
    ) -> Result<FdoNotificationRelay<'a>> {
        let proxy = NotificationsProxy::new(connection).await?;
        let notification_close_signal = proxy.0.receive_signal("notification_close").await?;
        let client = client.lock().await;
        let mpd_event_rx = client.get_mpd_event_rx();
        let state = client.get_status();
        let mut hints = HashMap::new();
        hints.insert("urgency", Value::from(0));

        debug!("FdoNotification min interval set to {:?}", notification_interval);

        let res = FdoNotificationRelay {
            proxy,
            mpd_event_rx: Mutex::new(mpd_event_rx),
            state,
            mpd_icon: DEFAULT_MPD_ICON_PATH.to_owned(),
            notification_timeout: Duration::from_secs(DEFUALT_NOTIFICATION_DURATION),
            notification_close_signal: Mutex::new(notification_close_signal),
            last_notification: Mutex::new(LastNotification::new()),
            hints,
            notification_interval,
        };

        debug!("Notification timeout: {}", res.notification_timeout.as_secs());

        Ok(res)
    }

    async fn close_notification(&self) {
        let mut signal_stream = self.notification_close_signal.lock().await;
        // Wait for a notification_close signal
        let _ = signal_stream.next().await;
        debug!("Last notification has been closed, resetting internal register.");
        let mut last = self.last_notification.lock().await;
        last.id = 0;
    }

    async fn send_notification_on_event(&self) -> Result<()> {
        use PlayerStateChange::*;
        loop {
            debug!("Waiting for MPD state change from NotificationRelay...");
            let event = self.mpd_event_rx.lock().await.recv().await?;
            debug!("New event from state server: {:?}", event);
            match event {
                Playback | Song | CurrentSong => {
                    self.send_notification().await?;
                }
                _ => (),
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
        // If last notification is sent within interval, ignore this message
        // This is a rate-limit measure to prevent this error:
        // org.freedesktop.Notifications.Error.ExcessNotificationGeneration
        if last_notification.time.elapsed() < self.notification_interval {
            debug!("Not sending notification due to rate-limit.");
            return Ok(());
        } else {
            debug!(
                "Last notification sent on {:?}, we shouldn't be hitting rate limits",
                last_notification.time.elapsed()
            );
        }

        let state = self.state.read().await;
        let playback_status = state.playback_state.to_string();
        let mut img_uri = state
            .album_art
            .as_ref()
            .map_or_else(|| self.mpd_icon.clone(), |path| path.display().to_string());
        img_uri.insert_str(0, "file://");
        let body = if state.playback_state == MpdPlaybackState::Stopped {
            "Playback stopped".to_string()
        } else if let Some(metadata) = &state.current_song {
            let title = metadata.get("Title").map(|list| list[0].as_str());
            let album = metadata.get("Album").map(|list| list[0].as_str());
            let artist = metadata.get("Artist").map(|list| list[0].as_str());
            // MPD removes the `icy` part of the ICY tag used by internet radios,
            // thus icy_name becomes just "Name"
            // Hopefully nobody is using this tag in local music
            let radio_name = metadata.get("Name").map(|list| list[0].as_str());
            if title.is_none() && artist.is_none() {
                metadata.get("file").map_or("Unknown", |l| l[0].as_str()).to_owned()
            } else if artist.is_none() && radio_name.is_some() {
                // Internet radio
                let radio_name =
                    trim_display_str(radio_name.unwrap_or("Unknown Station"), MAX_SEGMENT_LEN);
                format!("<b>{}</b>\n{radio_name}", title.unwrap_or("Unknown Song"))
            } else {
                let title = trim_display_str(title.unwrap_or("Unknown Song"), MAX_TITLE_LEN);
                let artist = trim_display_str(artist.unwrap_or_default(), MAX_SEGMENT_LEN);
                let album = trim_display_str(album.unwrap_or_default(), MAX_SEGMENT_LEN);
                format!("<b>{title}</b>\n{album}\n{artist}")
            }
        } else {
            "Unknown Song\nUnknown Artist".to_string()
        };

        let notification_id = self
            .proxy
            .notify(
                DEFAULT_PLAYER_NAME,
                last_notification.id,
                &img_uri,
                &playback_status,
                &body,
                &[],
                &self.hints,
                self.notification_timeout.as_millis() as i32,
            )
            .await?;

        debug!("New notification id is {notification_id}");
        last_notification.id = notification_id;
        last_notification.time = Instant::now();
        Ok(())
    }
}

pub async fn start(
    connection: &Connection,
    mpdclient: Arc<Mutex<MpdStateServer>>,
    notification_interval: f32,
) -> Result<JoinHandle<()>> {
    let interval = Duration::from_secs_f32(notification_interval);
    let notification_relay = FdoNotificationRelay::new(connection, mpdclient, interval).await?;
    let task = spawn(async move {
        loop {
            single_run(&notification_relay).await;
        }
    });
    Ok(task)
}

async fn single_run(notification_relay: &FdoNotificationRelay<'_>) {
    tokio::select! {
        res = notification_relay.send_notification_on_event() => {
            if let Err(e) = res {
                error!("NotificationRelay dead, restarting. Reason: {e}");
                sleep(crate::RETRY_INTERVAL).await;
            }
        },
        _ = notification_relay.close_notification() => {
            debug!("Last notification closed based on server signal.");
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
