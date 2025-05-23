use std::sync::OnceLock;

pub mod client;
pub mod config;
pub mod service;

pub mod util {
    pub mod account;
    pub mod getter;
    pub mod jrpc_client;
}

pub static BIN_VERSION: &str = env!("SYNC_SERVICE_VERSION");
pub static BIN_BUILD: &str = env!("SYNC_SERVICE_BUILD");

pub fn version_string() -> &'static str {
    static STRING: OnceLock<String> = OnceLock::new();
    STRING.get_or_init(|| format!("(release {BIN_VERSION}) (build {BIN_BUILD})"))
}
