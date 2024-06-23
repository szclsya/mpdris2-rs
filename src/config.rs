/// The configuration file format
//use clap::{ArgAction, Parser};
use argh::FromArgs;

#[derive(FromArgs, Debug)]
/// A daemon to expose MPRIS V2.1 D-Bus interface for mpd
pub struct Args {
    /// address of MPD server (Default: localhost)
    #[argh(option, default = "String::from(\"localhost\")")]
    pub host: String,
    /// port of MPD server (Default: 6600)
    #[argh(option, default = "6600")]
    pub port: u32,
    /// disable notification
    #[argh(switch)]
    pub no_notification: bool,
    /// minimum interval between notifications
    #[argh(option, default = "0.1")]
    pub notification_interval: f32,
    /// verbose
    #[argh(switch, short = 'v')]
    pub verbose: u8,
}
