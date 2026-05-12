//! fajar-wasi-p2 — WASI Preview 2 (Component Model) for Fajar Lang.
//!
//! Full WIT parser, type system, component binary format, and WASI P2 interfaces.
//! Originally built on top of fajar-lang's V12 WASI P1 (8 syscalls wired into wasm compiler).
//!
//! Extracted from fajar-lang per Compass §5.1.
//!
//! ## Module Organization
//! - `wit_lexer` — Tokenizer for `.wit` files
//! - `wit_parser` — Recursive-descent parser producing `WitDocument`
//! - `wit_types` — WIT-to-Fajar type mapping and type system
//! - `resources` — Resource lifecycle, handle table, own/borrow semantics
//! - `deployment` — Validation, benchmarks, conformance, and deployment tooling (W10)

#![doc(html_root_url = "https://docs.rs/fajar-wasi-p2/0.1.0")]
// Nightly clippy allow-list — lints that differ between stable and nightly.
// Mirrors fajar-lang's src/lib.rs allow-list at extraction time.
#![allow(clippy::collapsible_if)]

pub mod component;
pub mod composition;
pub mod deployment;
pub mod filesystem;
pub mod http;
pub mod resources;
pub mod sockets;
pub mod streams;
pub mod wit_lexer;
pub mod wit_parser;
pub mod wit_types;
