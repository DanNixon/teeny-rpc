#![cfg_attr(feature = "no-std", no_std)]
#![allow(clippy::uninlined_format_args)] // Required as defmt cannot use inlined format args

mod fmt;

mod client;
mod error;
mod server;
#[cfg(test)]
mod test;
pub mod transport;
mod wire;

pub use client::Client;
pub use error::Error;
pub use server::Server;
