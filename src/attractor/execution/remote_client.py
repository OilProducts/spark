from __future__ import annotations

from collections.abc import Iterator
import json
import os
from typing import Any

import httpx
from pydantic import ValidationError

from .errors import ExecutionLaunchError, ExecutionProtocolError, WorkerAPIError
from .models import WorkerProfile
from .worker_models import (
    WORKER_PROTOCOL_VERSION,
    WorkerCallbackRequest,
    WorkerCallbackResponse,
    WorkerCancelResponse,
    WorkerCleanupResponse,
    WorkerErrorResponse,
    WorkerEvent,
    WorkerHealthResponse,
    WorkerInfoResponse,
    WorkerNodeAcceptedResponse,
    WorkerNodeRequest,
    WorkerRunAdmissionRequest,
    WorkerRunAdmissionResponse,
    WorkerRunSnapshot,
)


class RemoteWorkerClient:
    """HTTP/SSE client for the selected v1 remote worker."""

    def __init__(
        self,
        worker: WorkerProfile,
        *,
        client: httpx.Client | None = None,
        timeout: float | httpx.Timeout | None = 30.0,
    ) -> None:
        self.worker = worker
        self._token = _resolve_worker_token(worker)
        self._owns_client = client is None
        self._client = client or httpx.Client(base_url=worker.base_url, timeout=timeout)

    def close(self) -> None:
        if self._owns_client:
            self._client.close()

    def __enter__(self) -> RemoteWorkerClient:
        return self

    def __exit__(self, *_exc_info: object) -> None:
        self.close()

    def health(self) -> WorkerHealthResponse:
        return self._request_model("GET", "/v1/health", WorkerHealthResponse)

    def worker_info(self) -> WorkerInfoResponse:
        return self._request_model("GET", "/v1/worker-info", WorkerInfoResponse)

    def admit_run(self, request: WorkerRunAdmissionRequest) -> WorkerRunAdmissionResponse:
        return self._request_model("POST", "/v1/runs", WorkerRunAdmissionResponse, json=request.model_dump(mode="json"))

    def get_run(self, run_id: str) -> WorkerRunSnapshot:
        return self._request_model("GET", f"/v1/runs/{run_id}", WorkerRunSnapshot)

    def submit_node(self, run_id: str, request: WorkerNodeRequest) -> WorkerNodeAcceptedResponse:
        return self._request_model(
            "POST",
            f"/v1/runs/{run_id}/nodes",
            WorkerNodeAcceptedResponse,
            json=request.model_dump(mode="json"),
        )

    def answer_human_gate(
        self,
        run_id: str,
        gate_id: str,
        request: WorkerCallbackRequest,
    ) -> WorkerCallbackResponse:
        return self._callback(f"/v1/runs/{run_id}/human-gates/{gate_id}/answer", request)

    def child_run_result(
        self,
        run_id: str,
        request_id: str,
        request: WorkerCallbackRequest,
    ) -> WorkerCallbackResponse:
        return self._callback(f"/v1/runs/{run_id}/child-runs/{request_id}/result", request)

    def child_status_result(
        self,
        run_id: str,
        request_id: str,
        request: WorkerCallbackRequest,
    ) -> WorkerCallbackResponse:
        return self._callback(f"/v1/runs/{run_id}/child-status/{request_id}/result", request)

    def cancel_run(self, run_id: str) -> WorkerCancelResponse:
        return self._request_model("POST", f"/v1/runs/{run_id}/cancel", WorkerCancelResponse)

    def cleanup_run(self, run_id: str) -> WorkerCleanupResponse:
        return self._request_model("DELETE", f"/v1/runs/{run_id}", WorkerCleanupResponse)

    def stream_events(
        self,
        run_id: str,
        *,
        after: int | None = None,
        last_event_id: str | None = None,
    ) -> Iterator[WorkerEvent]:
        params = {"after": after} if after is not None else None
        headers = self._headers()
        if last_event_id is not None:
            headers["Last-Event-ID"] = last_event_id
        try:
            with self._client.stream("GET", f"/v1/runs/{run_id}/events", params=params, headers=headers) as response:
                self._raise_for_worker_error(response)
                for event in _iter_worker_events(response.iter_lines(), self.worker.id):
                    yield event
        except httpx.HTTPError as exc:
            raise ExecutionLaunchError(f"Remote worker {self.worker.id!r} request failed: {exc}") from exc

    def _callback(self, path: str, request: WorkerCallbackRequest) -> WorkerCallbackResponse:
        return self._request_model("POST", path, WorkerCallbackResponse, json=request.model_dump(mode="json"))

    def _request_model(self, method: str, path: str, model_type: type[Any], **kwargs: Any) -> Any:
        response = self._request(method, path, **kwargs)
        self._raise_for_worker_error(response)
        payload = _response_json(response, self.worker.id, path)
        return _validate_compatible_model(model_type, payload, self.worker.id, path)

    def _request(self, method: str, path: str, **kwargs: Any) -> httpx.Response:
        headers = httpx.Headers(kwargs.pop("headers", None))
        headers.update(self._headers())
        try:
            return self._client.request(method, path, headers=headers, **kwargs)
        except httpx.HTTPError as exc:
            raise ExecutionLaunchError(f"Remote worker {self.worker.id!r} request failed: {exc}") from exc

    def _headers(self) -> dict[str, str]:
        return {"Authorization": f"Bearer {self._token}"}

    def _raise_for_worker_error(self, response: httpx.Response) -> None:
        if response.status_code < 400:
            return
        payload = _response_json(response, self.worker.id, str(response.request.url))
        try:
            error = WorkerErrorResponse.model_validate(payload).error
        except ValidationError as exc:
            raise ExecutionProtocolError(
                f"Remote worker {self.worker.id!r} returned HTTP {response.status_code} without a structured worker error."
            ) from exc
        raise WorkerAPIError(
            code=error.code,
            message=error.message,
            status_code=response.status_code,
            retryable=error.retryable,
            details=error.details,
        )


