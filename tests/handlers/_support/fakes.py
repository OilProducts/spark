from __future__ import annotations

from contextlib import contextmanager
import threading
import time
from pathlib import Path

from attractor.engine.context import Context
from attractor.engine.outcome import Outcome, OutcomeStatus
from attractor.interviewer import Answer, Interviewer, Question

class _StubBackend:
    def __init__(self, ok: bool = True):
        self.ok = ok
        self.calls = []

    def run(
        self,
        node_id: str,
        prompt: str,
        context: Context,
        *,
        response_contract: str = "",
        contract_repair_attempts: int = 0,
        timeout=None,
        model=None,
        write_contract=None,
    ) -> bool:
        self.calls.append(
            (node_id, prompt, dict(context.values), response_contract, contract_repair_attempts, model)
        )
        return self.ok

class _ArtifactProbeBackend:
    def __init__(self, logs_root: Path):
        self.logs_root = logs_root
        self.prompt_exists_during_call = False
        self.response_exists_during_call = False
        self.prompt_text_during_call = ""

    def run(
        self,
        node_id: str,
        prompt: str,
        context: Context,
        *,
        response_contract: str = "",
        contract_repair_attempts: int = 0,
        timeout=None,
        model=None,
        write_contract=None,
    ) -> bool:
        del prompt, context, response_contract, contract_repair_attempts, timeout, model, write_contract
        stage_dir = self.logs_root / node_id
        prompt_path = stage_dir / "prompt.md"
        response_path = stage_dir / "response.md"
        self.prompt_exists_during_call = prompt_path.exists()
        if self.prompt_exists_during_call:
            self.prompt_text_during_call = prompt_path.read_text(encoding="utf-8").strip()
        self.response_exists_during_call = response_path.exists()
        return True

class _TextBackend:
    def __init__(self, text: str):
        self.text = text

    def run(
        self,
        node_id: str,
        prompt: str,
        context: Context,
        *,
        response_contract: str = "",
        contract_repair_attempts: int = 0,
        timeout=None,
        model=None,
        write_contract=None,
    ) -> str:
        del node_id, prompt, context, response_contract, contract_repair_attempts, timeout, model, write_contract
        return self.text

class _OutcomeBackend:
    def __init__(self, outcome: Outcome):
        self.outcome = outcome

    def run(
        self,
        node_id: str,
        prompt: str,
        context: Context,
        *,
        response_contract: str = "",
        contract_repair_attempts: int = 0,
        timeout=None,
        model=None,
        write_contract=None,
    ) -> Outcome:
        del node_id, prompt, context, response_contract, contract_repair_attempts, timeout, model, write_contract
        return self.outcome

class _FanInRankingBackend:
    def __init__(self, response: str):
        self.response = response
        self.calls = []

    def run(
        self,
        node_id: str,
        prompt: str,
        context: Context,
        *,
        response_contract: str = "",
        contract_repair_attempts: int = 0,
        timeout=None,
        model=None,
        write_contract=None,
    ) -> str:
        self.calls.append(
            {
                "node_id": node_id,
                "prompt": prompt,
                "context": dict(context.values),
                "response_contract": response_contract,
                "contract_repair_attempts": contract_repair_attempts,
                "timeout": timeout,
                "model": model,
                "write_contract": write_contract,
            }
        )
        return self.response


class _StageLoggingBackend:
    def __init__(self, response: str):
        self.response = response
        self.bind_calls = []
        self.run_bound = False
        self._active_bindings = 0

    @contextmanager
    def bind_stage_raw_rpc_log(self, node_id: str, logs_root: Path | None):
        self.bind_calls.append((node_id, logs_root))
        self._active_bindings += 1
        try:
            yield
        finally:
            self._active_bindings -= 1

    def run(
        self,
        node_id: str,
        prompt: str,
        context: Context,
        *,
        response_contract: str = "",
        contract_repair_attempts: int = 0,
        timeout=None,
        model=None,
        write_contract=None,
    ) -> str:
        del node_id, prompt, context, response_contract, contract_repair_attempts, timeout, model, write_contract
        self.run_bound = self._active_bindings > 0
        return self.response

class _PluginHandler:
    def execute(self, runtime):
        return Outcome(status=OutcomeStatus.SUCCESS, notes=f"plugin:{runtime.node_id}")

class _ExecuteOnlyHandler:
    def __init__(self):
        self.calls = []

    def execute(self, runtime):
        self.calls.append(runtime)
        return Outcome(status=OutcomeStatus.SUCCESS, notes=f"execute:{runtime.node_id}")

class _RuntimeCaptureHandler:
    def __init__(self):
        self.calls = []

    def execute(self, runtime):
        self.calls.append(runtime)
        return Outcome(status=OutcomeStatus.SUCCESS, notes="captured")

class _SlowHandler:
    def execute(self, runtime):
        time.sleep(0.2)
        return Outcome(status=OutcomeStatus.SUCCESS, notes="slow handler completed")

