//! `octoscode olp-mcp-serve` (OUTER_LOOP_REVIEW #31): run the OLP-MCP
//! outer-loop server over stdio. Pure Rust port of the Python prototype;
//! see `crate::olp_mcp` for the protocol contract.

/// Run the server; returns the process exit code (0 on clean EOF).
pub fn run() -> i32 {
    let timeout_secs = std::env::var("OLP_MCP_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(crate::olp_mcp::ASK_TIMEOUT_SECS);
    let timeout_secs = clamp_timeout(timeout_secs);
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

/// Keep the wait below the MCP client's tool-call timeout. A wait the client
/// would abort is worse than a shorter one: the inner loop then sees a tool
/// failure instead of the degraded guidance. Non-positive or non-finite
/// values fall back to the default.
fn clamp_timeout(requested: f64) -> f64 {
    use crate::olp_mcp::{ASK_TIMEOUT_CEILING_SECS, ASK_TIMEOUT_SECS};
    if !requested.is_finite() || requested <= 0.0 {
        return ASK_TIMEOUT_SECS;
    }
    if requested > ASK_TIMEOUT_CEILING_SECS {
        eprintln!(
            "olp-mcp-serve: OLP_MCP_TIMEOUT_SECS={requested} exceeds the MCP client's tool-call limit; using {ASK_TIMEOUT_CEILING_SECS}"
        );
        return ASK_TIMEOUT_CEILING_SECS;
    }
    requested
}

#[cfg(test)]
mod tests {
    use super::clamp_timeout;
    use crate::olp_mcp::{ASK_TIMEOUT_CEILING_SECS, ASK_TIMEOUT_SECS};

    /// octos aborts an MCP tool call after 60 s; the wait must end first.
    const MCP_CLIENT_TOOL_CALL_TIMEOUT_SECS: f64 = 60.0;

    #[test]
    fn ask_outer_wait_stays_below_the_mcp_client_tool_call_timeout() {
        const { assert!(ASK_TIMEOUT_SECS < ASK_TIMEOUT_CEILING_SECS) };
        const { assert!(ASK_TIMEOUT_CEILING_SECS < MCP_CLIENT_TOOL_CALL_TIMEOUT_SECS) };
        assert_eq!(clamp_timeout(0.4), 0.4, "tests compress the clock");
        assert_eq!(clamp_timeout(ASK_TIMEOUT_SECS), ASK_TIMEOUT_SECS);
        assert_eq!(clamp_timeout(90.0), ASK_TIMEOUT_CEILING_SECS);
        assert_eq!(clamp_timeout(0.0), ASK_TIMEOUT_SECS);
        assert_eq!(clamp_timeout(-1.0), ASK_TIMEOUT_SECS);
        assert_eq!(clamp_timeout(f64::NAN), ASK_TIMEOUT_SECS);
    }
}
