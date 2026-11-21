//! OctoLoop tools in Rust (docs/OLP_CLI.md): the scripts under `scripts/`
//! move here one command at a time, with the command line, the output and
//! the exit codes unchanged. `board` is the one reader of the blackboard's
//! Markdown; every command shares it.

pub mod board;
pub mod inbox;
