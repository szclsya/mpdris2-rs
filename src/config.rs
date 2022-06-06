/// The configuration file format
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Config {
    pub host: String,
    pub port: usize,
    pub notification: bool,
    pub debug: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            host: "127.0.0.1".to_string(),
            port: 6600,
            notification: true,
            debug: false,
        }
    }
}
