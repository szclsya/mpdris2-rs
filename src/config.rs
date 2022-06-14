/// The configuration file format
use clap::{ArgAction, Parser};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
/// A daemon to expose MPRIS V2.1 D-Bus interface for mpd
pub struct Args {
    /// Address of MPD server (Default: localhost)
    #[clap(action, long, default_value = "localhost")]
    pub host: String,
    /// Port of MPD server (Default: 6600)
    #[clap(action, long, default_value = "6600")]
    pub port: u32,
    /// Disable notification
    #[clap(action, long)]
    pub no_notification: bool,
    /// Verbosity
    #[clap(action(ArgAction::Count), short)]
    pub verbose: u8,
}
