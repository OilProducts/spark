## Test Buckets

- `tests/api`, `tests/engine`, `tests/handlers`, `tests/interviewer`, `tests/transforms`, and behavior-oriented integration tests are the core `spec_behavior` suite. These tests should prefer observable runtime behavior and should be traceable to normative spec statements.
- `tests/repo_hygiene` contains repository-governance checks such as CI wiring, formatting guards, and fixture hygiene. These are useful, but they are not product-behavior evidence.
- `tests/docs_traceability` contains documentation and traceability checks. These verify linkage and governance rules, not runtime semantics.

When adding to the `spec_behavior` suite, prefer tests that assert externally visible behavior over internal delegation, type-hint structure, markdown layout, or exact prose.
