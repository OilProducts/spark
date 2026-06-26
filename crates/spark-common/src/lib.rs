#![forbid(unsafe_code)]

//! Shared Spark primitives for the Rust rewrite.
//!
//! This foundation crate owns compatibility-oriented runtime path, settings,
//! source checkout, project identity, process output, logging, and event
//! boundaries.

pub mod error;
pub mod events;
pub mod logging;
pub mod paths;
pub mod process;
pub mod project;
pub mod settings;
pub mod source_checkout;

pub use error::{Result, SparkCommonError};
