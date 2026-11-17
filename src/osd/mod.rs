//! Shared logic for the on-screen visualizer binaries.
//!
//! Both `voxtype-osd-native` (SCTK + wgpu + egui-wgpu) and `voxtype-osd-gtk4`
//! (GTK4 + gtk4-layer-shell) consume the same daemon IPC, run the same
//! peak-hold + waveform envelope math, parse the same Omarchy theme, and
//! honor the same `[osd]` configuration. That logic lives here so the two
//! frontends only differ in their rendering surface.
//!
//! ## Module layout
//!
//! - [`ipc`] — Unix-socket connection, frame decode, ring buffer, reconnect.
//! - [`visual`] — peak-hold decay, waveform envelope helpers, palette types.
//! - [`config`] — `[osd]` config block (`OsdConfig`).
//! - [`theme`] — Omarchy theme parsing + change watcher.
//! - [`shell_style`] — opt-in Omarchy Shell popup chrome for voice strips.
//! - [`gtk_env`] — environment workarounds for GTK4 bugs (GTK frontend only).
//! - [`session`] — runtime files describing the session in flight, for OSDs.

pub mod config;
pub mod gtk_env;
pub mod ipc;
pub mod recipe;
pub mod session;
pub mod shell_style;
pub mod style;
pub mod supervisor;
pub mod theme;
pub mod visual;
