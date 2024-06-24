/// The root interface (org.mpris.MediaPlayer2) implementation
use zbus::interface;

#[derive(Default)]
pub struct RootInterface {}

#[interface(name = "org.mpris.MediaPlayer2")]
impl RootInterface {
    #[zbus()]
    async fn raise(&self) {
        // mpd runs in daemon, do nothing
    }

    #[zbus()]
    async fn quit(&self) {
        // mpd runs in daemon, do nothing
    }

    #[zbus(property)]
    async fn can_quit(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn can_set_fullscreen(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn can_raise(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn has_track_list(&self) -> bool {
        true
    }

    #[zbus(property)]
    async fn identity(&self) -> &str {
        "Music Player Daemon (mpdris2-rs)"
    }

    #[zbus(property)]
    async fn desktop_entry(&self) -> &str {
        "mpdris2-rs"
    }

    #[zbus(property)]
    async fn supported_uri_schemes(&self) -> &[&str] {
        &[]
    }

    #[zbus(property)]
    async fn supported_mime_types(&self) -> &[&str] {
        &[]
    }
}
