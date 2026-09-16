# Contributing to octoscode

Thank you for helping improve octoscode. Contributions of code, tests,
documentation, design feedback, and reproducible bug reports are welcome.

By participating, you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).
Security vulnerabilities must follow [SECURITY.md](SECURITY.md), not the public
bug-report workflow.

## Before you start

- Search existing issues and pull requests before opening a duplicate.
- Use an issue to discuss substantial behavior, protocol, architecture, or UX
  changes before investing in an implementation.
- Keep changes focused. Unrelated cleanup makes review and rollback harder.
- Protocol changes belong in the shared `octos-core` contract first; octoscode
  must not invent client-only wire fields. See
  [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

Small typo, test, and narrowly scoped bug fixes can go directly to a pull
request.

## Development setup

Install the stable Rust toolchain supported by `Cargo.toml` (Rust 1.85 or
newer). The full Unix validation suite also requires Bash, Python, and the
pinned `agent-spec` CLI:

```bash
cargo install agent-spec --version 1.4.0 --locked
```

On macOS, install GNU `stat` and `flock` as well:

```bash
brew install coreutils flock
```

Then fork the repository, clone it, and verify your fork:

```bash
git clone https://github.com/YOUR-USER/octoscode.git
cd octoscode
git remote add upstream https://github.com/octos-org/octoscode.git
scripts/verify.sh -- cargo test --all-targets
```

The wrapper configures the subprocess environment, including locating
Homebrew's GNU coreutils on macOS, but does not install these prerequisites.
The normal test suite is mock-backed and does not need an Octos server. A few
operational OLP tests execute Unix-only tooling; those targets are intentionally
not run on native Windows. Portable Rust tests must continue to compile and
pass on Windows.

## Making a change

1. Create a topic branch from the latest `main`.
2. Add or update tests that demonstrate the intended behavior.
3. When behavior changes, update the corresponding contract under `specs/` and
   bind each scenario to the real test name.
4. Update user-facing documentation and localized text when applicable.
5. Run the required checks:

```bash
scripts/verify.sh -- cargo fmt --all --check
scripts/verify.sh -- cargo clippy --all-targets -- -D warnings
scripts/verify.sh -- cargo test --all-targets
scripts/verify.sh -- cargo test --doc
```

Do not commit generated build output, credentials, provider keys, access
tokens, private prompts, or user session data.

## Pull requests

A reviewable pull request should:

- explain the problem and the chosen solution;
- link the relevant issue when one exists;
- call out user-visible, protocol, security, compatibility, or migration impact;
- include tests and the commands used to validate the change;
- include screenshots or terminal captures for visible TUI changes; and
- contain only commits needed for the stated change.

CI must pass before merge. Maintainers may request design changes, additional
tests, documentation, or a smaller scope. Approval does not guarantee immediate
merge when a change conflicts with the roadmap or a shared protocol contract.

Contributors retain copyright in their contributions. Unless explicitly stated
otherwise, contributions intentionally submitted for inclusion are licensed
under the repository's [Apache License 2.0](LICENSE), as described by section 5
of that license.

## Review and decision process

Maintainers evaluate correctness, safety, compatibility, maintainability, and
fit with the project direction. Significant or disputed decisions follow
[GOVERNANCE.md](GOVERNANCE.md). Be patient and assume good intent; reviews are
technical collaboration, not a judgment of the contributor.
