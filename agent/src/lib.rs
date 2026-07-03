//! Módulos compartidos por los binarios del agente.

pub mod account;
pub mod autostart;
pub mod capture;
pub mod chat;
pub mod config;
pub mod decode;
pub mod device;
pub mod encode;
pub mod files;
pub mod input;
pub mod monitors;
pub mod proto;
pub mod session;
#[cfg(windows)]
pub mod tray;
pub mod ui;
pub mod update;
pub mod viewer;
#[cfg(windows)]
pub mod winsvc;
