from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_SKIP_REASON = (
    "live smoke tests are skipped by default; use --run-live or -m live"
)
MISSING_OPENAI_CREDENTIAL_REASON = (
    "openai live smoke tests require OPENAI_API_KEY"
)
OPENAI_LIVE_NODEID = (
    "tests/adapters/test_cross_provider_parity.py::"
    "test_live_native_generation_and_streaming[openai]"
)


def _child_pytest_env() -> dict[str, str]:
    env = os.environ.copy()
    for name in ("PYTEST_ADDOPTS", "PYTEST_CURRENT_TEST"):
        env.pop(name, None)
    return env


def _run_pytest(
    *args: str,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    uv = shutil.which("uv")
    assert uv is not None

    return subprocess.run(
        [uv, "run", "pytest", *args],
        capture_output=True,
        check=False,
        cwd=REPO_ROOT,
        env=env or _child_pytest_env(),
        text=True,
    )


def test_adapter_live_smoke_default_selection_is_opt_in() -> None:
    result = _run_pytest("-q", "-rs", OPENAI_LIVE_NODEID)
    output = result.stdout + result.stderr

    assert result.returncode == 0, output
    assert DEFAULT_SKIP_REASON in output
    assert "tests/adapters/test_cross_provider_parity.py" in output


def test_adapter_live_smoke_explicit_selection_requires_credentials() -> None:
    env = _child_pytest_env()
    for name in (
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY",
    ):
        env.pop(name, None)

    result = _run_pytest("-q", "-rs", "-m", "live", OPENAI_LIVE_NODEID, env=env)
    output = result.stdout + result.stderr

    assert result.returncode == 0, output
    assert MISSING_OPENAI_CREDENTIAL_REASON in output
    assert "tests/adapters/test_cross_provider_parity.py" in output
