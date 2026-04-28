from __future__ import annotations

from fastapi.testclient import TestClient

import spark.app as product_app


def test_llm_profiles_endpoint_returns_safe_metadata(product_api_client: TestClient, monkeypatch) -> None:
    settings = product_app.get_settings()
    (settings.config_dir / "llm-profiles.toml").write_text(
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
    monkeypatch.setenv("LMSTUDIO_API_KEY", "secret-value")

    response = product_api_client.get("/attractor/api/llm-profiles")

    assert response.status_code == 200, response.text
    assert response.json() == {
        "profiles": [
            {
                "id": "lan-lmstudio",
                "label": "LAN LM Studio",
                "provider": "openai_compatible",
                "models": ["qwen2.5-coder-32b-instruct"],
                "default_model": "qwen2.5-coder-32b-instruct",
                "configured": True,
            }
        ]
    }
    assert "192.168.1.50" not in response.text
    assert "secret-value" not in response.text


def test_pipeline_preflight_validates_effective_llm_profile(attractor_api_client: TestClient) -> None:
    settings = product_app.get_settings()
    (settings.config_dir / "llm-profiles.toml").write_text(
        """
        [profiles.local]
        provider = "openai_compatible"
        base_url = "http://127.0.0.1:1234/v1"
        models = ["local-model"]
        """,
        encoding="utf-8",
    )

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": """
            digraph ProfileFlow {
                graph [ui_default_llm_profile=local, model_stylesheet="* { llm_model: local-model; }"]
                start [shape=Mdiamond]
                task [shape=box, prompt="Use local model"]
                done [shape=Msquare]
                start -> task -> done
            }
            """,
            "working_directory": str(settings.data_dir),
        },
    )

    assert response.status_code == 200, response.text
    payload = response.json()
    assert payload["status"] == "started"
    assert payload["llm_profile"] == "local"
    assert payload["llm_provider"] == "openai_compatible"
    record = product_app.attractor_server._read_run_meta(product_app.attractor_server._run_meta_path(str(payload["run_id"])))
    assert record is not None
    assert record.llm_provider == "openai_compatible"
    assert record.llm_profile == "local"


def test_pipeline_launch_normalizes_profile_id_supplied_as_provider(
    attractor_api_client: TestClient,
) -> None:
    settings = product_app.get_settings()
    (settings.config_dir / "llm-profiles.toml").write_text(
        """
        [profiles.local]
        provider = "openai_compatible"
        base_url = "http://127.0.0.1:1234/v1"
        models = ["local-model"]
        """,
        encoding="utf-8",
    )

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": """
            digraph ProfileFlow {
                start [shape=Mdiamond]
                task [shape=box, prompt="Use local model"]
                done [shape=Msquare]
                start -> task -> done
            }
            """,
            "working_directory": str(settings.data_dir),
            "model": "local-model",
            "llm_provider": "local",
            "llm_profile": "local",
        },
    )

    assert response.status_code == 200, response.text
    payload = response.json()
    assert payload["status"] == "started"
    assert payload["llm_provider"] == "openai_compatible"
    assert payload["llm_profile"] == "local"
    record = product_app.attractor_server._read_run_meta(product_app.attractor_server._run_meta_path(str(payload["run_id"])))
    assert record is not None
    assert record.llm_provider == "openai_compatible"
    assert record.llm_profile == "local"


def test_pipeline_preflight_uses_node_and_stylesheet_profile_before_launch_override(
    attractor_api_client: TestClient,
) -> None:
    settings = product_app.get_settings()
    (settings.config_dir / "llm-profiles.toml").write_text(
        """
        [profiles.launch]
        provider = "openai_compatible"
        base_url = "http://127.0.0.1:1111/v1"
        models = ["launch-model"]

        [profiles.style]
        provider = "openai_compatible"
        base_url = "http://127.0.0.1:2222/v1"
        models = ["style-model"]

        [profiles.node]
        provider = "openai_compatible"
        base_url = "http://127.0.0.1:3333/v1"
        models = ["node-model"]
        """,
        encoding="utf-8",
    )

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": """
            digraph ProfileFlow {
                graph [model_stylesheet="* { llm_profile: style; llm_model: style-model; }"]
                start [shape=Mdiamond]
                styled [shape=box, prompt="Use style profile"]
                explicit [shape=box, prompt="Use node profile", llm_profile=node, llm_model=node-model]
                done [shape=Msquare]
                start -> styled -> explicit -> done
            }
            """,
            "working_directory": str(settings.data_dir),
            "llm_profile": "launch",
            "model": "launch-model",
        },
    )

    assert response.status_code == 200, response.text
    payload = response.json()
    assert payload["status"] == "started"
    assert payload["llm_profile"] == "launch"


def test_pipeline_preflight_rejects_missing_profile(attractor_api_client: TestClient) -> None:
    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": """
            digraph ProfileFlow {
                graph [ui_default_llm_profile=missing, model_stylesheet="* { llm_model: local-model; }"]
                start [shape=Mdiamond]
                task [shape=box, prompt="Use local model"]
                done [shape=Msquare]
                start -> task -> done
            }
            """,
            "working_directory": str(product_app.get_settings().data_dir),
        },
    )

    assert response.status_code == 200, response.text
    payload = response.json()
    assert payload["status"] == "validation_error"
    assert "LLM profile 'missing' was not found" in payload["error"]
