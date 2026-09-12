#!/usr/bin/env python3
"""Reproduce settings defects using a disposable home/server; no model calls.

Usage: python3 api_probe.py /absolute/path/to/target/debug/spark-server
The script only changes its own temporary Spark home and terminates its server.
"""
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request


def main():
    binary = str(Path(sys.argv[1]).resolve())
    with tempfile.TemporaryDirectory(prefix="spark-settings-audit-") as directory:
        root = Path(directory)
        home = root / "home"
        (home / "config").mkdir(parents=True)
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        base = f"http://127.0.0.1:{port}/workspace/api"
        environment = os.environ.copy()
        environment["SPARK_HOME"] = str(home)
        with (root / "server.log").open("w") as log:
            server = subprocess.Popen(
                [binary, "serve", "--port", str(port)], cwd=root,
                env=environment, stdout=log, stderr=subprocess.STDOUT,
            )
            try:
                def api(method, path, body=None):
                    data = None if body is None else json.dumps(body).encode()
                    request = urllib.request.Request(
                        base + path, data=data, method=method,
                        headers={"Content-Type": "application/json"},
                    )
                    try:
                        response = urllib.request.urlopen(request, timeout=5)
                    except urllib.error.HTTPError as error:
                        response = error
                    with response:
                        return response.status, json.load(response)

                for _ in range(100):
                    if server.poll() is not None:
                        raise RuntimeError("Disposable audit server exited during startup")
                    try:
                        status, view = api("GET", "/settings")
                        if status == 200:
                            break
                    except (urllib.error.URLError, OSError):
                        pass
                    time.sleep(0.1)
                else:
                    raise RuntimeError("Disposable audit server did not become ready")

                def save(section, value, revision=None):
                    if revision is None:
                        code, current = api("GET", "/settings")
                        assert code == 200, current
                        revision = current[section]["revision"]
                    return api("PATCH", "/settings", {
                        "section": section, "value": value,
                        "expected_revision": revision,
                    })

                profile = {
                    "id": "audit-local", "provider": "openai_compatible",
                    "base_url": "http://127.0.0.1:1",
                    "models": ["audit-model"], "default_model": "audit-model",
                }
                assert save("llm_profiles", [profile])[0] == 200
                assert save("models", {
                    "llm_profile": "audit-local", "model": "audit-model",
                })[0] == 200
                path = "/conversations/audit-inherited-profile/settings"
                code, conversation = api("PUT", path, {
                    "project_path": str(root), "expected_revision": "0",
                    "model_settings": None,
                })
                assert code == 200, conversation
                before = conversation["settings"]["models"]
                code, conversation = api("PUT", path, {
                    "project_path": str(root), "expected_revision": before["revision"],
                    "provider": "openai_compatible", "model": "audit-model",
                    "reasoning_effort": "high",
                })
                assert code == 200, conversation
                after = conversation["settings"]["models"]
                print(json.dumps({"finding": "F1", "before": before, "after": after}, indent=2))

                _, current = api("GET", "/settings")
                deletion = {"section": "llm_profiles", "value": [],
                            "expected_revision": current["llm_profiles"]["revision"]}
                print(json.dumps({"finding": "F6", "validate": api("POST", "/settings/validate", deletion),
                                  "save": api("PATCH", "/settings", deletion)}, indent=2))

                changed = {**profile, "models": ["replacement-model"],
                           "default_model": "replacement-model"}
                changed_code, changed_view = save("llm_profiles", [changed])
                broken_read = api("GET", "/settings")
                print(json.dumps({"finding": "F3", "profile_save_status": changed_code,
                                  "next_settings_read": broken_read}, indent=2))
                assert changed_code == 200, changed_view
                assert save("llm_profiles", [profile], changed_view["llm_profiles"]["revision"])[0] == 200

                code, saved = save("connections", {"client_api_base_url": "http://127.0.0.1:4987"})
                assert code == 200, saved
                _, current = api("GET", "/settings")
                print(json.dumps({"finding": "F7", "connections": current["connections"],
                                  "source_fields": {key: current[key].get("source")
                                      for key in ["runtime", "connections", "providers", "agents"]}}, indent=2))
            finally:
                server.terminate()
                try:
                    server.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    server.kill()
                    server.wait()


if __name__ == "__main__":
    main()
