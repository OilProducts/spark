from __future__ import annotations

from dataclasses import dataclass
import os
from pathlib import Path
import tomllib
from typing import Any, Mapping

from unified_llm.adapters import OpenAICompatibleAdapter


PROFILE_CONFIG_FILE = "llm-profiles.toml"
SUPPORTED_PROFILE_PROVIDERS = {"openai_compatible"}


class LlmProfileConfigurationError(ValueError):
    pass


@dataclass(frozen=True)
class LlmProfile:
    id: str
    provider: str
    base_url: str
    models: tuple[str, ...]
    label: str | None = None
    api_key_env: str | None = None
    default_model: str | None = None

    @property
    def configured(self) -> bool:
        if self.api_key_env is None:
            return True
        return bool(os.environ.get(self.api_key_env, "").strip())

    def to_public_dict(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "label": self.label,
            "provider": self.provider,
            "models": list(self.models),
            "default_model": self.default_model,
            "configured": self.configured,
        }

    def api_key(self) -> str | None:
        if self.api_key_env is None:
            return None
        return os.environ.get(self.api_key_env)

    def build_adapter(self) -> OpenAICompatibleAdapter:
        if self.api_key_env is not None and not self.configured:
            raise LlmProfileConfigurationError(
                f"LLM profile {self.id!r} is not configured: missing environment variable {self.api_key_env}."
            )
        return OpenAICompatibleAdapter(
            api_key=self.api_key(),
            base_url=self.base_url,
            require_api_key=False,
        )


def llm_profiles_path(config_dir: Path | str) -> Path:
    return Path(config_dir).expanduser().resolve(strict=False) / PROFILE_CONFIG_FILE


def load_llm_profiles(config_dir: Path | str) -> dict[str, LlmProfile]:
    path = llm_profiles_path(config_dir)
    if not path.exists():
        return {}
    try:
        raw = tomllib.loads(path.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as exc:
        raise LlmProfileConfigurationError(f"Invalid LLM profile config: {exc}") from exc
    except OSError as exc:
        raise LlmProfileConfigurationError(f"Unable to read LLM profile config: {path}") from exc

    profiles_raw = raw.get("profiles", {})
    if not isinstance(profiles_raw, Mapping):
        raise LlmProfileConfigurationError("LLM profile config must contain a [profiles] table.")

    profiles: dict[str, LlmProfile] = {}
    for profile_id, profile_raw in profiles_raw.items():
        normalized_id = _require_non_empty_text(profile_id, f"profile id {profile_id!r}")
        if not isinstance(profile_raw, Mapping):
            raise LlmProfileConfigurationError(f"LLM profile {normalized_id!r} must be a table.")
        profiles[normalized_id] = _parse_profile(normalized_id, profile_raw)
    return profiles


def get_llm_profile(config_dir: Path | str, profile_id: str) -> LlmProfile:
    normalized_id = str(profile_id or "").strip()
    if not normalized_id:
        raise LlmProfileConfigurationError("LLM profile id is required.")
    profile = load_llm_profiles(config_dir).get(normalized_id)
    if profile is None:
        raise LlmProfileConfigurationError(f"LLM profile {normalized_id!r} was not found.")
    return profile


def public_llm_profiles(config_dir: Path | str) -> list[dict[str, Any]]:
    return [profile.to_public_dict() for profile in load_llm_profiles(config_dir).values()]


def _parse_profile(profile_id: str, raw: Mapping[str, Any]) -> LlmProfile:
    provider = _require_non_empty_text(raw.get("provider"), f"LLM profile {profile_id!r} provider").lower()
    if provider not in SUPPORTED_PROFILE_PROVIDERS:
        raise LlmProfileConfigurationError(
            f"LLM profile {profile_id!r} has unsupported provider {provider!r}; supported providers: openai_compatible."
        )
    base_url = _require_non_empty_text(raw.get("base_url"), f"LLM profile {profile_id!r} base_url")
    models_raw = raw.get("models")
    if not isinstance(models_raw, list):
        raise LlmProfileConfigurationError(f"LLM profile {profile_id!r} models must be a non-empty list.")
    models = tuple(_require_non_empty_text(model, f"LLM profile {profile_id!r} model") for model in models_raw)
    if not models:
        raise LlmProfileConfigurationError(f"LLM profile {profile_id!r} must declare at least one model.")
    default_model = _optional_text(raw.get("default_model"))
    if default_model is not None and default_model not in models:
        raise LlmProfileConfigurationError(
            f"LLM profile {profile_id!r} default_model {default_model!r} is not listed in models."
        )
    return LlmProfile(
        id=profile_id,
        label=_optional_text(raw.get("label")),
        provider=provider,
        base_url=base_url,
        api_key_env=_optional_text(raw.get("api_key_env")),
        models=models,
        default_model=default_model,
    )


def _optional_text(value: Any) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise LlmProfileConfigurationError("LLM profile text values must be strings.")
    normalized = value.strip()
    return normalized or None


def _require_non_empty_text(value: Any, label: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise LlmProfileConfigurationError(f"{label} is required.")
    return value.strip()


__all__ = [
    "LlmProfile",
    "LlmProfileConfigurationError",
    "PROFILE_CONFIG_FILE",
    "get_llm_profile",
    "llm_profiles_path",
    "load_llm_profiles",
    "public_llm_profiles",
]
