spec: task
name: "provider account provisioning runbook and capped live-spend probe"
inherits: project
tags: [doctor, providers, billing, onboarding, quality-gate]
depends: []
estimate: 1.5d
---

## Intent

octoscode routes turns through several LLM provider accounts (openai, moonshot/kimi,
zai/glm, minimax, deepseek, anthropic, openrouter), but nothing proves a saved
account is actually usable: onboarding accepts a key that may be malformed, revoked,
out of credit, or on a plan that does not carry the selected model, and the user only
discovers this mid-turn. This task adds two halves — a human runbook for signing up
and funding one account per provider, and `octoscode doctor --live-provider`, which
spends a hard-capped, sub-cent amount against each configured account to classify it
as usable or to say precisely why it is not. This is distinct from the existing
AppUI WebSocket capability probe (`task-doctor-live-capability-probe-and-quality-gate`),
which never touches provider billing.

## Decisions

- Signup and payment are **operator steps, not code**. `docs/provider-accounts.md`
  is a runbook: per provider, the console URL, where the API key is minted, the
  minimum top-up accepted, and which env var or profile route octoscode reads.
  No code in this task opens a browser, fills a signup form, submits payment
  details, or stores a payment method.
- One account per provider. The probe iterates the providers present in the
  resolved profile's `profile_llm_state`, not a hardcoded list, and introduces
  **no new env var names** — key resolution reuses the existing onboarding/profile
  path.
- The probe issues **at most one** completion request per provider per invocation:
  a fixed prompt, `max_tokens = 16`, temperature 0, non-streaming. It is a
  billability check, not a benchmark.
- Spend cap: `--max-spend-usd <f64>`, default `0.05` per provider per invocation.
  The estimated cost of the request is computed from the provider's configured
  price table **before** sending; if the estimate exceeds the cap the request is
  not sent.
- Verdict taxonomy, one per provider, mapped onto the existing `CheckStatus`:

  | verdict             | trigger                                   | CheckStatus |
  | ok                  | 2xx with a usage block                    | pass        |
  | unauthorized        | 401 / 403                                 | fail        |
  | unfunded            | 402, or 429 with insufficient_quota        | fail        |
  | model_unavailable   | 404 / model_not_found                     | fail        |
  | unreachable         | connect, DNS, or TLS error; timeout        | fail        |
  | over_cap            | pre-send estimate exceeds `--max-spend-usd`| warn        |
  | unconfigured        | no key resolved for that provider          | warn        |

- `unfunded` and `unauthorized` are separate verdicts. Collapsing them is the
  specific failure this task exists to prevent — "your card ran out" and "your key
  is wrong" have different fixes.
- The probe's base URL is injectable via `ProviderProbeTransport`. Contract tests
  do not mock the HTTP layer: they bind a loopback `std::net::TcpListener` stub on
  port 0 that serves canned status codes and bodies, and point the probe at it —
  the same pattern the AppUI protocol tests already use. If the environment forbids
  a loopback bind the test skips early rather than panicking. Exactly one
  `#[ignore]`-marked test hits a real provider endpoint and spends real money.
- API keys are redacted at the formatting layer: rendered text, `--json`, and every
  error string carry at most the last 4 characters, never the key.
- Reuses the existing `Check` / `Report` / `--json` / `--strict` / `exit_code`
  machinery in `src/cmd/doctor.rs` under a new `CAT_PROVIDERS` category. No new
  reporting surface, no new dependencies beyond the `reqwest` already in the tree.
- Without `--live-provider`, `doctor` behavior is byte-identical to today. The flag
  is opt-in because it spends money.

## Boundaries

### Allowed Changes
- src/cmd/doctor.rs
- src/cmd/provider_probe.rs
- src/cmd/mod.rs
- docs/provider-accounts.md
- tests/provider_live_probe_contract.rs
- tests/docs_drift.rs
- locales/en.yml
- locales/zh.yml

### Forbidden
- Do not automate account signup, ToS acceptance, CAPTCHA solving, phone or
  identity verification, or payment submission for any provider.
- Do not create more than one account per provider, and do not add any code path
  that rotates between multiple accounts of the same provider.
- Do not commit an API key, a funded account credential, or a `.env` file.
- Do not send a provider request when `--live-provider` is absent.
- Do not modify `src/transport.rs` or `src/menu/providers.rs`.
- Do not add a new runtime dependency.

## Out of Scope

- Automated signup or payment for any provider.
- Multiple accounts per provider and failover between them.
- Per-account billing dashboards, spend history, or budget alerting over time.
- Latency or quality benchmarking across providers.
- Changing how onboarding stores or validates keys.

## Completion Criteria

