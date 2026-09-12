# Native review turn identity implementation plan

**Goal:** prevent a missing native result write from assigning another runtime turn's model to a report; reject malformed cross state without a traceback.

**Architecture:** octos writes the existing runtime TurnId into every native result frontmatter. octoscode freezes the native result path and SHA256, resolves that immutable report's turn_id against the complete session ledger, and checks that exact turn's completion/model. Report ordinals remain display/file identifiers; they no longer select ledger turns. Legacy reports may remain audit records but cannot satisfy model acceptance without native turn_id.

**Tech stack:** Rust native writer; Python evidence verifier; existing Python and Rust integration suites. No new dependencies.

## Constraints

- Keep GLM/K3 default lane policy unchanged. This implementation is reviewed by two operator-approved native GLM peers.
- Preserve existing Herdr sessions, global credentials, and unrelated worktree changes. Use the existing isolated PR637 worktree and a clean octos worktree based on latest origin/main.
- Never derive or backfill native turn_id from report ordinal. Missing, stale, malformed, duplicate, foreign or noncompleted identity rejects model acceptance. Retained ledger-prefix loss remains unverifiable.
- Bind both initial and cross native report bytes. A changed/deleted native report invalidates its evidence; compatibility collection remains auditable with warnings.

## Tasks

- [x] RED: extend native writer roundtrip to require turn_id; add model tests for completed-write-loss / wrong model, malformed cross entries/slug types, missing identity and native-report drift. Run and retain failing output.
- [x] Native writer: pass the current TurnId through both terminal writer call sites and test call sites; emit turn_id in result.md/result-N.md, preserve existing result numbering and turns index format. Exercise actual failed version/index writes in a temporary staged peer and verify the next result retains its actual UUID.
- [x] Verifier: freeze native report `{path, sha256}` bindings; bind cross independently after native authority validation; resolve exact turn_id in full ledger; preserve cross-after-initial ordering in both native and runtime order; reject reuse of the same runtime ID.
- [x] State defense: validate cross record fields needed by status; internal predicates fail closed on malformed input, including nonhashable slugs. Keep legacy records without model evidence readable.
- [x] Tests/contracts/docs: update synthetic fixtures to explicit native IDs and add CLI/live-Cargo regression paths; update corresponding .spec scenarios and compatibility recovery documentation.
- [ ] Verify: cargo fmt, clippy all-targets -D warnings, cargo test --all-targets for octoscode; octos-cli api all-targets tests/clippy for runtime plus production binary build. Retain exact commands and failures; do not hide unrelated baseline flakes.
- [ ] Freeze both diffs and run two new native GLM reviewer peers on the updated binary. Freeze initials before cross. Verify actual model and TurnId receipts, including a final delta if fixes follow review.
- [ ] Commit only owned files; update PR637 and open a linked runtime writer PR. Track CI and current review comments; no merge.
