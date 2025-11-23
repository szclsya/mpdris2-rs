use crate::mpd::types::MpdState;

use std::{borrow::Cow, time::Duration};

const FALLBACK_MESSAGE: &str = "Empty Playlist";

pub fn format_notification(state: &MpdState, tmpl: &str, max_seg_len: usize) -> String {
    // Un-escape new lines. They are allowed in notification body
    let mut res = tmpl.replace("\\n", "\n");
    // States that will always be there
    res = res.replace("%state%", state.playback_state.as_str());
    res = res.replace("%loop%", state.loop_state.as_str());
    // %elapsed%
    // Normalized to secs
    res = res.replace(
        "%elapsed%",
        &state
            .playback_state
            .get_elapsed()
            .map(|d| humantime::format_duration(Duration::from_secs(d.as_secs())).to_string())
            .unwrap_or_default(),
    );
    if state.random {
        res = res.replace("%random%", "Random");
    } else {
        res = res.replace("%random%", "");
    }

    // song metadata
    if let Some(song) = &state.current_song {
        // %uri%
        res = res.replace("%uri%", &song.uri);
        // %id%
        res = res.replace("%id%", &song.id.to_string());
        // %duration%
        // Normalized to secs
        res = res.replace(
            "%duration%",
            &song
                .duration
                .map(|d| humantime::format_duration(Duration::from_secs(d.as_secs())).to_string())
                .unwrap_or_default(),
        );
        // %title%
        // Use either title (file) or name (ICY stream) or filename if neither exists
        let title = song.title.as_deref().or(song.name.as_deref()).unwrap_or(&song.uri);
        res = res.replace("%title%", &str_clamp(title, max_seg_len));
        // %album%
        res = res.replace("%album%", &str_clamp(song.album.as_deref().unwrap_or_default(), max_seg_len));
        // %album_artist%
        res = res.replace("%album_artist%", &str_clamp(song.album_artist.as_deref().unwrap_or_default(), max_seg_len));
        // %artist%
        res = res.replace("%artist%", &str_clamp(song.artist.as_deref().unwrap_or_default(), max_seg_len));
        // %composer%
        res = res.replace("%composer%", &str_clamp(song.composer.as_deref().unwrap_or_default(), max_seg_len));
        // %genre%
        res = res.replace("%genre%", &str_clamp(song.genre.as_deref().unwrap_or_default(), max_seg_len));
        // %disc%
        res = res.replace("%disc%", &str_clamp(song.disc.as_deref().unwrap_or_default(), max_seg_len));
        // %track%
        res = res.replace("%track%", &str_clamp(song.track.as_deref().unwrap_or_default(), max_seg_len));
        // %comment%
        res = res.replace("%comment%", &str_clamp(song.comment.as_deref().unwrap_or_default(), max_seg_len));
    } else {
        // null replaces
        res = res.replace("%uri%", "");
        res = res.replace("%id%", "");
        res = res.replace("%duration%", "");
        res = res.replace("%title%", "");
        res = res.replace("%album%", "");
        res = res.replace("%album_artist%", "");
        res = res.replace("%artist%", "");
        res = res.replace("%composer%", "");
        res = res.replace("%genre%", "");
        res = res.replace("%disc%", "");
        res = res.replace("%track%", "");
        res = res.replace("%comment%", "");
    }

    res = res.replace("%fallback%", FALLBACK_MESSAGE);

    res = res.replace("<b></b>", "");
    res = res.replace("<i></i>", "");
    res = res.trim_start_matches("\n").to_string();

    res
}

fn str_clamp(s: &str, max_len: usize) -> Cow<'_, str> {
    if s.len() < max_len {
        s.into()
    } else {
        let mut res = s[..max_len-3].to_string();
        res.push_str("...");
        res.into()
    }
}
