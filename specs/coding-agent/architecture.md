# Coding Agent Architecture

The coding-agent runtime is Rust-owned after the Cargo cutover. Normal Spark chat, agent-turn, and codergen-adjacent execution goes through [crates/spark-agent-adapter](../../crates/spark-agent-adapter) and [crates/unified-llm-adapter](../../crates/unified-llm-adapter).

There is no supported Python package implementation for this runtime. The repository validation gate is the Cargo/frontend gate documented in [AGENTS.md](../../AGENTS.md).
