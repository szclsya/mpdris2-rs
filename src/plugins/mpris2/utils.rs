use anyhow::Result;
use std::collections::HashMap;
use zvariant::{ObjectPath, Value};

use crate::types::SongMetadata;

pub fn id_to_object_path<'a>(id: impl std::fmt::Display) -> ObjectPath<'a> {
    let path = format!("/org/musicpd/song/{id}");
    ObjectPath::try_from(path).unwrap()
}

pub fn object_path_to_id(path: &ObjectPath) -> Option<u64> {
    if let Some(n) = path.strip_prefix("/org/musicpd/song/") {
        if let Ok(n) = n.parse() {
            return Some(n);
        }
    }
    None
}

pub fn to_mpris_metadata(
    i: &SongMetadata,
    buf: &mut HashMap<String, Value<'_>>
) -> Result<()> {
    let mut optional_insert = |tag: &str, src: &Option<String>| {
        if let Some(value) = src {
            buf.insert(tag.to_owned(), Value::new(value.to_owned()));
        }
    };
    optional_insert("xesam:title", &i.title);
    optional_insert("xesam:album", &i.album);
    optional_insert("xesam:albumArtist", &i.album_artist);
    optional_insert("xesam:artist", &i.artist);
    optional_insert("xesam:composer", &i.composer);
    optional_insert("xesam:discNumber", &i.disc);
    optional_insert("xesam:genre", &i.genre);
    optional_insert("xesam:trackNumber", &i.track);

    // Special types
    buf.insert("mpris:trackid".to_owned(), Value::new(id_to_object_path(i.id)));
    buf.insert("xesam:url".to_owned(), Value::new(i.uri.clone()));
    if let Some(value) = i.duration {
        buf.insert("mpris:length".to_owned(), Value::new(value.as_micros() as u64));
    }
    // Use filename as title if no title is declared
    if i.title.is_none() {
        buf.insert("xesam:title".to_owned(), Value::new(find_filename_from_relpath(&i.uri).to_owned()));
    }

    Ok(())
}

fn find_filename_from_relpath(i: &str) -> &str {
    if let Some(pos) = i.rfind('/') {
        &i[pos + 1..]
    } else {
        i
    }
}
