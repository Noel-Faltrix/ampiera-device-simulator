//! Read-only view of what the Ampiera customer app shows for an installation. The simulator
//! compares these values with the simulated wallbox (scenario S11).
//!
//! Tokens are kept in memory only. Error messages are German and never contain secrets.

mod client;
mod error;
pub mod extract;
mod types;

pub use client::{AppClient, AppClientOptions};
pub use error::AppError;
pub use types::{AppLoginResult, AppSummary, AppViewSnapshot, NextScheduleStep};
