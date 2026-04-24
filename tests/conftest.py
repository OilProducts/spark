from __future__ import annotations

from pathlib import Path
import sys

import pytest
from _pytest.mark.expression import Expression


REPO_ROOT = Path(__file__).resolve().parents[1]
SRC_ROOT = REPO_ROOT / "src"

if str(SRC_ROOT) not in sys.path:
    sys.path.insert(0, str(SRC_ROOT))
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))


def _expression_selects_live(expression: str | None) -> bool:
    if not expression:
        return False

    try:
        compiled = Expression.compile(expression)
    except SyntaxError:
        return False

    return compiled.evaluate(lambda name, **kwargs: name in {"live", "live_smoke"})


def pytest_addoption(parser: pytest.Parser) -> None:
    parser.addoption(
        "--run-live",
        action="store_true",
        default=False,
        help="run tests marked live or live_smoke",
    )


def pytest_collection_modifyitems(config: pytest.Config, items: list[pytest.Item]) -> None:
    if config.getoption("--run-live"):
        return

    markexpr = getattr(config.option, "markexpr", None)
    keywordexpr = getattr(config.option, "keyword", None)
    if _expression_selects_live(markexpr) or _expression_selects_live(keywordexpr):
        return

    skip_live = pytest.mark.skip(
        reason="live smoke tests are skipped by default; use --run-live or -m live",
    )
    for item in items:
        if (
            item.get_closest_marker("live") is not None
            or item.get_closest_marker("live_smoke") is not None
        ):
            item.add_marker(skip_live)
