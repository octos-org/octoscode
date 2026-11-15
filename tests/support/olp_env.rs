//! Included by every integration test file (`#[path = "support/olp_env.rs"]
//! mod olp_env;`). Before the test harness starts — still single-threaded —
//! it drops every `OLP_*` variable the test process inherited, so the tools a
//! test runs can only reach the sandbox the test builds and passes to them
//! explicitly. Retrospective #15 (2026-11-15): a test with a temporary HOME
//! inherited `OLP_UNIT_CONF` from a configured unit and wrote a synthetic ACK
//! to the production hub board. `OLP_MCP_SERVER_BIN` stays: it selects the
//! binary under test and routes nowhere.

/// Inherited variables kept: test knobs that select what is tested.
pub const KEPT: &[&str] = &["OLP_MCP_SERVER_BIN"];

#[used]
#[allow(unsafe_code)]
#[cfg_attr(target_os = "linux", unsafe(link_section = ".init_array"))]
#[cfg_attr(target_os = "macos", unsafe(link_section = "__DATA,__mod_init_func"))]
#[cfg_attr(windows, unsafe(link_section = ".CRT$XCU"))]
static DROP_INHERITED_OLP_ENV: extern "C" fn() = drop_inherited_olp_env;

#[allow(unsafe_code)]
extern "C" fn drop_inherited_olp_env() {
    let names: Vec<std::ffi::OsString> = std::env::vars_os()
        .map(|(name, _)| name)
        .filter(|name| {
            let name = name.to_string_lossy();
            name.starts_with("OLP_") && !KEPT.contains(&name.as_ref())
        })
        .collect();
    for name in names {
        // SAFETY: runs from the binary's initializer list, before `main` and
        // before any thread exists.
        unsafe { std::env::remove_var(name) }
    }
}
