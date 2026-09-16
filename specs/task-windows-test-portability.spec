spec: task
name: "Windows path and test portability"
inherits: project
tags: [windows, tests, onboarding, workspace]
depends: []
estimate: 0.5d
---

## Intent

The Windows build must preserve native paths while inspecting a configured
stdio launch command, reject drive roots as overly broad workspaces, and compile
the full test target tree without importing Unix-only APIs.

## Acceptance Criteria

场景: Windows data-dir paths retain their separators during profile discovery
  测试: should_discover_end_to_end_via_data_dir_flag
  假设 A stdio launch command contains a native absolute data-dir path
  当 The client discovers profiles from that launch command
  那么 The native path resolves to the expected profile directory

场景: Filesystem roots are rejected on every supported platform
  测试: validate_workspace_rejects_filesystem_root
  假设 The staged workspace resolves to the current filesystem root
  当 The client validates the onboarding workspace
  那么 Validation rejects the root instead of accepting a whole drive or filesystem

场景: Canonical workspace assertions use host path semantics
  测试: finish_promotes_canonical_workspace_path_not_raw_candidate
  假设 A relative workspace is canonicalized on the current host
  当 The onboarding flow promotes the validated workspace
  那么 The promoted path is absolute according to the host path implementation
