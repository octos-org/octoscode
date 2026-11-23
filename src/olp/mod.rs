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
pub mod doctor;
pub mod duty;
pub mod evo;
pub mod evoharvest;
pub mod evolib;
pub mod evometrics;
pub mod evoretro;
pub mod forward;
pub mod import;
pub mod inbox;
pub mod init;
pub mod keepalive;
pub mod localpr;
pub mod log;
pub mod mdremote;
pub mod outer;
pub mod outerwatch;
pub mod render;
pub mod repo;
pub mod reviewcargo;
pub mod reviewclassify;
pub mod reviewevidence;
pub mod reviewmonitor;
pub mod reviewpeer;
pub mod reviewstate;
pub mod sentinel;
pub mod serve;
pub mod stall;
pub mod summary;
pub mod unit;
