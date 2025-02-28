/// The configuration file format
//use clap::{ArgAction, Parser};
use argh::FromArgs;

#[derive(FromArgs, Debug)]
/// A daemon to expose MPRIS V2.1 D-Bus interface for mpd
pub struct Args {
    /// host address of MPD server
    /// default to `MPD_HOST` or `localhost:6600`
    /// can be a TCP address + port (i.e. localhost:6600) or a socket path (i.e. /var/run/mpd/socket)
    /// socket path must be an absolute path
    #[argh(option)]
    pub host: Option<String>,
    /// port of TCP connection (deprecated, add port in --host)
    #[argh(option, hidden_help)]
    pub port: Option<u16>,
    /// disable notification
    #[argh(switch)]
    pub no_notification: bool,
    /// minimum interval between notifications
    #[argh(option, default = "0.1")]
    pub notification_interval: f32,
    /// verbose
    #[argh(switch, short = 'v')]
    pub verbose: bool,
    /// VERY verbose
    #[argh(switch, short = 'V')]
    pub trace: bool,
}
