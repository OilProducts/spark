from __future__ import annotations

import json
import subprocess
import tempfile
from functools import lru_cache
from pathlib import Path


FRONTEND_CONTRACT_TEST_FILE = "src/components/__tests__/ContractBehavior.test.tsx"


@lru_cache(maxsize=1)
def _run_frontend_contract_behavior_tests() -> dict[str, str]:
    repo_root = Path(__file__).resolve().parents[4]
    frontend_dir = repo_root / "frontend"
    report_handle = tempfile.NamedTemporaryFile(
        prefix="frontend-contract-behavior-",
        suffix=".json",
        delete=False,
    )
    report_handle.close()
    report_path = Path(report_handle.name)

    try:
        result = subprocess.run(
            [
                "npm",
                "--prefix",
                str(frontend_dir),
                "run",
                "test:unit",
                "--",
                "--run",
                FRONTEND_CONTRACT_TEST_FILE,
                "--reporter=json",
                "--outputFile",
                str(report_path),
            ],
            cwd=repo_root,
            capture_output=True,
            text=True,
            check=False,
        )

        if result.returncode != 0:
            raise AssertionError(
                "Frontend behavior contract tests failed.\n"
                f"stdout:\n{result.stdout}\n"
                f"stderr:\n{result.stderr}"
            )

        if not report_path.exists():
            raise AssertionError("Frontend behavior contract report was not generated.")

        report = json.loads(report_path.read_text(encoding="utf-8"))
    finally:
        if report_path.exists():
            report_path.unlink()

    test_statuses: dict[str, str] = {}
    for suite in report.get("testResults", []):
        for assertion in suite.get("assertionResults", []):
            title = str(assertion.get("title", ""))
            if not title:
                continue
            test_statuses[title] = str(assertion.get("status", "unknown"))

    if not test_statuses:
        raise AssertionError(
            "Frontend behavior contract test report contained no assertion results.\n"
            f"report:\n{json.dumps(report, indent=2)}"
        )
    return test_statuses


def assert_frontend_behavior_test_passed(test_title: str) -> None:
    statuses = _run_frontend_contract_behavior_tests()
    status = statuses.get(test_title)
    if status != "passed":
        available = ", ".join(sorted(statuses))
        raise AssertionError(
            f"Expected frontend behavior test to pass: {test_title!r}; got {status!r}. "
            f"Available tests: {available}"
        )
