# Coding Agent Rust Runtime

This runtime is now the normal Spark agent path. Spark chat, agent-turn, codergen, tool execution, events, prompt context, steering, and provider selection are owned by Rust crates.

Primary code:

- [crates/spark-agent-adapter](../../crates/spark-agent-adapter)
- [crates/unified-llm-adapter](../../crates/unified-llm-adapter)
- [crates/attractor-runtime](../../crates/attractor-runtime)
- [crates/spark-http](../../crates/spark-http)

Validation is the Cargo/frontend gate in [AGENTS.md](../../AGENTS.md).
