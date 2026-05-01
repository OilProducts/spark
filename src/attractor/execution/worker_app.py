from __future__ import annotations

from collections.abc import AsyncIterator
import json
import os
from typing import Any, Iterable

import anyio
from fastapi import Depends, FastAPI, Header, Query, Request
from fastapi.exceptions import RequestValidationError
from fastapi.responses import JSONResponse, StreamingResponse
from starlette.exceptions import HTTPException as StarletteHTTPException

from .errors import WorkerAPIError, worker_error_payload
from .worker_models import (
    DEFAULT_WORKER_VERSION,
    WORKER_PROTOCOL_VERSION,
    WorkerCallbackRequest,
    WorkerCallbackResponse,
    WorkerCancelResponse,
    WorkerCleanupResponse,
    WorkerEvent,
    WorkerHealthResponse,
    WorkerInfoResponse,
    WorkerNodeAcceptedResponse,
    WorkerNodeRequest,
    WorkerRunAdmissionRequest,
    WorkerRunAdmissionResponse,
    WorkerRunSnapshot,
)
from .worker_runtime import LocalProcessWorkerRuntime, WorkerRuntime
from .worker_state import WorkerRunRecord, WorkerState


def create_worker_app(
    *,
    token: str | None = None,
    token_env: str = "SPARK_WORKER_TOKEN",
    worker_id: str = "spark-worker",
    worker_version: str = DEFAULT_WORKER_VERSION,
    capabilities: dict[str, Any] | None = None,
    supported_images: Iterable[str] | None = None,
    policies: dict[str, Any] | None = None,
    resource_labels: dict[str, str] | None = None,
    diagnostics: dict[str, Any] | None = None,
    runtime: WorkerRuntime | None = None,
) -> FastAPI:
    expected_token = token if token is not None else os.environ.get(token_env, "")
    capability_payload = dict(capabilities or {})
    policy_payload = dict(policies or {})
    image_payload = list(supported_images or [])
    resource_label_payload = dict(resource_labels or {})
    state = WorkerState(
        worker_id=worker_id,
        worker_version=worker_version,
        supported_images=image_payload,
        policies=policy_payload,
        capabilities=capability_payload,
        runtime=runtime,
    )

    app = FastAPI(title="Spark Worker API", docs_url="/docs", redoc_url=None, openapi_url="/openapi.json")
    app.state.worker_state = state

    async def require_bearer(authorization: str | None = Header(default=None)) -> None:
        if authorization is None or not authorization.startswith("Bearer ") or not authorization.removeprefix("Bearer ").strip():
            raise WorkerAPIError(
                "unauthorized",
                "Missing or malformed bearer credentials.",
                401,
                details={"scheme": "Bearer"},
            )
        supplied_token = authorization.removeprefix("Bearer ").strip()
        if supplied_token != expected_token or not expected_token:
            raise WorkerAPIError("forbidden", "Bearer credentials were rejected.", 403)

    @app.exception_handler(WorkerAPIError)
    async def _worker_api_error_handler(_request: Request, exc: WorkerAPIError) -> JSONResponse:
        return JSONResponse(exc.as_payload(), status_code=exc.status_code)

    @app.exception_handler(RequestValidationError)
    async def _validation_error_handler(_request: Request, exc: RequestValidationError) -> JSONResponse:
        return JSONResponse(
            worker_error_payload(
                "invalid_request",
                "Request validation failed.",
                details={"errors": exc.errors()},
            ),
            status_code=422,
        )

    @app.exception_handler(StarletteHTTPException)
    async def _http_error_handler(_request: Request, exc: StarletteHTTPException) -> JSONResponse:
        code = "not_found" if exc.status_code == 404 else "http_error"
        message = "Worker API route was not found." if exc.status_code == 404 else str(exc.detail)
        return JSONResponse(worker_error_payload(code, message), status_code=exc.status_code)

    @app.get("/v1/health", response_model=WorkerHealthResponse, dependencies=[Depends(require_bearer)])
    async def health() -> WorkerHealthResponse:
        return WorkerHealthResponse(
            worker_id=worker_id,
            worker_version=worker_version,
            status="ok",
            capabilities=capability_payload,
            diagnostics=diagnostics,
        )

    @app.get("/v1/worker-info", response_model=WorkerInfoResponse, dependencies=[Depends(require_bearer)])
    async def worker_info() -> WorkerInfoResponse:
        return WorkerInfoResponse(
            worker_id=worker_id,
            worker_version=worker_version,
            status="ok",
            capabilities=capability_payload,
            supported_images=image_payload,
            policies=policy_payload,
            resource_labels=resource_label_payload,
            version_details={"worker_version": worker_version, "protocol_version": WORKER_PROTOCOL_VERSION},
            validation_details={"profile_selection": "control_plane_owned"},
        )

    @app.post(
        "/v1/runs",
        response_model=WorkerRunAdmissionResponse,
        status_code=202,
        dependencies=[Depends(require_bearer)],
    )
    async def admit_run(request: WorkerRunAdmissionRequest) -> WorkerRunAdmissionResponse:
        record, admission_status, admission_last_sequence = state.admit_run(request)
        return WorkerRunAdmissionResponse(
            run_id=request.run_id,
            worker_id=worker_id,
            status=admission_status,
            event_url=f"/v1/runs/{record.request.run_id}/events",
            last_sequence=admission_last_sequence,
        )

    @app.get("/v1/runs/{run_id}", response_model=WorkerRunSnapshot, dependencies=[Depends(require_bearer)])
    async def get_run(run_id: str) -> WorkerRunSnapshot:
        return state.snapshot(state.require_run(run_id))

    @app.post(
        "/v1/runs/{run_id}/nodes",
        response_model=WorkerNodeAcceptedResponse,
        status_code=202,
        dependencies=[Depends(require_bearer)],
    )
    async def submit_node(run_id: str, request: WorkerNodeRequest) -> WorkerNodeAcceptedResponse:
        node_execution_id = await anyio.to_thread.run_sync(state.accept_node, run_id, request)
        return WorkerNodeAcceptedResponse(
            run_id=run_id,
            node_execution_id=node_execution_id,
            node_id=request.node_id,
            attempt=request.attempt,
        )

    @app.get("/v1/runs/{run_id}/events", dependencies=[Depends(require_bearer)])
    async def stream_events(
        request: Request,
        run_id: str,
        after: int | None = Query(default=None, ge=0),
        limit: int | None = Query(default=None, ge=1),
        last_event_id: str | None = Header(default=None, alias="Last-Event-ID"),
    ) -> StreamingResponse:
        record = state.require_run(run_id)
        start_after = _resolve_event_cursor(after=after, last_event_id=last_event_id)
        return StreamingResponse(
            _stream_sse_events(request, state, record, start_after, limit),
            media_type="text/event-stream",
        )

    @app.post(
        "/v1/runs/{run_id}/human-gates/{gate_id}/answer",
        response_model=WorkerCallbackResponse,
        dependencies=[Depends(require_bearer)],
    )
    async def answer_human_gate(run_id: str, gate_id: str, request: WorkerCallbackRequest) -> WorkerCallbackResponse:
        _record_callback(state.observe_control_plane(run_id), f"human_gate:{gate_id}", request.payload)
        return WorkerCallbackResponse(run_id=run_id, request_id=gate_id)

    @app.post(
        "/v1/runs/{run_id}/child-runs/{request_id}/result",
        response_model=WorkerCallbackResponse,
        dependencies=[Depends(require_bearer)],
    )
    async def child_run_result(run_id: str, request_id: str, request: WorkerCallbackRequest) -> WorkerCallbackResponse:
        _record_callback(state.observe_control_plane(run_id), f"child_run:{request_id}", request.payload)
        return WorkerCallbackResponse(run_id=run_id, request_id=request_id)

    @app.post(
        "/v1/runs/{run_id}/child-status/{request_id}/result",
        response_model=WorkerCallbackResponse,
        dependencies=[Depends(require_bearer)],
    )
    async def child_status_result(run_id: str, request_id: str, request: WorkerCallbackRequest) -> WorkerCallbackResponse:
        _record_callback(state.observe_control_plane(run_id), f"child_status:{request_id}", request.payload)
        return WorkerCallbackResponse(run_id=run_id, request_id=request_id)

    @app.post("/v1/runs/{run_id}/cancel", response_model=WorkerCancelResponse, dependencies=[Depends(require_bearer)])
    async def cancel_run(run_id: str) -> WorkerCancelResponse:
        record = state.cancel_run(run_id)
        return WorkerCancelResponse(run_id=run_id, status=record.status)

    @app.delete("/v1/runs/{run_id}", response_model=WorkerCleanupResponse, dependencies=[Depends(require_bearer)])
    async def cleanup_run(run_id: str) -> WorkerCleanupResponse:
        status, deleted = state.cleanup_run(run_id)
        return WorkerCleanupResponse(run_id=run_id, status=status, deleted=deleted)

    return app


