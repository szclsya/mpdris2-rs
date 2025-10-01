#![allow(clippy::too_many_arguments)]
mod template;
use template::format_notification;

use crate::mpd::{
    types::{MpdLoopState, MpdPlaybackState, MpdState},
    MpdStateServer,
};
/// Sending MPD activities as notifications
use crate::types::PlayerStateChange;
use crate::config::Args;

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

    settings: NotificationSetting,
    // Rate-limit settings and internal variables
    notification_signals: Mutex<SignalStream<'a>>,
    last_notification: Mutex<LastNotification>,
    send_actions: bool,
    hints: HashMap<&'a str, Value<'a>>,
}

pub struct NotificationSetting {
    timeout: Duration,
    interval: Duration,
    app_name: String,
    app_icon: String,
    summary_tmpl: String,
    body_tmpl: String,
    paused_summary_tmpl: String,
    paused_body_tmpl: String,
}

impl From<Args> for NotificationSetting {
    fn from(args: Args) -> Self {
        NotificationSetting {
            timeout: Duration::from_secs_f32(args.notification_timeout),
            interval: Duration::from_secs_f32(args.notification_interval),
            app_name: args.app_name,
            app_icon: args.app_icon,
            summary_tmpl: args.notification_summary,
            paused_summary_tmpl: args.notification_summary_paused,
            body_tmpl: args.notification_body,
            paused_body_tmpl: args.notification_body_paused
        }
    }
}

impl<'a> FdoNotificationRelay<'a> {
    pub async fn new(
        connection: &Connection,
        client: Arc<Mutex<MpdStateServer>>,
        args: Args
    ) -> Result<FdoNotificationRelay<'a>> {
        let proxy = NotificationsProxy::new(connection).await?;
        let notification_signals = proxy.0.receive_all_signals().await?;
        let c = client.lock().await;
        let mpd_event_rx = c.get_mpd_event_rx();
        let state = c.get_status();
        let mut hints = HashMap::new();
        hints.insert("urgency", Value::from(0));
        drop(c);

        let settings = NotificationSetting::from(args);

        // Ask server if they support actions
        let capabilities = proxy.get_capabilities().await?;
        let send_actions = capabilities.contains(&"actions".to_string());
        let res = FdoNotificationRelay {
            proxy,
            mpd_event_rx: Mutex::new(mpd_event_rx),
            state,
            client,
            send_actions,
            settings,
            notification_signals: Mutex::new(notification_signals),
            last_notification: Mutex::new(LastNotification::new()),
            hints,
        };

        debug!("Notification timeout: {}", res.settings.timeout.as_secs());

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
        if last_notification.time.elapsed() > self.settings.timeout {
            debug!("Last notification has timed out without server notification. Resetting internal register.");
            last_notification.id = 0;
        }

        debug!(
            "Last notification sent on {:?}, we shouldn't be hitting rate limits",
            last_notification.time.elapsed()
        );

        let state = self.state.read().await;
        //let playback_status = state.playback_state.to_string();
        let (summary, body) = match state.playback_state {
            MpdPlaybackState::Playing(_) => {
                let summary = format_notification(&state, &self.settings.summary_tmpl);
                let body = format_notification(&state, &self.settings.body_tmpl);
                (summary, body)
            }
            MpdPlaybackState::Paused(_) | MpdPlaybackState::Stopped => {
                let summary = format_notification(&state, &self.settings.paused_summary_tmpl);
                let body = format_notification(&state, &self.settings.paused_body_tmpl);
                (summary, body)
            }
        };
        let album_art = state.album_art.clone().map(|p| format!("file://{}", p.display()));
        let can_next = state.next_song.is_some() || state.loop_state == MpdLoopState::Playlist;

        // Update last notification
        last_notification.summary.clone_from(&summary);
        last_notification.body.clone_from(&body);
        last_notification.album_art.clone_from(&album_art);

        if summary != last_notification.summary
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
        if last_notification.time.elapsed() < self.settings.interval {
            debug!("Not sending notification due to rate-limit.");
            return Ok(());
        }

        let mut hints = self.hints.clone();
        if let Some(album_art) = &album_art {
            hints.insert("image-path", Value::from(album_art));
        } else {
            hints.insert("image-path", Value::from(&self.settings.app_icon));
        }
        let actions =
            if self.send_actions { generate_actions(&state.playback_state, can_next) } else { &[] };
        let notification_id = self
            .proxy
            .notify(
                &self.settings.app_name,
                last_notification.id,
                &self.settings.app_icon,
                &summary,
                &body,
                actions,
                &hints,
                self.settings.timeout.as_millis() as i32,
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
    args: Args,
) -> Result<Vec<JoinHandle<()>>> {
    let notification_relay =
        Arc::new(FdoNotificationRelay::new(connection, mpdclient, args).await?);
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