def _resolve_worker_token(worker: WorkerProfile) -> str:
    token = os.environ.get(worker.auth_token_env, "").strip()
    if not token:
        raise ExecutionLaunchError(
            f"Remote worker {worker.id!r} auth token env {worker.auth_token_env!r} is missing or empty."
        )
    return token


def _response_json(response: httpx.Response, worker_id: str, path: str) -> Any:
    try:
        return response.json()
    except json.JSONDecodeError as exc:
        raise ExecutionProtocolError(f"Remote worker {worker_id!r} returned non-JSON response for {path}.") from exc


def _validate_compatible_model(model_type: type[Any], payload: Any, worker_id: str, path: str) -> Any:
    if model_type in (WorkerHealthResponse, WorkerInfoResponse):
        _validate_advertised_protocol(payload, worker_id)
    try:
        model = model_type.model_validate(payload)
    except ValidationError as exc:
        raise ExecutionProtocolError(
            f"Remote worker {worker_id!r} returned an invalid {model_type.__name__} payload for {path}."
        ) from exc
    if isinstance(model, (WorkerHealthResponse, WorkerInfoResponse)):
        _validate_worker_compatibility(model, worker_id)
    return model


def _validate_advertised_protocol(payload: Any, worker_id: str) -> None:
    if not isinstance(payload, dict) or "protocol_version" not in payload:
        raise ExecutionProtocolError(f"Remote worker {worker_id!r} did not advertise protocol metadata.")


def _validate_worker_compatibility(model: WorkerHealthResponse | WorkerInfoResponse, worker_id: str) -> None:
    if model.protocol_version != WORKER_PROTOCOL_VERSION:
        raise ExecutionProtocolError(
            f"Remote worker {worker_id!r} advertises unsupported protocol {model.protocol_version!r}; "
            f"expected {WORKER_PROTOCOL_VERSION!r}."
        )
    if not model.worker_id.strip() or not model.worker_version.strip():
        raise ExecutionProtocolError(f"Remote worker {worker_id!r} did not advertise stable identity metadata.")


def _iter_worker_events(lines: Iterator[str], worker_id: str) -> Iterator[WorkerEvent]:
    block: list[str] = []
    for line in lines:
        if line:
            block.append(line)
            continue
        if block:
            yield _parse_worker_event(block, worker_id)
            block = []
    if block:
        yield _parse_worker_event(block, worker_id)


def _parse_worker_event(lines: list[str], worker_id: str) -> WorkerEvent:
    data_lines: list[str] = []
    for line in lines:
        field, separator, value = line.partition(":")
        if not separator:
            continue
        if field == "data":
            data_lines.append(value[1:] if value.startswith(" ") else value)
    if not data_lines:
        raise ExecutionProtocolError(f"Remote worker {worker_id!r} emitted an SSE event without data.")
    try:
        payload = json.loads("\n".join(data_lines))
        return WorkerEvent.model_validate(payload)
    except (json.JSONDecodeError, ValidationError) as exc:
        raise ExecutionProtocolError(f"Remote worker {worker_id!r} emitted an invalid worker event.") from exc
