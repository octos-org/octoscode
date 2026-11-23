//! OctoLoop tools in Rust (docs/OLP_CLI.md): the scripts under `scripts/`
//! move here one command at a time, with the command line, the output and
//! the exit codes unchanged. `board` is the one reader of the blackboard's
//! Markdown; every command shares it. `log` is the JSON Lines event log
//! that replaces that Markdown as the place data is kept.

pub mod answer;
pub mod append;
pub mod archive;
pub mod board;
pub mod card;
pub mod cards;
pub mod client;
pub mod dispatch;
pub mod duty;
pub mod import;
pub mod inbox;
pub mod log;
pub mod render;
pub mod repo;
pub mod sentinel;
pub mod serve;
pub mod stall;
pub mod summary;
pub mod unit;
