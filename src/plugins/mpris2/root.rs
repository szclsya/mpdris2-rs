/// The root interface (org.mpris.MediaPlayer2) implementation
use zbus::interface;

#[derive(Default)]
pub struct RootInterface {}

#[interface(name = "org.mpris.MediaPlayer2")]
impl RootInterface {
    #[dbus_interface(name = "Raise")]
    async fn raise(&self) {
        // mpd runs in daemon, do nothing
    }

    #[dbus_interface(name = "Quit")]
    async fn quit(&self) {
        // mpd runs in daemon, do nothing
    }

    #[dbus_interface(property, name = "CanQuit")]
    async fn can_quit(&self) -> bool {
        false
    }

    #[dbus_interface(property, name = "CanSetFullscreen")]
    async fn can_set_fullscreen(&self) -> bool {
        false
    }

    #[dbus_interface(property, name = "CanRaise")]
    async fn can_raise(&self) -> bool {
        false
    }

    #[dbus_interface(property, name = "HasTrackList")]
    async fn has_track_list(&self) -> bool {
        // TODO: Implement TrackList interface and change this to true
        false
    }

    #[dbus_interface(property, name = "Identity")]
    async fn identity(&self) -> &str {
        "Music Player Daemon (mpdris2-rs)"
    }

    #[dbus_interface(property, name = "DesktopEntry")]
    async fn desktop_entry(&self) -> &str {
        "mpdris2-rs"
    }

    #[dbus_interface(property, name = "SupportedUriSchemes")]
    async fn supported_uri_schemes(&self) -> &[&str] {
        &[]
    }

    #[dbus_interface(property, name = "SupportedMimeTypes")]
    async fn supported_mime_types(&self) -> &[&str] {
        &[]
    }
}
