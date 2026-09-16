# Context snapshot coherence

- Additive semantic-cache diagnostics are extracted from raw lifecycle
  notifications, `session/open` results, and `session/hydrate` results before
  the pinned protocol types discard unknown fields.
- Lifecycle state and its optional cache diagnostics are applied in one client
  event. A new snapshot without diagnostics clears the previous epoch instead
  of rendering generation N beside generation N-1 diagnostics.
- Diagnostics are interpreted only while the server advertises
  `context.semantic_cache.v1`. A capability downgrade clears every cached
  diagnostic immediately while preserving the ordinary context lifecycle.
- Backend relaunch invalidates startup-pinned model catalogs. A later
  `session/status/read` marks the reported effective model selected, or drops
  a catalog that cannot represent it so `/model` refetches server truth.
