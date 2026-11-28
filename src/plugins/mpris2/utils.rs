use std::collections::HashMap;
use zbus::{ObjectPath, Variant};

use crate::types::SongMetadata;

pub fn id_to_object_path<'a>(id: impl std::fmt::Display) -> ObjectPath<'a> {
    let path = format!("/org/musicpd/song/{id}");
    ObjectPath::try_from(path).unwrap()
}

pub fn object_path_to_id(path: &ObjectPath) -> Option<u64> {
    if let Some(n) = path.strip_prefix("/org/musicpd/song/")
        && let Ok(n) = n.parse()
    {
        return Some(n);
    }
    None
}

pub fn to_mpris_metadata(i: &SongMetadata, buf: &mut HashMap<&str, Variant<'_>>) {
    let mut optional_insert = |tag: &'static str, src: &Option<String>| {
        if let Some(value) = src {
            buf.insert(tag, Variant::new(value.to_owned()));
        }
    };
    optional_insert("xesam:title", &i.title);
    optional_insert("xesam:album", &i.album);

    // MPD tags are raw ("12/16"); take the leading digits, omit if there are none.
    let mut optional_insert_leading_int = |tag: &'static str, src: &Option<String>| {
        if let Some(value) = src {
            let digits: String =
                value.trim_start().chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = digits.parse::<i32>() {
                buf.insert(tag, Variant::new(n));
            }
        }
    };
    optional_insert_leading_int("xesam:discNumber", &i.disc);
    optional_insert_leading_int("xesam:trackNumber", &i.track);

    let mut optional_insert_array = |tag: &'static str, src: &Vec<String>| {
        if !src.is_empty() {
            buf.insert(tag, Variant::new(src.to_owned()));
        }
    };
    optional_insert_array("xesam:composer", &i.composer);
    optional_insert_array("xesam:artist", &i.artist);
    optional_insert_array("xesam:albumArtist", &i.album_artist);
    optional_insert_array("xesam:genre", &i.genre);
    optional_insert_array("xesam:comment", &i.comment);

    // Special types
    buf.insert("mpris:trackid", Variant::new(id_to_object_path(i.id)));
    buf.insert("xesam:url", Variant::new(i.uri.clone()));
    if let Some(value) = i.duration {
        buf.insert("mpris:length", Variant::new(value.as_micros() as i64));
    }
    // Use filename as title if no title is declared
    if i.title.is_none() {
        buf.insert("xesam:title", Variant::new(find_filename_from_relpath(&i.uri).to_owned()));
    }
}

fn find_filename_from_relpath(i: &str) -> &str {
    if let Some(pos) = i.rfind('/') { &i[pos + 1..] } else { i }
}
