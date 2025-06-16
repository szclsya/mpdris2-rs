/// The configuration file format
//use clap::{ArgAction, Parser};
use argh::FromArgs;

#[derive(FromArgs, Debug)]
/// A daemon to expose MPRIS V2.1 D-Bus interface for mpd
pub struct Args {
    /// host address of MPD server
    /// default to `MPD_HOST` or `localhost:6600`
    /// can be a TCP address + port (i.e. localhost:6600), a socket path (i.e. /var/run/mpd/socket),
    /// or an abstract socket (i.e. @mpd_socket)
    /// socket path must be an absolute path, abstract sockets start with @
    #[argh(option, short = 'h')]
    pub host: Option<String>,
    /// disable notification
    #[argh(switch, short = 'n')]
    pub no_notification: bool,
    /// minimum interval between notifications
    #[argh(option, short = 'i', default = "0.1")]
    pub notification_interval: f32,
    /// verbose
    #[argh(switch, short = 'v')]
    pub verbose: bool,
    /// VERY verbose
    #[argh(switch, short = 'V')]
    pub trace: bool,
}