Scenario: funded account probes clean
  Test: test_funded_account_reports_ok
  Level: integration
  Test Double: loopback `std::net::TcpListener` stub on port 0
  Given a configured provider whose key resolves
  And the transport returns 200 with a usage block of 12 prompt and 9 completion tokens
  When doctor runs with "--live-provider"
  Then the provider verdict is "ok"
  And the check status is "pass"
  And the reported cost is derived from the 21 returned tokens

Scenario: json output carries one entry per configured provider
  Test: test_live_provider_json_shape
  Given two configured providers and one unconfigured provider
  When doctor runs with "--live-provider --json"
  Then the providers category contains "3" entries
  And each entry contains the keys "provider", "verdict", "tokens", "cost_usd"

Scenario: runbook lists every provider the probe knows about
  Test: test_provider_runbook_covers_all_probed_providers
  Given the provider identifiers the probe can resolve
  When the docs drift check reads "docs/provider-accounts.md"
  Then every identifier appears in the runbook with a console URL and a minimum top-up

Scenario: revoked key is reported as unauthorized
  Test: test_revoked_key_reports_unauthorized
  Given a configured provider
  And the transport returns 401
  When doctor runs with "--live-provider"
  Then the provider verdict is "unauthorized"
  And the check status is "fail"
  And the transport receives exactly "1" request

Scenario: exhausted credit is distinguished from a bad key
  Test: test_exhausted_credit_reports_unfunded
  Given a configured provider
  And the transport returns the following responses:
    | status | body_code           | expected_verdict |
    | 402    | payment_required    | unfunded         |
    | 429    | insufficient_quota  | unfunded         |
    | 429    | rate_limit_exceeded | unreachable      |
  When doctor runs with "--live-provider"
  Then each response produces its expected verdict
  And no response produces the verdict "unauthorized"

Scenario: model absent from the account plan is named as such
  Test: test_plan_missing_model_reports_model_unavailable
  Given a configured provider whose selected model is not on the account plan
  And the transport returns 404 with code "model_not_found"
  When doctor runs with "--live-provider"
  Then the provider verdict is "model_unavailable"
  And the detail names the model that was requested

Scenario: an estimate above the cap sends no request
  Test: test_estimate_over_cap_sends_no_request
  Given a configured provider
  When doctor runs with "--live-provider --max-spend-usd 0.0"
  Then the provider verdict is "over_cap"
  And the check status is "warn"
  And the transport receives exactly "0" requests

Scenario: an unconfigured provider is skipped rather than failed
  Test: test_unconfigured_provider_is_skipped
  Given a provider present in the profile with no key resolved
  When doctor runs with "--live-provider" without "--strict"
  Then the provider verdict is "unconfigured"
  And the check status is "warn"
  And the process exit code is "0"

Scenario: the api key never reaches any output surface
  Test: test_api_key_is_redacted_everywhere
  Given a configured provider whose key is "sk-live-000000000000abcd"
  And the transport returns 401 with the key echoed in the error body
  When doctor runs with "--live-provider --json"
  Then stdout does not contain "sk-live-000000000000abcd"
  And stderr does not contain "sk-live-000000000000abcd"
  And the rendered detail contains "abcd"

Scenario: a network failure is not reported as an auth failure
  Test: test_network_error_reports_unreachable
  Level: integration
  Test Double: loopback `std::net::TcpListener` stub, closed before the request
  Given a configured provider
  And the stub is bound then closed so the connection is refused
  When doctor runs with "--live-provider"
  Then the provider verdict is "unreachable"
  And the check status is "fail"

Scenario: doctor without the flag spends nothing
  Test: test_doctor_without_flag_sends_no_provider_request
  Given two configured providers with resolvable keys
  When doctor runs without "--live-provider"
  Then the transport receives exactly "0" requests
  And the report contains no providers category

Scenario: contract tests resolve every verdict against a loopback http stub
  Test: test_probe_is_driven_by_loopback_stub
  Level: integration
  Test Double: loopback `std::net::TcpListener` stub on port 0
  Given a loopback stub bound on port 0 serving canned status codes and bodies
  And `ProviderProbeTransport` pointed at the stub base URL
  When the contract suite runs with the ignored live-spend test excluded
  Then every verdict is resolved from a real HTTP exchange with the stub
  And no request leaves the loopback interface

Scenario: a loopback bind refusal skips rather than panics
  Test: test_stub_bind_refusal_skips_early
  Level: integration
  Test Double: loopback `std::net::TcpListener` stub on port 0
  Given an environment that refuses to bind a loopback listener
  When the contract suite starts the stub
  Then the affected test reports skipped
  And no thread panics

Scenario: one real account is charged end to end
  Test: test_live_spend_smoke_ignored_by_default
  Level: e2e
  Test Double: none, the real `ProviderProbeTransport` implementation
  Given the test is marked ignore and runs only when invoked by name
  And a real funded account for one provider
  When the probe runs against the live endpoint with the default cap
  Then the verdict is "ok"
  And the recorded cost is below "0.05"
