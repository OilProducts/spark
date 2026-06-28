from __future__ import annotations

from pathlib import Path, PurePosixPath
import subprocess


REPO_ROOT = Path(__file__).resolve().parents[2]

FORBIDDEN_DIRECTORY_COMPONENTS = {
    ".idea",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".tox",
    ".venv",
    ".vscode",
    "__pycache__",
    "build",
    "dist",
    "htmlcov",
    "node_modules",
    "target",
    "venv",
}
FORBIDDEN_PATH_PREFIXES = (
    ".spark/",
    ".planflow/",
    "artifacts/",
    "codex-app-server.schema/",
    "frontend/.tmp-",
    "frontend/src/__tests__/.tmp-compat-probes/",
    "src/spark/ui_dist/",
    "tests/compat/.server-logs/",
    "tests/compat/.tmp/",
    "tests/compat/_generated/",
)
FORBIDDEN_BASENAMES = {
    ".coverage",
    ".ds_store",
    ".env",
    ".envrc",
    ".netrc",
    "auth.json",
    "compose.override.yaml",
    "compose.override.yml",
    "final-validation-artifacts.json",
    "latest-exit-code.txt",
    "latest-stderr.txt",
    "latest-stdout.txt",
    "m0-coverage-ledger.json",
    "m0-validation-gate.json",
    "provider.env",
    "requirement-decision-coverage-review.json",
    "validation-result.json",
}
FORBIDDEN_SUFFIXES = {
    ".egg-info",
    ".key",
    ".p12",
    ".pfx",
    ".pyc",
    ".pyo",
}
SECRET_DATA_SUFFIXES = {
    ".env",
    ".ini",
    ".json",
    ".pem",
    ".toml",
    ".yaml",
    ".yml",
}
SECRET_STEM_TOKENS = {
    "apikey",
    "api_key",
    "credential",
    "credentials",
    "secret",
    "secrets",
    "token",
    "tokens",
}
SECRET_STEM_SUFFIXES = (
    "_apikey",
    "_api_key",
    "_credentials",
    "_secrets",
    "_tokens",
)


def test_git_tracked_paths_exclude_runtime_generated_build_cache_and_secret_files() -> None:
    violations = {
        path: violation
        for path in _git_ls_files()
        if (violation := _tracked_path_violation(path)) is not None
    }

    assert violations == {}


def test_git_tracked_path_hygiene_allows_legitimate_credentials_source_modules() -> None:
    assert _tracked_path_violation("crates/example/src/credentials.rs") is None
    assert _tracked_path_violation("src/spark/auth/token.py") is None

    assert _tracked_path_violation(".spark/spec-implementation/current/state.json")
    assert _tracked_path_violation("target/debug/spark-server")
    assert _tracked_path_violation("config/credentials.json")
    assert _tracked_path_violation("config/secrets.toml")
    assert _tracked_path_violation("src/package/__pycache__/module.cpython-313.pyc")


def _tracked_path_violation(path: str) -> str | None:
    posix_path = PurePosixPath(path)
    parts = posix_path.parts
    name = posix_path.name
    lower_name = name.lower()

    if path.startswith(FORBIDDEN_PATH_PREFIXES):
        return "runtime/generated repository path"

    forbidden_components = FORBIDDEN_DIRECTORY_COMPONENTS.intersection(parts)
    if forbidden_components:
        return f"build/cache/IDE directory component: {sorted(forbidden_components)}"

    if lower_name in FORBIDDEN_BASENAMES:
        return "generated validation, local config, or credential file"

    if any(lower_name.endswith(suffix) for suffix in FORBIDDEN_SUFFIXES):
        return "cache, build, or private key suffix"

    if _looks_like_secret_data_file(posix_path):
        return "credential or secret data file"

    return None


def _looks_like_secret_data_file(path: PurePosixPath) -> bool:
    name = path.name.lower()
    if name.startswith(".env."):
        return True

    suffix = path.suffix.lower()
    if suffix not in SECRET_DATA_SUFFIXES:
        return False

    normalized_stem = path.stem.lower().replace("-", "_")
    return normalized_stem in SECRET_STEM_TOKENS or normalized_stem.endswith(
        SECRET_STEM_SUFFIXES
    )


def _git_ls_files() -> list[str]:
    result = subprocess.run(
        ["git", "ls-files"],
        cwd=REPO_ROOT,
        check=True,
        text=True,
        capture_output=True,
    )
    return [line.strip() for line in result.stdout.splitlines() if line.strip()]
