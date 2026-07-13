## Test Execution Policy
- Before reporting completion of a code change, run the full validation gate unless the user asks otherwise:
  - `cargo fmt --all -- --check`
  - `cargo test --workspace --all-features`
  - `npm --prefix frontend run test:unit`
  - `npm --prefix frontend run build`
- Rust integration tests are grouped into `contracts` and, for process-global or live-resource cases, `process_contracts`. For focused triage, filter by module and case, for example `cargo test -p spark-agent-adapter --test contracts prompt_context_contracts::case_name` or `cargo test -p spark-agent-adapter --test process_contracts llm_backend_contracts::case_name`.
- Development and test profiles use line-table-only debug information to retain useful backtraces with smaller artifacts. Use `just rust-cache-size` to inspect Cargo cache usage and invoke `just clean-rust-cache` only when you explicitly want to remove Cargo's configured target directory.
- Write tests against observable behavior through real interfaces (CLI output, API responses, UI behavior, filesystem effects, state transitions), not repository text.
- When a change replaces or removes a behavior, delete or rewrite tests for the old behavior unless backward compatibility is an explicit requirement.
- Do not add or keep tests that depend on source/prompt/doc/spec strings or deprecated details, or that would fail after harmless refactoring or rewording while behavior remains correct.

## UI Spec Guidance
- If the look is not already established in the product or request, clarify it before implementing.
- Spec answers: what should happen?
- Tests answer: does it actually do that?
