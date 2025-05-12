/// The root interface (org.mpris.MediaPlayer2) implementation
use zbus::interface;

#[derive(Default)]
pub struct RootInterface {}

#[interface(name = "org.mpris.MediaPlayer2")]
impl RootInterface {
    #[zbus()]
    fn raise(&self) {
        // mpd runs in daemon, do nothing
    }

    #[zbus()]
    fn quit(&self) {
        // mpd runs in daemon, do nothing
    }

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn can_set_fullscreen(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn identity(&self) -> &'static str {
        "Music Player Daemon (mpdris2-rs)"
    }

    #[zbus(property)]
    fn desktop_entry(&self) -> &'static str {
        "mpdris2-rs"
    }

    #[zbus(property)]
    fn supported_uri_schemes(&self) -> &[&str] {
        &[]
    }

    #[zbus(property)]
    fn supported_mime_types(&self) -> &[&str] {
        &[]
    }
}
