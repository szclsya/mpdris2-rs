/// The configuration file format
use clap::Parser;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct Args {
    /// Address of MPD server (Default: localhost)
    #[clap(long, default_value = "localhost")]
    pub host: String,
    /// Port of MPD server (Default: 6600)
    #[clap(long, default_value = "6600")]
    pub port: u32,
    /// Disable notification
    #[clap(long)]
    pub no_notification: bool,
    /// Verbosity
    #[clap(short, default_value_t = 0)]
    pub verbose: u8,
}
