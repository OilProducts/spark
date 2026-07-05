# Unified LLM Architecture

Unified LLM behavior is Rust-owned after the Cargo cutover. The active implementation lives in [crates/unified-llm-adapter](../../crates/unified-llm-adapter), with model catalog data embedded by [crates/spark-assets](../../crates/spark-assets).

The supported validation gate is the Cargo/frontend gate documented in [AGENTS.md](../../AGENTS.md).
