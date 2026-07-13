use std::path::Path;

use attractor_api::{AttractorApiService, PipelineStartRequest};
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;
use spark_http::build_app;
use tower::ServiceExt;

#[tokio::test]
async fn deprecated_workspace_and_attractor_event_routes_keep_text_410_contracts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app = build_app(settings(temp.path()));

    let conversation = request(
        app.clone(),
        "GET",
        "/workspace/api/conversations/missing/events",
    )
    .await;
    assert_eq!(conversation.status, StatusCode::GONE);
    assert_eq!(conversation.content_type, "text/plain; charset=utf-8");
    assert_eq!(
        conversation.body_text(),
        "Deprecated. Use /workspace/api/live/events with conversation_id and conversation_revision."
    );

    let runs = request(app, "GET", "/attractor/runs/events").await;
    assert_eq!(runs.status, StatusCode::GONE);
    assert_eq!(runs.content_type, "text/plain; charset=utf-8");
    assert_eq!(
        runs.body_text(),
        "Deprecated. Use /workspace/api/live/events with include_runs_overview=true."
    );
}

#[tokio::test]
async fn deprecated_pipeline_events_streams_gap_fill_with_sse_headers() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = AttractorApiService::new(settings.clone());
    service.start_pipeline(PipelineStartRequest {
        run_id: Some("run-http-events".to_string()),
        flow_content: Some(simple_flow()),
        working_directory: temp.path().join("Project").to_string_lossy().to_string(),
        ..PipelineStartRequest::default()
    });
    let app = build_app(settings);

    let response = request(
        app,
        "GET",
        "/attractor/pipelines/run-http-events/events?after_sequence=0",
    )
    .await;

    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.content_type, "text/event-stream");
    assert_eq!(response.cache_control.as_deref(), Some("no-cache"));
    assert_eq!(response.connection.as_deref(), Some("keep-alive"));
    let entries = sse_data_entries(&response.body_text());
    assert!(!entries.is_empty());
    let sequences = entries
        .iter()
        .map(|entry| entry["sequence"].as_u64().expect("sequence"))
        .collect::<Vec<_>>();
    let mut sorted_sequences = sequences.clone();
    sorted_sequences.sort_unstable();
    assert_eq!(sequences, sorted_sequences);
}

#[tokio::test]
async fn deprecated_pipeline_event_errors_and_route_boundaries_stay_json() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = AttractorApiService::new(settings.clone());
    service.start_pipeline(PipelineStartRequest {
        run_id: Some("run-http-errors".to_string()),
        flow_content: Some(simple_flow()),
        working_directory: temp.path().join("Project").to_string_lossy().to_string(),
        ..PipelineStartRequest::default()
    });
    let app = build_app(settings);

    let invalid_cursor = request(
        app.clone(),
        "GET",
        "/attractor/pipelines/run-http-errors/events?after_sequence=bad",
    )
    .await;
    assert_eq!(invalid_cursor.status, StatusCode::BAD_REQUEST);
    assert!(invalid_cursor.content_type.starts_with("application/json"));
    assert_eq!(
        invalid_cursor.body_json(),
        json!({"detail": "after_sequence must be zero or greater"})
    );

    let missing_pipeline = request(
        app.clone(),
        "GET",
        "/attractor/pipelines/missing/events?after_sequence=0",
    )
    .await;
    assert_eq!(missing_pipeline.status, StatusCode::NOT_FOUND);
    assert!(missing_pipeline
        .content_type
        .starts_with("application/json"));
    assert_eq!(
        missing_pipeline.body_json(),
        json!({"detail": "Unknown pipeline"})
    );

    for path in ["/workspace/api/missing", "/attractor/missing"] {
        let response = request(app.clone(), "GET", path).await;
        assert_eq!(response.status, StatusCode::NOT_FOUND, "{path}");
        assert!(
            response.content_type.starts_with("application/json"),
            "{path}"
        );
        assert_eq!(
            response.body_json(),
            json!({"detail": "Not Found"}),
            "{path}"
        );
    }
}

#[derive(Debug)]
struct TestResponse {
    status: StatusCode,
    content_type: String,
    cache_control: Option<String>,
    connection: Option<String>,
    body: Vec<u8>,
}

impl TestResponse {
    fn body_text(&self) -> String {
        String::from_utf8(self.body.clone()).expect("utf-8 body")
    }

    fn body_json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("json body")
    }
}

async fn request(app: axum::Router, method: &str, uri: &str) -> TestResponse {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body")
        .to_vec();
    TestResponse {
        status,
        content_type: header_value(&headers, "content-type").unwrap_or_default(),
        cache_control: header_value(&headers, "cache-control"),
        connection: header_value(&headers, "connection"),
        body,
    }
}

fn header_value(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

fn sse_data_entries(body: &str) -> Vec<Value> {
    body.split("\n\n")
        .filter_map(|frame| frame.strip_prefix("data: "))
        .map(|payload| serde_json::from_str::<Value>(payload).expect("sse data json"))
        .collect()
}

fn simple_flow() -> String {
    r#"
schema_version: '1'
id: api-inspect
title: API Inspect
nodes:
  start:
    kind: start
  task:
    kind: agent_task
    label: Task
    config:
      kind: agent_task
      prompt: Write an inspection note
  done:
    kind: exit
edges:
  - from: start
    to: task
  - from: task
    to: done
"#
    .to_string()
}

fn settings(root: &Path) -> SparkSettings {
    SparkSettings {
        project_root: root.join("source"),
        data_dir: root.join("spark-home"),
        config_dir: root.join("spark-home/config"),
        runtime_dir: root.join("spark-home/runtime"),
        logs_dir: root.join("spark-home/logs"),
        workspace_dir: root.join("spark-home/workspace"),
        projects_dir: root.join("spark-home/workspace/projects"),
        attractor_dir: root.join("spark-home/attractor"),
        runs_dir: root.join("spark-home/attractor/runs"),
        flows_dir: root.join("flows"),
        ui_dir: None,
        project_roots: Vec::new(),
    }
}