def create_worker_app_from_env() -> FastAPI:
    return create_worker_app(
        token_env=os.environ.get("SPARK_WORKER_TOKEN_ENV", "SPARK_WORKER_TOKEN"),
        worker_id=os.environ.get("SPARK_WORKER_ID", "spark-worker"),
        worker_version=os.environ.get("SPARK_WORKER_VERSION", DEFAULT_WORKER_VERSION),
        runtime=LocalProcessWorkerRuntime(),
    )


def _resolve_event_cursor(*, after: int | None, last_event_id: str | None) -> int:
    if after is not None:
        return after
    if not last_event_id:
        return 0
    try:
        return int(last_event_id)
    except ValueError as exc:
        raise WorkerAPIError(
            "invalid_request",
            "Last-Event-ID must be an integer sequence.",
            400,
            details={"last_event_id": last_event_id},
        ) from exc


def _serialize_sse_events(events: Iterable[WorkerEvent]) -> Iterable[str]:
    for event in events:
        yield _serialize_sse_event(event)


async def _stream_sse_events(
    request: Request,
    state: WorkerState,
    record: WorkerRunRecord,
    start_after: int,
    limit: int | None,
) -> AsyncIterator[str]:
    cursor = start_after
    emitted = 0
    while True:
        events = state.event_store.after(record, cursor)
        if events:
            for event in events:
                cursor = event.sequence
                emitted += 1
                yield _serialize_sse_event(event)
                if limit is not None and emitted >= limit:
                    return
            continue
        if await request.is_disconnected():
            return
        await anyio.to_thread.run_sync(state.event_store.wait_for_event_after, record, cursor, 0.25)


def _serialize_sse_event(event: WorkerEvent) -> str:
    data = event.model_dump(mode="json")
    return f"id: {event.sequence}\nevent: {event.event_type}\ndata: {json.dumps(data, separators=(',', ':'))}\n\n"


def _record_callback(record: WorkerRunRecord, callback_id: str, payload: dict[str, Any]) -> None:
    with record.event_condition:
        existing = record.callbacks.get(callback_id)
        if existing is not None:
            if existing == payload:
                return
            raise WorkerAPIError(
                "conflict",
                "Callback result conflicts with an earlier delivery.",
                409,
                details={"callback_id": callback_id},
            )
        record.callbacks[callback_id] = dict(payload)
        record.event_condition.notify_all()
