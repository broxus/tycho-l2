use std::sync::OnceLock;

pub mod api;
pub mod client;

pub static BIN_VERSION: &str = env!("PROOFS_API_VERSION");
pub static BIN_BUILD: &str = env!("PROOFS_API_BUILD");

pub fn version_string() -> &'static str {
    static STRING: OnceLock<String> = OnceLock::new();
    STRING.get_or_init(|| format!("(release {BIN_VERSION}) (build {BIN_BUILD})"))
}
