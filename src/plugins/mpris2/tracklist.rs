use super::utils::*;
/// `TrackList` interface (org.mpris.MediaPlayer2.TrackList) implementation
use crate::mpd::MpdStateServer;

use async_std::sync::{Arc, Mutex};
use log::error;
use std::collections::HashMap;
use zbus::{dbus_interface, SignalContext};
use zvariant::{ObjectPath, Value};

pub struct TracklistInterface {
    mpdclient: Arc<Mutex<MpdStateServer>>,
}

impl TracklistInterface {
    pub fn new(mpdclient: Arc<Mutex<MpdStateServer>>) -> Self {
        TracklistInterface { mpdclient }
    }
}

#[dbus_interface(name = "org.mpris.MediaPlayer2.TrackList")]
impl<'a> TracklistInterface {
    #[dbus_interface(name = "GetTracksMetadata")]
    async fn get_track_metadata(
        &self,
        tracks: Vec<ObjectPath<'_>>,
    ) -> zbus::fdo::Result<Vec<HashMap<String, Value<'a>>>> {
        let ids: Vec<Value<'_>> = tracks.into_iter().map(Value::new).collect();

        let metadatas = get_current_playlist(self.mpdclient.clone()).await?;
        let metadatas = metadatas
            .into_iter()
            .filter(|metadatas| {
                if let Some(path) = metadatas.get("mpris:trackid") {
                    ids.contains(path)
                } else {
                    false
                }
            })
            .collect();

        Ok(metadatas)
    }

    #[dbus_interface(name = "AddTrack")]
    async fn add_track(&self, _uri: String, _after: ObjectPath<'_>, _set_as_current: bool) {
        // We don't do that here.jpg
    }

    #[dbus_interface(name = "RemoveTrack")]
    async fn remove_track(&self, _track: ObjectPath<'_>) {
        // We don't do that here either
    }

    #[dbus_interface(name = "GoTo")]
    async fn goto(
        &self,
        #[zbus(signal_context)] ctxt: SignalContext<'_>,
        track: ObjectPath<'_>,
    ) -> zbus::fdo::Result<()> {
        let id = if let Some(id) = object_path_to_id(&track) {
            id
        } else {
            return Ok(());
        };

        let cmd = format!("playid {}", id);
        match self.mpdclient.lock().await.issue_command(&cmd).await {
            Ok(_resp) => {
                let mut new_metadata = self.get_track_metadata(vec![track.clone()]).await?;
                let new_metadata = new_metadata.remove(0);
                if !new_metadata.is_empty() {
                    TracklistInterface::track_metadata_changed(&ctxt, track, new_metadata).await?;
                }
            }
            Err(e) => {
                error!("org.mpris.MediaPlayer2.TrackList.GoTo failed: {e}");
            }
        }
        Ok(())
    }

    #[dbus_interface(signal, name = "TrackListReplaced")]
    pub async fn track_list_replaced(
        ctxt: &SignalContext<'_>,
        tracks: Vec<ObjectPath<'_>>,
        current: ObjectPath<'_>,
    ) -> zbus::Result<()>;

    #[allow(dead_code)]
    #[dbus_interface(signal, name = "TrackAdded")]
    async fn track_added(
        ctxt: &SignalContext<'_>,
        metadata: HashMap<String, Value<'_>>,
        after: ObjectPath<'_>,
    ) -> zbus::Result<()>;

    #[allow(dead_code)]
    #[dbus_interface(signal, name = "TrackRemoved")]
    async fn track_removed(ctxt: &SignalContext<'_>, track: ObjectPath<'_>) -> zbus::Result<()>;

    #[dbus_interface(signal, name = "TrackMetadataChanged")]
    async fn track_metadata_changed(
        ctxt: &SignalContext<'_>,
        track: ObjectPath<'_>,
        metadata: HashMap<String, Value<'_>>,
    ) -> zbus::Result<()>;

    #[dbus_interface(property, name = "Tracks")]
    async fn tracks(&self) -> Vec<ObjectPath<'_>> {
        let client = self.mpdclient.lock().await;
        let resp = match client.issue_command("playlistinfo").await {
            Ok(resp) => resp,
            Err(e) => {
                error!("org.mpris.MediaPlayer2.Tracks failed: {e}");
                return Vec::new();
            }
        };

        let mut ids = Vec::new();
        for (name, value) in resp.fields {
            if name == "Id" {
                let id = id_to_object_path(value);
                ids.push(id);
            }
        }

        ids
    }

    #[dbus_interface(property, name = "CanEditTracks")]
    async fn can_edit_tracks(&self) -> bool {
        false
    }
}

pub async fn get_current_playlist<'a>(
    client: Arc<Mutex<MpdStateServer>>,
) -> zbus::fdo::Result<Vec<HashMap<std::string::String, zvariant::Value<'a>>>> {
    let client = client.lock().await;
    let res = client
        .issue_command("playlistinfo")
        .await
        .map_err(to_fdo_err)?;

    let mut metadatas = Vec::new();

    let mut buf = HashMap::new();
    for (name, value) in res.fields {
        buf.entry(name.clone())
            .or_insert_with(|| vec![value.clone()])
            .push(value.clone());
        if name == "Id" {
            let mut new_buf = HashMap::new();
            std::mem::swap(&mut buf, &mut new_buf);
            let new_metadata = to_mpris_metadata(new_buf).map_err(to_fdo_err)?;
            metadatas.push(new_metadata);
        }
    }

    Ok(metadatas)
}

pub fn extract_ids_from_metadata<'a>(
    i: &HashMap<String, Value<'_>>,
) -> zbus::fdo::Result<ObjectPath<'a>> {
    let path = i
        .get("mpris::trackid")
        .ok_or_else(|| zbus::fdo::Error::Failed("mpris::trackid doesn't exist".to_string()))?;
    if let Value::ObjectPath(p) = path {
        Ok(p.to_owned())
    } else {
        Err(zbus::fdo::Error::Failed(
            "mpris::trackid is not ObjectPath".to_string(),
        ))
    }
}

fn to_fdo_err(e: anyhow::Error) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(e.to_string())
}
