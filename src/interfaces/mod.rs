mod root;
pub use root::RootInterface;
pub mod player;
pub use player::PlayerInterface;
pub mod tracklist;
pub use tracklist::TracklistInterface;

mod types;
pub use types::MprisStateChange;

mod utils;
