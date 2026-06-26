from __future__ import annotations

import os
from pathlib import Path
import subprocess
import sys
from typing import Callable


def spark_main() -> int:
    return _dispatch("spark", _python_spark_main)


def spark_server_main() -> int:
    return _dispatch("spark-server", _python_spark_server_main)


def _dispatch(command_name: str, fallback: Callable[[], int]) -> int:
    binary_path = _packaged_binary_path(command_name)
    if binary_path is None:
        return fallback()
    argv = [str(binary_path), *sys.argv[1:]]
    if os.name == "posix":
        os.execv(str(binary_path), argv)
    completed = subprocess.run(argv, check=False)
    return int(completed.returncode)


def _packaged_binary_path(command_name: str) -> Path | None:
    suffix = ".exe" if os.name == "nt" else ""
    candidate = Path(__file__).resolve().parent / "bin" / f"{command_name}{suffix}"
    if candidate.is_file():
        return candidate
    return None


def _python_spark_main() -> int:
    from spark.cli import main

    return int(main())


def _python_spark_server_main() -> int:
    from spark.server_cli import main

    return int(main())
