from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import sys
import textwrap


def test_chat_and_codergen_facades_do_not_import_python_agent_runtime() -> None:
    repo_root = Path(__file__).resolve().parents[3]
    env = os.environ.copy()
    src_path = str(repo_root / "src")
    env["PYTHONPATH"] = src_path + (os.pathsep + env["PYTHONPATH"] if env.get("PYTHONPATH") else "")

    completed = subprocess.run(
        [
            sys.executable,
            "-c",
            textwrap.dedent(
                """
                import importlib.abc
                import json
                import sys
                import tempfile
                from pathlib import Path

                BLOCKED_PREFIXES = ("agent", "src.agent", "unified_llm", "src.unified_llm")


                class BlockedImportFinder(importlib.abc.MetaPathFinder):
                    def find_spec(self, fullname, path=None, target=None):
                        if any(fullname == prefix or fullname.startswith(prefix + ".") for prefix in BLOCKED_PREFIXES):
                            raise AssertionError(f"unexpected Python agent/provider import: {fullname}")
                        return None


                sys.meta_path.insert(0, BlockedImportFinder())

                from attractor.api.codex_backends import build_codergen_backend
                from attractor.engine.context import Context
                from spark.chat.session import CodexAppServerChatSession, UnifiedAgentChatSession


                class Boundary:
                    def __init__(self):
                        self.agent_requests = []
                        self.codergen_requests = []

                    def run_agent_turn(self, payload):
                        self.agent_requests.append(dict(payload))
                        return {"final_assistant_text": f"chat:{payload['provider']}"}

                    def run_codergen(self, payload):
                        self.codergen_requests.append(dict(payload))
                        return {"response": {"kind": "text", "value": f"codergen:{payload['provider']}"}}

                    def steer_codergen_turn(self, payload):
                        return {"status": "rejected", "reason": "not_active"}


                with tempfile.TemporaryDirectory() as tmp:
                    tmp_path = Path(tmp)
                    boundary = Boundary()
                    chat_providers = (
                        "codex",
                        "openrouter",
                        "litellm",
                        "openai-compatible",
                        "openai",
                        "anthropic",
                        "gemini",
                    )
                    for provider in chat_providers:
                        session = UnifiedAgentChatSession(
                            str(tmp_path),
                            provider=provider,
                            model="model-x",
                            llm_profile="team-profile" if provider == "codex" else None,
                            boundary=boundary,
                        )
                        result = session.turn(f"hello {provider}", None)
                        assert result.assistant_message.startswith("chat:")

                    codex_session = CodexAppServerChatSession(str(tmp_path))
                    codex_session.close()
                    build_codergen_backend(
                        "codex-app-server",
                        str(tmp_path),
                        lambda event: None,
                        model="codex-model",
                    )

                    router = build_codergen_backend(
                        "provider-router",
                        str(tmp_path),
                        lambda event: None,
                        model="fallback-model",
                        boundary=boundary,
                    )
                    codergen_providers = (
                        "openrouter",
                        "litellm",
                        "openai-compatible",
                        "openai",
                        "anthropic",
                        "gemini",
                    )
                    for provider in codergen_providers:
                        result = router.run("plan", "Prompt", Context(), provider=provider, model="model-x")
                        assert result.startswith("codergen:")
                    profiled_result = router.run(
                        "plan",
                        "Prompt",
                        Context(),
                        provider="codex",
                        model="profile-model",
                        llm_profile="team-profile",
                    )
                    assert profiled_result.startswith("codergen:")

                loaded = sorted(
                    name
                    for name in sys.modules
                    if any(name == prefix or name.startswith(prefix + ".") for prefix in BLOCKED_PREFIXES)
                )
                if loaded:
                    raise AssertionError(f"blocked modules were loaded: {loaded}")

                print(json.dumps({
                    "agent_providers": [request["provider"] for request in boundary.agent_requests],
                    "codergen_profiles": [
                        request.get("llm_profile")
                        for request in boundary.codergen_requests
                        if request["provider"] == "codex"
                    ],
                    "codergen_providers": [request["provider"] for request in boundary.codergen_requests],
                    "codergen_models": [request.get("model") for request in boundary.codergen_requests],
                }, sort_keys=True))
                """
            ),
        ],
        cwd=repo_root,
        env=env,
        capture_output=True,
        text=True,
    )

    assert completed.returncode == 0, completed.stderr
    assert completed.stderr == ""
    observed = json.loads(completed.stdout)
    assert observed == {
        "agent_providers": [
            "codex",
            "openrouter",
            "litellm",
            "openai_compatible",
            "openai",
            "anthropic",
            "gemini",
        ],
        "codergen_profiles": ["team-profile"],
        "codergen_providers": [
            "openrouter",
            "litellm",
            "openai_compatible",
            "openai",
            "anthropic",
            "gemini",
            "codex",
        ],
        "codergen_models": [
            "model-x",
            "model-x",
            "model-x",
            "model-x",
            "model-x",
            "model-x",
            "profile-model",
        ],
    }
