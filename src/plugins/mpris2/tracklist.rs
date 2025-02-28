use super::utils::*;
/// `TrackList` interface (org.mpris.MediaPlayer2.TrackList) implementation
use crate::mpd::MpdStateServer;

use anyhow::{format_err, Context};
use log::{error, warn};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;
use zbus::{interface, object_server::SignalEmitter};
use zvariant::{ObjectPath, Value};

pub struct TracklistInterface {
    mpdclient: Arc<Mutex<MpdStateServer>>,
}

impl TracklistInterface {
    pub fn new(mpdclient: Arc<Mutex<MpdStateServer>>) -> Self {
        TracklistInterface { mpdclient }
    }
}

#[interface(name = "org.mpris.MediaPlayer2.TrackList")]
impl<'a> TracklistInterface {
    #[zbus()]
    async fn get_track_metadata(
        &self,
        tracks: Vec<ObjectPath<'_>>,
    ) -> zbus::fdo::Result<Vec<HashMap<String, Value<'a>>>> {
        let ids: Vec<Value<'_>> = tracks.into_iter().map(Value::new).collect();

        let mut res = Vec::new();
        for id in ids {
            let Value::ObjectPath(obj_path) = id else {
                return Err(to_fdo_err(format_err!("Invalid song id")));
            };
            // Extract id
            let id = if let Some(id_str) = obj_path.as_str().strip_prefix("/org/musicpd/song/") {
                id_str.parse::<u64>().context("Invalid song id").map_err(to_fdo_err)?
            } else {
                return Err(to_fdo_err(format_err!("Invalid song id")));
            };
            if let Some(metadata) =
                self.mpdclient.lock().await.get_track(id).await.map_err(to_fdo_err)?
            {
                let mut mpris_metadata = HashMap::new();
                to_mpris_metadata(&metadata, &mut mpris_metadata);
                res.push(mpris_metadata);
            } else {
                warn!("MPRIS2 requested track {id} but it doesn't seem to exist, skipping");
            }
        }
        Ok(res)
    }

    #[zbus()]
    fn add_track(&self, _uri: String, _after: ObjectPath<'_>, _set_as_current: bool) {
        // We don't do that here.jpg
    }

    #[zbus()]
    fn remove_track(&self, _track: ObjectPath<'_>) {
        // We don't do that here either
    }

    #[zbus()]
    async fn goto(
        &self,
        #[zbus(signal_context)] ctxt: SignalEmitter<'_>,
        track: ObjectPath<'_>,
    ) -> zbus::fdo::Result<()> {
        let Some(id) = object_path_to_id(&track) else { return Ok(()) };

        let cmd = format!("playid {id}");
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

    #[zbus(signal)]
    pub async fn track_list_replaced(
        ctxt: &SignalEmitter<'_>,
        tracks: Vec<ObjectPath<'_>>,
        current: ObjectPath<'_>,
    ) -> zbus::Result<()>;

    #[allow(dead_code)]
    #[zbus(signal)]
    async fn track_added(
        ctxt: &SignalEmitter<'_>,
        metadata: HashMap<String, Value<'_>>,
        after: ObjectPath<'_>,
    ) -> zbus::Result<()>;

    #[allow(dead_code)]
    #[zbus(signal)]
    async fn track_removed(ctxt: &SignalEmitter<'_>, track: ObjectPath<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn track_metadata_changed(
        ctxt: &SignalEmitter<'_>,
        track: ObjectPath<'_>,
        metadata: HashMap<String, Value<'_>>,
    ) -> zbus::Result<()>;

    #[zbus(property)]
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

    #[zbus(property)]
    fn can_edit_tracks(&self) -> bool {
        false
    }
}

pub async fn get_current_playlist<'a>(
    client: Arc<Mutex<MpdStateServer>>,
) -> zbus::fdo::Result<Vec<HashMap<std::string::String, zvariant::Value<'a>>>> {
    let metadatas = client.lock().await.get_playlist().await.map_err(to_fdo_err)?;
    let mut res = Vec::with_capacity(metadatas.len());
    for metadata in metadatas {
        let mut entry = HashMap::new();
        to_mpris_metadata(&metadata, &mut entry);
        res.push(entry);
    }

    Ok(res)
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
        Err(zbus::fdo::Error::Failed("mpris::trackid is not ObjectPath".to_string()))
    }
}

fn to_fdo_err(e: anyhow::Error) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(e.to_string())
}
