//! UI: ties discovery and the rules engine together.
//!
//! This layer owns everything a person interacts with — CLI parsing, config and
//! keymap loading, the application state machine ([`app`]), and terminal
//! rendering ([`render`]). It depends on [`crate::discovery`] and
//! [`crate::plumber`]; they do not depend on it.

pub mod app;
pub mod cli;
pub mod config;
pub(crate) mod display;
pub mod filter;
pub mod keymap;
pub(crate) mod layout;
pub mod render;
pub(crate) mod viewport;

pub use app::App;
