//! Minimal GitHub Releases client for `update --check` and `doctor`.
//!
//! A plain blocking `reqwest` GET against the public Releases API — no auth is
//! required for public repos, but `OCTOSCODE_GITHUB_TOKEN` is honored to dodge
//! the unauthenticated rate limit (design §A.2 / Risks).

use std::time::Duration;

use eyre::{Result, WrapErr, eyre};
use serde::Deserialize;

/// `owner/name` slug for the released TUI binary.
pub const GITHUB_REPO: &str = "octos-org/octoscode";

const RELEASES_LATEST_URL: &str =
    "https://api.github.com/repos/octos-org/octoscode/releases/latest";
const RELEASES_URL: &str = "https://api.github.com/repos/octos-org/octoscode/releases";
const API_BASE: &str = "https://api.github.com";
const USER_AGENT: &str = concat!("octoscode/", env!("CARGO_PKG_VERSION"));
const TIMEOUT: Duration = Duration::from_secs(10);

/// The release info `update`/`doctor` care about.
#[derive(Debug, Clone)]
pub struct LatestRelease {
    /// The release tag, e.g. `v0.1.2`.
    pub tag: String,
    /// Whether GitHub marked this release as a prerelease.
    pub prerelease: bool,
}

#[derive(Debug, Deserialize)]
struct ReleasePayload {
    tag_name: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
}

fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(TIMEOUT)
        .build()
        .wrap_err("failed to build HTTP client")
}

/// The GitHub token from `OCTOSCODE_GITHUB_TOKEN`, if set and non-blank.
/// Shared with the self-update path so axoupdater honors the same token.
pub(crate) fn token() -> Option<String> {
    std::env::var("OCTOSCODE_GITHUB_TOKEN")
        .ok()
        .filter(|t| !t.trim().is_empty())
}

fn authed(req: reqwest::blocking::RequestBuilder) -> reqwest::blocking::RequestBuilder {
    let req = req
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28");
    match token() {
        Some(t) => req.bearer_auth(t),
        None => req,
    }
}

/// Query the target release for the requested channel. `prerelease_channel`
/// selects the newest non-draft prerelease; otherwise this returns the latest
/// stable release from GitHub's dedicated `/releases/latest` endpoint.
///
/// Returns `Ok(None)` when the selected channel has no published releases
/// (GitHub answers `404` on `/releases/latest` for an empty stable channel, and
/// the releases list contains no matching entry for an empty prerelease
/// channel). That is not an error. A real failure (network, 5xx, rate-limit)
/// still surfaces as `Err`.
pub fn latest_release(prerelease_channel: bool) -> Result<Option<LatestRelease>> {
    let client = client()?;

    if prerelease_channel {
        return newest_prerelease(&client);
    }

    let resp = authed(client.get(RELEASES_LATEST_URL))
        .send()
        .wrap_err("failed to reach api.github.com")?;
    let status = resp.status();
    if status.as_u16() == 404 {
        return Ok(None);
    }
    if !status.is_success() {
        return Err(eyre!(
            "GitHub returned {status} for the latest octoscode release"
        ));
    }
    let payload: ReleasePayload = resp
        .json()
        .wrap_err("failed to decode GitHub release payload")?;
    Ok(Some(LatestRelease {
        tag: payload.tag_name,
        prerelease: payload.prerelease,
    }))
}

/// Newest non-draft prerelease (first matching entry of `/releases`, which
/// GitHub returns newest-first). Stable releases are deliberately skipped: an
/// explicit prerelease-channel request must not snap back to stable merely
/// because stable has higher SemVer precedence.
fn newest_prerelease(client: &reqwest::blocking::Client) -> Result<Option<LatestRelease>> {
    let resp = authed(client.get(RELEASES_URL))
        .query(&[("per_page", "100")])
        .send()
        .wrap_err("failed to reach api.github.com")?;
    let status = resp.status();
    if !status.is_success() {
        return Err(eyre!(
            "GitHub returned {status} while querying octoscode prereleases"
        ));
    }
    let payloads: Vec<ReleasePayload> = resp
        .json()
        .wrap_err("failed to decode GitHub releases list")?;
    Ok(select_newest_prerelease(payloads))
}

fn select_newest_prerelease(payloads: Vec<ReleasePayload>) -> Option<LatestRelease> {
    payloads
        .into_iter()
        .find(|r| !r.draft && r.prerelease)
        .map(|r| LatestRelease {
            tag: r.tag_name,
            prerelease: r.prerelease,
        })
}

/// Whether `api.github.com` is reachable (a cheap GET against the API root).
/// Used by `doctor`'s network check; surfaces 403 rate-limit distinctly so the
/// caller can warn rather than fail.
pub fn reachability() -> Reachability {
    let client = match client() {
        Ok(c) => c,
        Err(_) => return Reachability::Unreachable("failed to build HTTP client".into()),
    };
    match authed(client.get(API_BASE)).send() {
        Ok(resp) if resp.status().is_success() => Reachability::Ok,
        Ok(resp) if resp.status().as_u16() == 403 => Reachability::RateLimited,
        Ok(resp) => Reachability::Unreachable(format!("HTTP {}", resp.status())),
        Err(err) => Reachability::Unreachable(err.to_string()),
    }
}

/// Result of the GitHub reachability probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reachability {
    /// Reachable.
    Ok,
    /// Reachable but rate-limited (403) — warn, don't fail.
    RateLimited,
    /// Not reachable (network/proxy/DNS).
    Unreachable(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, prerelease: bool, draft: bool) -> ReleasePayload {
        ReleasePayload {
            tag_name: tag.to_string(),
            prerelease,
            draft,
        }
    }

    #[test]
    fn prerelease_channel_skips_newer_stable_and_draft_entries() {
        let selected = select_newest_prerelease(vec![
            release("v0.3.0", false, false),
            release("v0.3.0-rc.9", true, true),
            release("v0.3.0-rc.8", true, false),
        ])
        .expect("published prerelease");

        assert_eq!(selected.tag, "v0.3.0-rc.8");
        assert!(selected.prerelease);
    }

    #[test]
    fn prerelease_channel_returns_none_without_a_published_prerelease() {
        assert!(
            select_newest_prerelease(vec![
                release("v0.3.0", false, false),
                release("v0.4.0-rc.1", true, true),
            ])
            .is_none()
        );
    }
}
