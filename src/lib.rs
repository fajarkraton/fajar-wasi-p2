//! fajar-wasi-p2 — WASI Preview 2 (Component Model) for Fajar Lang.
//!
//! Extracted from fajar-lang per Compass §5.1.
//! Skeleton stage — source files will land at Phase E.2.

#![doc(html_root_url = "https://docs.rs/fajar-wasi-p2/0.1.0")]

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skeleton_version_matches_cargo() {
        assert_eq!(version(), "0.1.0");
    }
}
