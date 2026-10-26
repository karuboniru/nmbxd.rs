pub const APP_NAME: &str = "R岛";
pub const APP_ID: &str = "org.niconi.rdnmb";
pub const USER_AGENT: &str = concat!("rdnmb/", env!("CARGO_PKG_VERSION"));

pub mod api;
pub mod config_paths;
pub mod content;
pub mod forum_cache;
pub mod global_notice;
pub mod history;
pub mod model;
pub mod tree;
pub mod update;
