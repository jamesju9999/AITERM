//! Manages the Python environment the app's Python-backed features need.
//!
//! Everything runs through the bundled uv binary: it installs an interpreter,
//! creates a venv under app data, and installs per-profile requirements. No
//! feature touches the user's own Python installation.

pub mod paths;
pub mod profiles;
