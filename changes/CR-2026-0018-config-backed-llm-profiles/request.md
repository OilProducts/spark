# Config-Backed LLM Profiles

## Summary

Add named LLM profiles backed by `$SPARK_HOME/config/llm-profiles.toml` so users can configure LAN/self-hosted OpenAI-compatible endpoints once, then select them from Spark UI and reuse them across large workflows without repeating endpoint details. Flow files should reference profile ids and model ids, never base URLs or secrets.

## Key Changes

- Add a Spark LLM profile config reader for `$SPARK_HOME/config/llm-profiles.toml`.
  - Config shape:
    ```toml
    [profiles.lan-lmstudio]
    label = "LAN LM Studio"
    provider = "openai_compatible"
    base_url = "http://192.168.1.50:1234/v1"
    api_key_env = "LMSTUDIO_API_KEY"
    models = ["qwen2.5-coder-32b-instruct"]
    default_model = "qwen2.5-coder-32b-instruct"
    ```
  - Required: `provider`, `base_url` for `openai_compatible`, at least one `models` entry.
  - Optional: `label`, `api_key_env`, `default_model`.
  - Secrets stay in env vars; the config stores only the env var name.

- Add `openai_compatible` as a selectable Spark/Attractor provider backed by the existing `OpenAICompatibleAdapter`.
  - Require explicit model selection.
  - Allow missing API key for local endpoints.
  - Use the profile’s `base_url` and optional `api_key_env` to construct the adapter.

- Add `llm_profile` as a first-class runtime selection key.
  - Allow it as a node attribute and in `model_stylesheet`.
  - Resolution order mirrors existing model/provider behavior: explicit node attr, stylesheet/defaults, launch override, then absent.
  - If `llm_profile` is present, it resolves provider/base URL/API key from config; `llm_model` still selects the model.
  - `llm_provider` remains valid for built-in providers and direct provider selection.

- Add a backend workspace endpoint for safe UI metadata:
  - `GET /api/llm-profiles`
  - Returns profile id, label, provider, models, default model, and configured status.
  - Does not return `base_url`, API key values, or secret material.

- Update frontend provider/model selection surfaces.
  - Populate profile options from `/api/llm-profiles`.
  - Let users choose either a built-in provider or a configured profile.
  - When a profile is selected, model suggestions come from that profile’s `models`.
  - Global defaults and graph defaults should support `llm_profile` alongside `llm_provider` and `llm_model`.

- Update Attractor launch, retry, child-run, and project-chat paths to preserve `llm_profile`.
  - Run records should store the selected profile id when used.
  - Preflight should fail before execution if the profile is missing, malformed, lacks models, or references a missing `api_key_env` value when one is declared.

## Test Plan

- Backend config tests:
  - valid TOML profile loads and redacts secrets from API output
  - missing profile, missing base URL, empty models, invalid provider, and missing declared env var fail clearly
  - `openai_compatible` profile constructs an adapter with the configured base URL and optional API key

- Attractor/workspace tests:
  - flow/node `llm_profile` resolves through stylesheet/default precedence
  - launch preflight validates all effective profiles before execution
  - run records preserve `llm_profile`
  - project chat can start a session using a configured profile and explicit model

- Frontend tests:
  - settings and graph/node selectors show configured profiles
  - selecting a profile changes model suggestions to that profile’s models
  - no base URL or secret fields are rendered from profile metadata

- Final validation:
  - `uv run pytest -q`
  - `npm --prefix frontend run test:unit` if frontend files changed

## Assumptions

- Profile config is user/workspace-local under `$SPARK_HOME/config`, not stored in project DOT files.
- DOT files may reference `llm_profile`, but never endpoint URLs or secrets.
- `openai_compatible` is the generic endpoint provider for LM Studio, llama.cpp server, vLLM, LocalAI, and similar servers.
- OpenRouter and LiteLLM remain separate named providers because they have distinct configuration expectations.
