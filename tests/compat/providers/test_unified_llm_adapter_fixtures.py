from __future__ import annotations

from dataclasses import asdict, is_dataclass
from pathlib import Path
from typing import Any, Mapping

import unified_llm
from unified_llm.retry import calculate_retry_delay
from spark.llm_profiles import public_llm_profiles
from tests.compat import harness


ITEM_ID = "M3-I05-CODERGEN-AGENT-LLM-ADAPTERS"
ITEM_REQUIREMENTS = ("RR-EXE-006",)
ITEM_DECISIONS = ("CD-RR-006", "CD-RR-013")
ITEM_ID_M6_I09 = "M6-I09-PROVIDER-MODEL-RESOURCE-PARITY"
ITEM_REQUIREMENTS_M6_I09 = ("RR-PKG-005", "RR-PKG-002")
ITEM_DECISIONS_M6_I09 = ("CD-RR-006", "CD-RR-012", "CD-RR-013")
PROVIDER_ENV_KEYS = (
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "OPENAI_ORG_ID",
    "OPENAI_PROJECT_ID",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_BASE_URL",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GEMINI_BASE_URL",
    "OPENROUTER_API_KEY",
    "OPENROUTER_BASE_URL",
    "OPENROUTER_HTTP_REFERER",
    "OPENROUTER_TITLE",
    "LITELLM_BASE_URL",
    "LITELLM_API_KEY",
    "OPENAI_COMPATIBLE_BASE_URL",
    "OPENAI_COMPATIBLE_API_KEY",
)


def test_unified_llm_adapter_fixtures_match_python_oracle(
    compat_fixture_root: Path,
    compat_update_goldens: bool,
    monkeypatch,
) -> None:
    manifests = [
        _catalog_env_manifest(monkeypatch),
        _retry_error_usage_stream_manifest(),
        _request_tool_structured_output_manifest(),
    ]

    for manifest in manifests:
        _assert_fixture(
            manifest,
            compat_fixture_root / f"{manifest['fixture_id']}.json",
            compat_update_goldens,
        )


def test_m6_provider_profile_resource_fixture_matches_python_oracle(
    tmp_path: Path,
    compat_fixture_root: Path,
    compat_update_goldens: bool,
    monkeypatch,
) -> None:
    manifest = _m6_provider_profile_resource_manifest(tmp_path, monkeypatch)

    _assert_fixture(
        manifest,
        compat_fixture_root / "providers/m6-provider-profile-resource-parity.json",
        compat_update_goldens,
    )


