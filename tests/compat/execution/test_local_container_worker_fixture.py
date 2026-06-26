from __future__ import annotations

import json
import subprocess
from pathlib import Path


def test_rust_hidden_worker_process_executes_json_line_request() -> None:
    repo_root = Path(__file__).resolve().parents[3]
    subprocess.run(
        ["cargo", "build", "-p", "spark-server", "--bin", "spark-server"],
        cwd=repo_root,
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    binary = repo_root / "target" / "debug" / "spark-server"
    request = {
        "run_id": "run-worker-compat",
        "graph": {
            "graph_id": "G",
            "graph_attrs": {},
            "nodes": {
                "start": {
                    "node_id": "start",
                    "attrs": {
                        "shape": {
                            "key": "shape",
                            "value": "Mdiamond",
                            "value_type": "string",
                            "line": 0,
                        }
                    },
                    "line": 0,
                    "explicit_attr_keys": ["shape"],
                }
            },
            "edges": [],
            "defaults": {"node": {}, "edge": {}},
            "subgraphs": [],
        },
        "node_id": "start",
        "prompt": "",
        "context": {},
        "context_logs": [],
        "logs_root": None,
        "working_dir": str(repo_root),
        "backend_name": None,
        "model": None,
        "config_dir": None,
    }

    help_result = subprocess.run(
        [str(binary), "--help"],
        cwd=repo_root,
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    assert "worker" not in help_result.stdout

    result = subprocess.run(
        [str(binary), "worker", "run-node"],
        cwd=repo_root,
        input=json.dumps(request, sort_keys=True) + "\n",
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )

    assert result.returncode == 0
    assert result.stderr == ""
    frame = json.loads(result.stdout)
    assert frame["type"] == "result"
    assert frame["outcome"]["status"] == "success"
