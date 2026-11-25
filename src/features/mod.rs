//! Feature modules.
//!
//! Each of the five user-facing features has one owner module. `core.rs` is the
//! orchestrator and should only pass an input frame plus shared game state to
//! these modules.

pub mod combat_art;
pub mod emblem_cap;
pub mod flow;
pub mod paper_guard;
pub mod prosthetic;
