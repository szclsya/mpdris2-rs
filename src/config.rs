/// The configuration file format
//use clap::{ArgAction, Parser};
use argh::FromArgs;

#[derive(FromArgs, Debug)]
/// A daemon to expose MPRIS V2.1 D-Bus interface for mpd
pub struct Args {
    /// host address of MPD server.
    /// default to `MPD_HOST` or `localhost:6600`.
    /// can be a TCP address + port (i.e. localhost:6600), a socket path (i.e. /var/run/mpd/socket),
    /// or an abstract socket (i.e. @mpd_socket).
    /// socket path must be an absolute path, abstract sockets must start with `@`.
    #[argh(option, short = 'h')]
    pub host: Option<String>,
    /// disable notification
    #[argh(switch, short = 'n')]
    pub no_notification: bool,
    /// notification expiry time
    #[argh(option, short = 't', default = "5.0")]
    pub notification_timeout: f32,
    /// minimum interval between notifications
    #[argh(option, short = 'i', default = "0.1")]
    pub notification_interval: f32,
    /// notification's app name
    #[argh(option, default = "default_app_name()")]
    pub app_name: String,
    /// notification's app icon
    #[argh(option, default = "default_app_icon()")]
    pub app_icon: String,
    /// notification summary (title) template
    #[argh(option, default = "default_summary_template()")]
    pub notification_summary: String,
    /// notification summary (title) template when paused
     #[argh(option, default = "default_paused_summary_template()")]
    pub notification_summary_paused: String,
    /// notification body template
    #[argh(option, default = "default_body_template()")]
    pub notification_body: String,
    /// notification body template when paused
     #[argh(option, default = "default_paused_body_template()")]
    pub notification_body_paused: String,
    /// verbose logging
    #[argh(switch, short = 'v')]
    pub verbose: bool,
    /// VERY verbose logging
    #[argh(switch, short = 'V')]
    pub trace: bool,
}

fn default_app_name() -> String {
    String::from("Music Player Daemon")
}

fn default_app_icon() -> String {
    String::from("/usr/share/icons/hicolor/scalable/apps/mpd.svg")
}

fn default_summary_template() -> String {
    String::from("%state%")
}

fn default_paused_summary_template() -> String {
    default_summary_template()
}

fn default_body_template() -> String {
    String::from("<b>%title%</b>\n%artist%\n%album%")
}

fn default_paused_body_template() -> String {
    default_body_template()
}
