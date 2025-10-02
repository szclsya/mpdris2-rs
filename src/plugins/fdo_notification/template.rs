use crate::mpd::types::MpdState;

use std::time::Duration;

pub fn format_notification(state: &MpdState, tmpl: &str) -> String {
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
        // Use either title (file) or name (ICY stream)
        res = res
            .replace("%title%", song.title.as_deref().or(song.name.as_deref()).unwrap_or_default());
        // %album%
        res = res.replace("%album%", song.album.as_deref().unwrap_or_default());
        // %album_artist%
        res = res.replace("%album_artist%", song.album_artist.as_deref().unwrap_or_default());
        // %artist%
        res = res.replace("%artist%", song.artist.as_deref().unwrap_or_default());
        // %composer%
        res = res.replace("%composer%", song.composer.as_deref().unwrap_or_default());
        // %genre%
        res = res.replace("%genre%", song.genre.as_deref().unwrap_or_default());
        // %disc%
        res = res.replace("%disc%", song.disc.as_deref().unwrap_or_default());
        // %track%
        res = res.replace("%track%", song.track.as_deref().unwrap_or_default());
        // %comment%
        res = res.replace("%comment%", song.comment.as_deref().unwrap_or_default());
    }

    // Finishing touch: replace all multiple occurrences of \n with a single one
    res = res.replace("\n\n", "\n");
    res = res.replace("\n\n\n", "\n");
    res
}
