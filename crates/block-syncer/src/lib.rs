#![deny(clippy::arithmetic_side_effects)]
#![deny(clippy::cast_possible_truncation)]
#![deny(unused_crate_dependencies)]
// #![deny(warnings)]

pub mod block_fetcher;
pub mod block_syncer;
mod compression_database;
pub mod config;
