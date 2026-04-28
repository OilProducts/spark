from __future__ import annotations

import pytest

from spark.llm_profiles import LlmProfileConfigurationError, get_llm_profile, load_llm_profiles, public_llm_profiles


def test_llm_profile_config_loads_and_public_metadata_redacts_endpoint_and_secret(tmp_path, monkeypatch) -> None:
    config_dir = tmp_path / "config"
    config_dir.mkdir()
    (config_dir / "llm-profiles.toml").write_text(
        """
        [profiles.lan-lmstudio]
        label = "LAN LM Studio"
        provider = "openai_compatible"
        base_url = "http://192.168.1.50:1234/v1"
        api_key_env = "LMSTUDIO_API_KEY"
        models = ["qwen2.5-coder-32b-instruct"]
        default_model = "qwen2.5-coder-32b-instruct"
        """,
        encoding="utf-8",
    )
    monkeypatch.setenv("LMSTUDIO_API_KEY", "secret-key")

    profile = load_llm_profiles(config_dir)["lan-lmstudio"]

    assert profile.base_url == "http://192.168.1.50:1234/v1"
    assert profile.api_key() == "secret-key"
    assert public_llm_profiles(config_dir) == [
        {
            "id": "lan-lmstudio",
            "label": "LAN LM Studio",
            "provider": "openai_compatible",
            "models": ["qwen2.5-coder-32b-instruct"],
            "default_model": "qwen2.5-coder-32b-instruct",
            "configured": True,
        }
    ]


@pytest.mark.parametrize(
    ("body", "message"),
    [
        ('[profiles.bad]\nprovider = "anthropic"\nbase_url = "http://x"\nmodels = ["m"]\n', "unsupported provider"),
        ('[profiles.bad]\nprovider = "openai_compatible"\nmodels = ["m"]\n', "base_url"),
        ('[profiles.bad]\nprovider = "openai_compatible"\nbase_url = "http://x"\nmodels = []\n', "at least one model"),
    ],
)
def test_llm_profile_config_rejects_malformed_profiles(tmp_path, body: str, message: str) -> None:
    config_dir = tmp_path / "config"
    config_dir.mkdir()
    (config_dir / "llm-profiles.toml").write_text(body, encoding="utf-8")

    with pytest.raises(LlmProfileConfigurationError, match=message):
        load_llm_profiles(config_dir)


def test_llm_profile_adapter_uses_configured_endpoint_and_optional_key(tmp_path, monkeypatch) -> None:
    config_dir = tmp_path / "config"
    config_dir.mkdir()
    (config_dir / "llm-profiles.toml").write_text(
        """
        [profiles.local]
        provider = "openai_compatible"
        base_url = "http://127.0.0.1:1234/v1"
        api_key_env = "LOCAL_LLM_KEY"
        models = ["local-model"]
        """,
        encoding="utf-8",
    )
    monkeypatch.delenv("LOCAL_LLM_KEY", raising=False)
    profile = get_llm_profile(config_dir, "local")
    assert public_llm_profiles(config_dir)[0]["configured"] is False
    with pytest.raises(LlmProfileConfigurationError, match="LOCAL_LLM_KEY"):
        profile.build_adapter()

    monkeypatch.setenv("LOCAL_LLM_KEY", "local-secret")
    adapter = get_llm_profile(config_dir, "local").build_adapter()

    assert adapter.base_url == "http://127.0.0.1:1234/v1"
    assert adapter.api_key == "local-secret"
