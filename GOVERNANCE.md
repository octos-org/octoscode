# Project Governance

octoscode uses lightweight, maintainer-led governance. The goal is to make
decisions transparent without imposing process that is heavier than the
project needs.

## Roles

**Contributors** are anyone who participates through issues, reviews,
documentation, code, design, testing, or community support.

**Maintainers** are contributors trusted by the Octos organization to triage
issues, review and merge pull requests, manage releases, handle security
reports, and represent the project's technical direction. Repository access is
granted and removed by the existing maintainers based on sustained,
constructive participation, sound judgment, and demonstrated care for users.

No role is permanent. A maintainer may step down at any time, and inactive
access may be removed to reduce operational and security risk.

## Decisions

Routine decisions are made in the issue or pull request where the work occurs.
Maintainers seek rough consensus, weighing technical evidence, user impact,
compatibility, security, maintenance cost, and alignment with Octos's shared
protocol.

For substantial or disputed changes:

1. document the problem, alternatives, and tradeoffs in a public issue or
   design document;
2. allow reasonable time for affected contributors to comment;
3. record the decision and rationale in the issue or pull request; and
4. have a maintainer who is not the sole author approve the change when another
   active maintainer is available.

If consensus cannot be reached, maintainers may defer or reject a proposal.
The Octos organization retains final responsibility for repository scope,
security, releases, and legal matters.

## Changes and releases

Changes merge through pull requests after required CI passes and requested
reviews are resolved. Direct pushes are reserved for repository recovery or an
urgent security response and should be documented afterward.

Maintainers choose release timing, versioning, and backports based on user
impact and risk. Release artifacts must be produced by the repository's
documented automation. Security releases follow [SECURITY.md](SECURITY.md) and
may be developed privately until coordinated disclosure.

## Conflicts of interest

Reviewers should disclose material conflicts and recuse themselves when their
impartiality could reasonably be questioned. Conduct reports must not be
handled solely by the person named in the report. Security reporters should be
credited according to their preference.

## Governance changes

Changes to this document use the same pull-request process as other project
policy. The rationale for a material governance change should be recorded in
the pull request.
