use anyhow::Result;
use log::error;
use std::{collections::HashMap, time::Duration};
use zvariant::{ObjectPath, Value};

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

pub fn to_mpris_metadata<'a>(
    mut i: HashMap<String, Vec<String>>,
) -> Result<HashMap<String, Value<'a>>> {
    let mut res = HashMap::new();

    let i = &mut i;
    let r = &mut res;
    if let Some(id) = i.remove("Id") {
        let object_id = id_to_object_path(&id[0]);
        r.insert("mpris:trackid".to_string(), Value::new(object_id));
    }
    if let Some(length) = i.remove("duration") {
        let length: f64 = length[0].parse()?;
        let t = Duration::from_secs_f64(length);
        r.insert("mpris:length".to_owned(), Value::new(t.as_micros() as u64));
    }
    // TODO: Create URI
    convert_str_tag(i, r, "Album", "xesam:album");
    convert_str_array_tag(i, r, "AlbumArtist", "xesam:albumArtist");
    convert_str_array_tag(i, r, "Artist", "xesam:artist");
    // TODO: Lyrics
    convert_str_array_tag(i, r, "Comment", "xesam:comment");
    convert_str_array_tag(i, r, "Composer", "xesam:composer");
    convert_str_tag(i, r, "Disc", "xesam:discNumber");
    convert_str_tag(i, r, "Genre", "xesam:genre");
    convert_str_tag(i, r, "Title", "xesam:title");
    convert_int_tag(i, r, "Track", "xesam:trackNumber");
    convert_str_tag(i, r, "file", "xesam:url");
    Ok(res)
}

fn convert_str_tag(
    i: &mut HashMap<String, Vec<String>>,
    res: &mut HashMap<String, Value>,
    mpd_key: &str,
    mpris_key: &str,
) {
    if let Some(mut value) = i.remove(mpd_key) {
        let r = value.remove(0);
        res.insert(mpris_key.to_owned(), Value::new(r));
    }
}

fn convert_str_array_tag(
    i: &mut HashMap<String, Vec<String>>,
    res: &mut HashMap<String, Value>,
    mpd_key: &str,
    mpris_key: &str,
) {
    if let Some(value) = i.remove(mpd_key) {
        res.insert(mpris_key.to_owned(), Value::new(value));
    }
}

fn convert_int_tag(
    i: &mut HashMap<String, Vec<String>>,
    res: &mut HashMap<String, Value>,
    mpd_key: &str,
    mpris_key: &str,
) {
    if let Some(value) = i.remove(mpd_key) {
        let value = &value[0];
        if let Ok(r) = value.parse::<i64>() {
            res.insert(mpris_key.to_owned(), Value::new(r));
        } else {
            error!("Can't parse metadata tag {mpd_key} -> {mpris_key} with value {value}");
        }
    }
}
