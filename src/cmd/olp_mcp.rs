//! `octoscode olp-mcp-serve` (OUTER_LOOP_REVIEW #31): run the OLP-MCP
//! outer-loop server over stdio. Pure Rust port of the Python prototype;
//! see `crate::olp_mcp` for the protocol contract.

/// Run the server; returns the process exit code (0 on clean EOF).
pub fn run() -> i32 {
    let timeout_secs = std::env::var("OLP_MCP_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(crate::olp_mcp::ASK_TIMEOUT_SECS);
    let stdin = std::io::stdin().lock();
    let stdout = std::io::stdout().lock();
    match crate::olp_mcp::serve(stdin, stdout, timeout_secs) {
        Ok(_) => 0,
        Err(err) => {
            eprintln!("olp-mcp-serve: {err}");
            1
        }
    }
}
