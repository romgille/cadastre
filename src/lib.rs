//! cadastre keeps a folder of Markdown up to date with what your Docker
//! containers are running.
//!
//! The crate is split in small modules with one job each. The binary in
//! `main.rs` only wires them together.

pub mod changelog;
pub mod config;
pub mod diff;
pub mod fsutil;
pub mod log;
pub mod model;
pub mod output;
pub mod render;
pub mod sanitize;
pub mod snapshot;
pub mod timefmt;

/// Version of the running build, taken from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::VERSION;

    #[test]
    fn version_looks_like_semver() {
        assert_eq!(
            VERSION.split('.').count(),
            3,
            "unexpected version {VERSION}"
        );
    }
}
