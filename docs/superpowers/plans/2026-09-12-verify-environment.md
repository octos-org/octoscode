# Unified verification environment implementation plan

1. Provide scripts/verify.sh to export PYTHONDONTWRITEBYTECODE=1 to the
   complete process tree. Reject malformed CLI arguments with exit 64 before
   environment checks. Pass custom command exits through unchanged.
2. On Darwin, probe GNU stat and realpath capabilities together. Accept an
   already-working PATH; otherwise resolve Homebrew coreutils gnubin, prepend
   it for this process and re-probe. Missing capability exits 127 with guidance.
3. Route existing CI fmt, clippy, all-targets tests and doctests through this
   wrapper. Keep package installation and the CI aggregate job unchanged.
4. Test real child processes: nested Python imports, custom exits and usage,
   controlled Darwin capability/fallback/missing-tool cases, and no unnecessary
   brew lookup. Use a fixture-owned prefix including spaces. Explicit uname
   shims exercise Darwin branches on either Linux or macOS.
5. Clear ambient PYTHONDONTWRITEBYTECODE in the test launcher. A scratch copy
   with the wrapper export removed must fail the nested import test, including
   when the outer CI environment sets the variable. Setting it to "0" does not
   disable Python's suppression; it is intentionally not used as a mutation.
6. Validate source with fmt, clippy and all-targets tests; independently review
   the final files and record exact exits, test totals and source hashes.

The Linux uname branch probe on a macOS host is not an actual Ubuntu CI run.
Model review failures remain explicit: K3 returned HTTP 403 quota exhaustion;
GLM streaming retries cannot count as a completed final review.
