//! UI: ties discovery and the rules engine together.
//!
//! This layer owns everything a person interacts with — CLI parsing, config and
//! keymap loading, the application state machine ([`app`]), and terminal
//! rendering ([`render`]). It depends on [`crate::discovery`] and
//! [`crate::plumber`]; they do not depend on it.

pub mod app;
pub mod cli;
pub mod config;
pub mod filter;
pub mod keymap;
pub mod render;

pub use app::App;
