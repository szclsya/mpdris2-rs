#![allow(clippy::too_many_arguments)]
use crate::mpd::{
    types::{MpdPlaybackState, MpdState},
    MpdStateServer,
};
/// Sending MPD activities as notifications
use crate::types::PlayerStateChange;

use anyhow::Result;
use async_broadcast::Receiver;
use async_std::{
    sync::{Arc, Mutex, RwLock},
    task,
};
use log::{debug, error};
use std::collections::HashMap;
use zbus::{dbus_proxy, Connection};
use zvariant::Value;

const DEFAULT_PLAYER_NAME: &str = "Music Player Daemon";
const DEFAULT_MPD_ICON_PATH: &str = "/usr/share/icons/hicolor/scalable/apps/mpd.svg";

#[dbus_proxy]
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
}

pub struct FdoNotificationRelay<'a> {
    proxy: NotificationsProxy<'a>,
    mpris_event_rx: Receiver<PlayerStateChange>,
    state: Arc<RwLock<MpdState>>,

    // Settings
    mpd_icon: String,
    notification_timeout: u32,
    last_notification_id: u32,
    hints: HashMap<&'a str, Value<'a>>,
}

impl<'a> FdoNotificationRelay<'a> {
    pub async fn new(
        connection: &Connection,
        client: Arc<Mutex<MpdStateServer>>,
    ) -> Result<FdoNotificationRelay<'a>> {
        let proxy = NotificationsProxy::new(connection).await?;
        let client = client.lock().await;
        let mpris_event_rx = client.get_mpris_event_rx();
        let state = client.get_status();
        let mut hints = HashMap::new();
        hints.insert("urgency", Value::from(0));

        let res = FdoNotificationRelay {
            proxy,
            mpris_event_rx,
            state,
            mpd_icon: DEFAULT_MPD_ICON_PATH.to_owned(),
            notification_timeout: 5000,
            last_notification_id: 0,
            hints,
        };

        Ok(res)
    }

    async fn send_notification_on_event(&mut self) -> Result<()> {
        use PlayerStateChange::*;
        loop {
            debug!("Waiting for MPD state change from NotificationRelay...");
            let event = self.mpris_event_rx.recv().await?;
            match event {
                Playback | Song => {
                    self.send_notification().await?;
                }
                _ => (),
            }
        }
    }

    async fn send_notification(&mut self) -> Result<()> {
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
            let artist = metadata.get("Artist").map(|list| list[0].as_str());
            if title.is_none() || artist.is_none() {
                metadata
                    .get("file")
                    .map_or("Unknown", |l| l[0].as_str())
                    .to_owned()
            } else {
                format!(
                    "{} - {}",
                    artist.unwrap_or("Unknown Artist"),
                    title.unwrap_or("Unknown Song")
                )
            }
        } else {
            "Unknown Song - Unknown Artist".to_string()
        };

        let notification_id = self
            .proxy
            .notify(
                DEFAULT_PLAYER_NAME,
                self.last_notification_id,
                &img_uri,
                &playback_status,
                &body,
                &[],
                &self.hints,
                self.notification_timeout as i32,
            )
            .await?;

        self.last_notification_id = notification_id;

        Ok(())
    }
}

pub async fn start(connection: &Connection, mpdclient: Arc<Mutex<MpdStateServer>>) -> Result<()> {
    let mut notification_relay = FdoNotificationRelay::new(connection, mpdclient).await?;
    async_std::task::spawn(async move {
        loop {
            if let Err(e) = notification_relay.send_notification_on_event().await {
                error!("NotificationRelay dead, restarting. Reason: {e}");
                task::sleep(crate::RETRY_INTERVAL).await;
            }
        }
    });
    Ok(())
}
