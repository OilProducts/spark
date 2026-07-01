# Unified LLM Rust Runtime

The Rust runtime is the normal provider path for Spark server and CLI execution. It owns provider configuration, model catalog resolution, request/response DTOs, streaming, retries, middleware, tool calls, and structured output behavior.

Primary code:

- [crates/unified-llm-adapter](../../crates/unified-llm-adapter)
- [crates/spark-assets/assets/unified_llm/data/models.json](../../crates/spark-assets/assets/unified_llm/data/models.json)

Validation is the Cargo/frontend gate in [AGENTS.md](../../AGENTS.md). Optional live provider smoke remains opt-in through Rust tests and provider credentials.