def _catalog_env_manifest(monkeypatch) -> dict[str, Any]:
    _clear_provider_env(monkeypatch)
    monkeypatch.setenv("ANTHROPIC_API_KEY", "anthropic-key")
    monkeypatch.setenv("OPENAI_API_KEY", "openai-key")
    monkeypatch.setenv("OPENAI_ORG_ID", "org-1")
    client = unified_llm.Client.from_env()

    return _manifest(
        fixture_id="providers/model-catalog-env-resolution",
        scenario="model_catalog_env_resolution",
        input_payload={"env_keys": ["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "OPENAI_ORG_ID"]},
        observation={
            "sonnet_alias": _model_payload(unified_llm.get_model_info("sonnet")),
            "gemini_models": [model.id for model in unified_llm.list_models("gemini")],
            "openai_latest": _model_payload(unified_llm.get_latest_model("openai")),
            "native_capability_latest": {
                "openai_tools": _latest_model_id("openai", "tools"),
                "anthropic_reasoning": _latest_model_id("anthropic", "reasoning"),
                "gemini_vision": _latest_model_id("gemini", "vision"),
            },
            "compatible_latest_defaults": {
                "openrouter": _model_payload(unified_llm.get_latest_model("openrouter")),
                "litellm": _model_payload(unified_llm.get_latest_model("litellm")),
                "openai_compatible": _model_payload(
                    unified_llm.get_latest_model("openai_compatible")
                ),
            },
            "client_default_provider": client.default_provider,
            "client_providers": sorted(client.providers.keys()),
        },
    )


def _m6_provider_profile_resource_manifest(
    tmp_path: Path,
    monkeypatch,
) -> dict[str, Any]:
    _clear_provider_env(monkeypatch)
    monkeypatch.setenv("OPENAI_API_KEY", "openai-key")
    monkeypatch.setenv("OPENAI_BASE_URL", "https://openai.example/responses")
    monkeypatch.setenv("OPENAI_ORG_ID", "org-1")
    monkeypatch.setenv("OPENAI_PROJECT_ID", "project-1")
    monkeypatch.setenv("ANTHROPIC_API_KEY", "anthropic-key")
    monkeypatch.setenv("ANTHROPIC_BASE_URL", "https://anthropic.example/messages")
    monkeypatch.setenv("GOOGLE_API_KEY", "google-key")
    monkeypatch.setenv("GEMINI_BASE_URL", "https://gemini.example/v1")
    monkeypatch.setenv("OPENROUTER_API_KEY", "openrouter-key")
    monkeypatch.setenv("OPENROUTER_HTTP_REFERER", "https://spark.example")
    monkeypatch.setenv("OPENROUTER_TITLE", "Spark")
    monkeypatch.setenv("LITELLM_BASE_URL", "https://litellm.example/api")
    monkeypatch.setenv("LITELLM_API_KEY", "")
    monkeypatch.setenv(
        "OPENAI_COMPATIBLE_BASE_URL",
        "https://compatible.example/custom/chat/completions",
    )
    monkeypatch.setenv("OPENAI_COMPATIBLE_API_KEY", "")

    automatic_client = unified_llm.Client.from_env()
    explicit_client = unified_llm.Client.from_env(default_provider="OpenRouter")
    config_dir = tmp_path / "spark-home" / "config"
    config_dir.mkdir(parents=True)
    config_dir.joinpath("llm-profiles.toml").write_text(
        """
[profiles.local]
provider = "openai_compatible"
base_url = "http://127.0.0.1:4000/v1"
models = ["local-small", "local-large"]
label = "Local"
api_key_env = "LOCAL_PROFILE_API_KEY"
default_model = "local-large"

[profiles.no_key]
provider = "openai_compatible"
base_url = "http://127.0.0.1:5000/v1"
models = ["no-key-model"]
""".strip(),
        encoding="utf-8",
    )
    monkeypatch.delenv("LOCAL_PROFILE_API_KEY", raising=False)
    profiles_without_env = public_llm_profiles(config_dir)
    monkeypatch.setenv("LOCAL_PROFILE_API_KEY", "profile-key")
    profiles_with_env = public_llm_profiles(config_dir)

    return _manifest(
        fixture_id="providers/m6-provider-profile-resource-parity",
        scenario="m6_provider_profile_resource_parity",
        input_payload={
            "env_keys": list(PROVIDER_ENV_KEYS),
            "profile_config": "llm-profiles.toml",
        },
        observation={
            "provider_environment": {
                "automatic_default_provider": automatic_client.default_provider,
                "explicit_default_provider": explicit_client.default_provider,
                "provider_keys": sorted(explicit_client.providers.keys()),
                "providers": {
                    name: _adapter_environment_payload(adapter)
                    for name, adapter in sorted(explicit_client.providers.items())
                },
            },
            "profiles": {
                "without_process_env": profiles_without_env,
                "with_process_env": profiles_with_env,
            },
            "model_catalog": {
                "codex_alias": _model_payload(unified_llm.get_model_info("codex"))["id"],
                "provider_counts": {
                    provider: len(unified_llm.list_models(provider))
                    for provider in ("openai", "anthropic", "gemini", "openrouter", "litellm")
                },
            },
        },
        item_id=ITEM_ID_M6_I09,
        requirements=ITEM_REQUIREMENTS_M6_I09,
        decisions=ITEM_DECISIONS_M6_I09,
        provenance_interfaces=[
            "unified_llm.Client.from_env",
            "spark.llm_profiles.public_llm_profiles",
            "unified_llm.models",
        ],
    )


def _retry_error_usage_stream_manifest() -> dict[str, Any]:
    policy = unified_llm.RetryPolicy()
    rate_limit = unified_llm.RateLimitError("slow down", provider="openai", retry_after=12.5)
    server_error = unified_llm.error_from_status_code(
        503,
        message="temporary failure",
        provider="openai",
    )
    accumulator = unified_llm.StreamAccumulator()
    events = [
        unified_llm.StreamEvent(
            type=unified_llm.StreamEventType.PROVIDER_EVENT,
            raw={"kind": "provider"},
        ),
        unified_llm.StreamEvent(type=unified_llm.StreamEventType.TEXT_DELTA, delta="hello "),
        unified_llm.StreamEvent(type=unified_llm.StreamEventType.TEXT_DELTA, delta="world"),
        unified_llm.StreamEvent(
            type=unified_llm.StreamEventType.FINISH,
            finish_reason=unified_llm.FinishReason(unified_llm.FinishReason.STOP),
            usage=unified_llm.Usage(input_tokens=10, output_tokens=5, total_tokens=15),
        ),
    ]
    accumulator.extend(events)
    model = unified_llm.get_model_info("gpt-5.2")
    usage = unified_llm.Usage(input_tokens=10, output_tokens=5, total_tokens=15)
    cost = {
        "input_cost": usage.input_tokens * model.input_cost_per_million / 1_000_000,
        "output_cost": usage.output_tokens * model.output_cost_per_million / 1_000_000,
    }
    cost["total_cost"] = cost["input_cost"] + cost["output_cost"]

    return _manifest(
        fixture_id="providers/retry-error-usage-stream",
        scenario="retry_error_usage_stream",
        input_payload={},
        observation={
            "retry_policy": _object_payload(policy),
            "retry_delays": {
                "attempt_0_no_jitter": calculate_retry_delay(
                    policy,
                    0,
                    random_source=lambda _a, _b: 1.0,
                ),
                "attempt_1_no_jitter": calculate_retry_delay(
                    policy,
                    1,
                    random_source=lambda _a, _b: 1.0,
                ),
                "retry_after": policy.calculate_delay(0, error=rate_limit),
            },
            "errors": {
                "server": _error_payload(server_error),
                "not_found_classifier": unified_llm.classify_provider_error_message(
                    "model does not exist",
                    None,
                ).__name__,
            },
            "stream": {
                "text": accumulator.text,
                "finish_reason": accumulator.finish_reason.reason,
                "usage": _object_payload(accumulator.usage),
                "raw_events": accumulator.raw_events,
                "event_types": [event.type.value for event in accumulator.events],
            },
            "usage_cost": cost,
        },
    )


def _request_tool_structured_output_manifest() -> dict[str, Any]:
    tool_call = unified_llm.ToolCall(
        id="call-1",
        name="weather",
        arguments={"city": "NYC"},
    )
    tool_result = unified_llm.ToolResult.success("call-1", {"forecast": "sunny"})
    request = unified_llm.Request(
        model="gpt-5.2",
        provider="openai",
        messages=[unified_llm.Message.user("hello")],
        response_format=unified_llm.ResponseFormat(
            type="json_schema",
            json_schema={"name": "Decision", "schema": {"type": "object"}},
            strict=True,
        ),
        reasoning_effort="medium",
        provider_options={"trace": "enabled"},
        metadata={"run_id": "run-1"},
    )
    return _manifest(
        fixture_id="providers/request-tool-structured-output",
        scenario="request_tool_structured_output",
        input_payload={},
        observation={
            "request": {
                "model": request.model,
                "provider": request.provider,
                "messages": [
                    {
                        "role": message.role.value,
                        "text": message.text,
                    }
                    for message in request.messages
                ],
                "response_format": _object_payload(request.response_format),
                "reasoning_effort": request.reasoning_effort,
                "provider_options": request.provider_options,
                "metadata": request.metadata,
            },
            "tool_call": _object_payload(tool_call),
            "tool_result": _object_payload(tool_result),
        },
    )


def _model_payload(model: Any) -> dict[str, Any] | None:
    if model is None:
        return None
    return {
        "id": model.id,
        "provider": model.provider,
        "display_name": model.display_name,
        "context_window": model.context_window,
        "supports_tools": model.supports_tools,
        "supports_vision": model.supports_vision,
        "supports_reasoning": model.supports_reasoning,
        "max_output": model.max_output,
        "input_cost_per_million": model.input_cost_per_million,
        "output_cost_per_million": model.output_cost_per_million,
        "aliases": list(model.aliases),
    }


def _latest_model_id(provider: str, capability: str) -> str | None:
    model = unified_llm.get_latest_model(provider, capability)
    return None if model is None else model.id


def _error_payload(error: Exception) -> dict[str, Any]:
    return {
        "type": error.__class__.__name__,
        "message": getattr(error, "message", str(error)),
        "provider": getattr(error, "provider", None),
        "status_code": getattr(error, "status_code", None),
        "retryable": getattr(error, "retryable", None),
        "retry_after": getattr(error, "retry_after", None),
    }


def _object_payload(value: Any) -> dict[str, Any]:
    if is_dataclass(value):
        return asdict(value)
    return dict(getattr(value, "__dict__", {}))


def _adapter_environment_payload(adapter: Any) -> dict[str, Any]:
    return {
        "config": dict(getattr(adapter, "config", {})),
        "default_headers": dict(getattr(adapter, "default_headers", {})),
        "require_api_key": getattr(adapter, "require_api_key", None),
    }


def _manifest(
    *,
    fixture_id: str,
    scenario: str,
    input_payload: Mapping[str, Any],
    observation: Mapping[str, Any],
    item_id: str = ITEM_ID,
    requirements: tuple[str, ...] = ITEM_REQUIREMENTS,
    decisions: tuple[str, ...] = ITEM_DECISIONS,
    provenance_interfaces: list[str] | None = None,
) -> dict[str, Any]:
    return {
        "schema_version": "compat-provider-v1",
        "fixture_id": fixture_id,
        "item_id": item_id,
        "requirements": list(requirements),
        "decisions": list(decisions),
        "provenance": {
            "oracle": "python-unified-llm-public-interfaces",
            "interfaces": provenance_interfaces or [
                "unified_llm.Client.from_env",
                "unified_llm.models",
                "unified_llm.retry",
                "unified_llm.errors",
                "unified_llm.StreamAccumulator",
                "unified_llm.Request",
            ],
        },
        "scenario": scenario,
        "input": dict(input_payload),
        "observation": dict(observation),
    }


def _assert_fixture(
    manifest: Mapping[str, Any],
    fixture_path: Path,
    update_goldens: bool,
) -> None:
    harness.validate_manifest_coverage(
        manifest,
        requirement_ids=tuple(str(value) for value in manifest.get("requirements", ())),
        decision_ids=tuple(str(value) for value in manifest.get("decisions", ())),
    )
    if update_goldens:
        harness.write_manifest(fixture_path, manifest)
    expected = harness.load_manifest(fixture_path)
    harness.assert_provider_manifest_matches_golden(manifest, expected)


def _clear_provider_env(monkeypatch) -> None:
    for key in PROVIDER_ENV_KEYS:
        monkeypatch.delenv(key, raising=False)
