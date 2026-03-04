from __future__ import annotations

import json
import re
import subprocess
import tempfile
from functools import lru_cache
from pathlib import Path


FRONTEND_CONTRACT_TEST_FILE = "src/components/__tests__/ContractBehavior.test.tsx"
CONTRACT_ID_PATTERN = re.compile(r"\[CID:([A-Za-z0-9_.-]+)\]")


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

    contract_statuses: dict[str, str] = {}
    missing_contract_ids: list[str] = []
    duplicate_contract_ids: dict[str, int] = {}
    for suite in report.get("testResults", []):
        for assertion in suite.get("assertionResults", []):
            title = str(assertion.get("title", ""))
            if not title:
                continue
            match = CONTRACT_ID_PATTERN.search(title)
            if match is None:
                missing_contract_ids.append(title)
                continue
            contract_id = match.group(1)
            status = str(assertion.get("status", "unknown"))
            if contract_id in contract_statuses:
                duplicate_contract_ids[contract_id] = duplicate_contract_ids.get(contract_id, 1) + 1
                continue
            contract_statuses[contract_id] = status

    if missing_contract_ids:
        missing = ", ".join(sorted(missing_contract_ids))
        raise AssertionError(
            "Frontend behavior contract tests must include [CID:<id>] in each test title.\n"
            f"Missing IDs for titles: {missing}"
        )
    if duplicate_contract_ids:
        duplicate_summary = ", ".join(
            f"{contract_id} ({count} titles)"
            for contract_id, count in sorted(duplicate_contract_ids.items())
        )
        raise AssertionError(
            "Duplicate frontend behavior contract IDs detected in Vitest report.\n"
            f"Duplicates: {duplicate_summary}"
        )
    if not contract_statuses:
        raise AssertionError(
            "Frontend behavior contract test report contained no assertion results.\n"
            f"report:\n{json.dumps(report, indent=2)}"
        )
    return contract_statuses


def assert_frontend_behavior_contract_passed(contract_id: str) -> None:
    statuses = _run_frontend_contract_behavior_tests()
    status = statuses.get(contract_id)
    if status != "passed":
        available = ", ".join(sorted(statuses))
        raise AssertionError(
            f"Expected frontend behavior contract to pass: {contract_id!r}; got {status!r}. "
            f"Available contract IDs: {available}"
        )