class _ConcurrentOutsideParallelHandler:
    def execute(self, runtime):
        targets = [edge.target for edge in runtime.outgoing_edges]
        if len(targets) < 2:
            return Outcome(status=OutcomeStatus.FAIL, failure_reason="need at least two branches")

        barrier = threading.Barrier(2)
        errors = []

        def invoke(target: str) -> None:
            try:
                barrier.wait(timeout=1.0)
                runtime.runner(target, "", runtime.context.clone())
            except Exception as exc:
                errors.append(str(exc))

        threads = [
            threading.Thread(target=invoke, args=(targets[0],), daemon=True),
            threading.Thread(target=invoke, args=(targets[1],), daemon=True),
        ]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(timeout=2.0)

        if any("parallel handlers" in message for message in errors):
            return Outcome(status=OutcomeStatus.SUCCESS, notes="concurrency gate enforced")
        return Outcome(
            status=OutcomeStatus.FAIL,
            failure_reason=f"unexpectedly allowed concurrent non-parallel handler execution: {errors}",
        )

class _SharedRefSeedHandler:
    def __init__(self, shared_ref):
        self.shared_ref = shared_ref

    def execute(self, runtime):
        return Outcome(
            status=OutcomeStatus.SUCCESS,
            context_updates={"shared_ref": self.shared_ref},
        )

class _SharedRefIsolationChecker:
    def __init__(self, marker: str, barrier: threading.Barrier):
        self.marker = marker
        self.barrier = barrier

    def execute(self, runtime):
        shared_ref = runtime.context.get("shared_ref", {})
        if not isinstance(shared_ref, dict):
            return Outcome(status=OutcomeStatus.FAIL, failure_reason="missing shared_ref dict")
        markers = shared_ref.setdefault("markers", [])
        if not isinstance(markers, list):
            return Outcome(status=OutcomeStatus.FAIL, failure_reason="shared_ref.markers must be list")
        markers.append(self.marker)
        try:
            self.barrier.wait(timeout=1.0)
        except threading.BrokenBarrierError:
            return Outcome(status=OutcomeStatus.FAIL, failure_reason="checker synchronization failed")

        if markers == [self.marker]:
            return Outcome(status=OutcomeStatus.SUCCESS, notes=f"isolated:{self.marker}")
        return Outcome(
            status=OutcomeStatus.FAIL,
            failure_reason=f"context leaked markers for {self.marker}: {markers}",
        )

class _MaxParallelProbeHandler:
    def __init__(self, state: dict[str, object], delay_s: float = 0.05):
        self.state = state
        self.delay_s = delay_s

    def execute(self, runtime):
        lock = self.state["lock"]
        with lock:
            self.state["in_flight"] += 1
            self.state["peak"] = max(self.state["peak"], self.state["in_flight"])
        try:
            time.sleep(self.delay_s)
        finally:
            with lock:
                self.state["in_flight"] -= 1
        return Outcome(status=OutcomeStatus.SUCCESS, notes=f"probe:{runtime.node_id}")

class _CustomConcurrencyProbeHandler:
    def __init__(self, state: dict[str, object], delay_s: float = 0.05):
        self.state = state
        self.delay_s = delay_s

    def execute(self, runtime):
        lock = self.state["lock"]
        with lock:
            self.state["in_flight"] += 1
            self.state["peak"] = max(self.state["peak"], self.state["in_flight"])
        try:
            time.sleep(self.delay_s)
        finally:
            with lock:
                self.state["in_flight"] -= 1
        return Outcome(status=OutcomeStatus.SUCCESS, notes=f"probe:{runtime.node_id}")

class _AlwaysSuccessHandler:
    def execute(self, runtime):
        return Outcome(status=OutcomeStatus.SUCCESS, notes=f"success:{runtime.node_id}")

class _AlwaysFailHandler:
    def execute(self, runtime):
        return Outcome(status=OutcomeStatus.FAIL, failure_reason=f"fail:{runtime.node_id}")

class _SystemExitHandler:
    def execute(self, runtime):
        del runtime
        raise SystemExit("handler terminated abruptly")

class _FalseyInterviewer(Interviewer):
    def __bool__(self) -> bool:
        return False

    def ask(self, question: Question) -> Answer:
        return Answer(selected_values=["Fix"])

__all__ = [
    "_StubBackend",
    "_ArtifactProbeBackend",
    "_TextBackend",
    "_OutcomeBackend",
    "_FanInRankingBackend",
    "_StageLoggingBackend",
    "_PluginHandler",
    "_ExecuteOnlyHandler",
    "_RuntimeCaptureHandler",
    "_SlowHandler",
    "_ConcurrentOutsideParallelHandler",
    "_SharedRefSeedHandler",
    "_SharedRefIsolationChecker",
    "_MaxParallelProbeHandler",
    "_CustomConcurrencyProbeHandler",
    "_AlwaysSuccessHandler",
    "_AlwaysFailHandler",
    "_SystemExitHandler",
    "_FalseyInterviewer",
]
