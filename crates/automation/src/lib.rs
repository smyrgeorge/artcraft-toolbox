//! ArtCraft Toolbox for agents (L6; PhotoCraft's `automation` crate, ported):
//!
//! * [`Headless`]: a [`Session`](artcraft_toolbox_engine::Session) driven by the control
//!   protocol's methods (`engine.execute`, `engine.commands`, `jobs.*`, `batch`, …).
//! * [`rpc`]: that protocol as JSON lines over stdio or a loopback TCP port with a bearer token
//!   (`artcraft-toolbox-cli serve`).
//! * [`BridgeClient`]: a client for the desktop app's control channel (the same protocol, served
//!   by `apps/artcraft-toolbox/src/control_server.rs`, handled by `ui_egui::control`).
//! * [`ToolboxMcp`]: an MCP server (the official `rmcp` SDK) exposing the command registry, the
//!   apps' statuses and, in bridge mode, the live window's state.
//! * [`security`] and [`budgets`]: tokens, bounded frames, connection limits, reply ceilings.
//!
//! `docs/control-protocol.md` and `docs/mcp.md` are the contract. No UI toolkit here.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod bridge;
pub mod budgets;
pub mod headless;
pub mod rpc;
pub mod security;
pub mod server;

pub use bridge::BridgeClient;
pub use headless::Headless;
pub use server::{Backend, ToolboxMcp};

#[derive(Debug, thiserror::Error)]
pub enum AutomationError {
    #[error("{0}")]
    BadRequest(String),
    #[error("I/O: {0}")]
    Io(String),
    #[error(transparent)]
    Engine(#[from] artcraft_toolbox_engine::EngineError),
    #[error("bridge: {0}")]
    Bridge(String),
    #[error("app: {0}")]
    App(String),
    #[error("{0}")]
    Other(String),
}
