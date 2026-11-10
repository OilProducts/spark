#![forbid(unsafe_code)]

//! Axum HTTP composition for Spark Workspace compatibility routes.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{FromRef, Path, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use axum::{Json, Router};
use serde_json::json;
use serde_json::Value;
use spark_agent_adapter::{AgentTurnBackend, RustLlmAgentTurnBackend};
use spark_common::settings::SparkSettings;
use spark_workspace::live::{
    execution_transcript_envelopes, full_run_usage_accumulator, latest_run_sequence,
    run_live_publication, run_upsert_envelope_with_usage, trigger_upsert_envelope,
    ExecutionTranscriptCursor, LiveEnvelope, RunUsageAccumulator,
};
use spark_workspace::{WorkspaceError, WorkspaceTriggerService};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{self, Duration};
use tokio_util::sync::CancellationToken;

mod claude_auth;
mod codex_auth;
mod workspace;

/// Builders open the Spark home's conversation store and hold its exclusive
/// home lock for the app's lifetime; a second app on the same home fails here
/// with [`spark_storage::StorageError::HomeLocked`]. A failed build releases
/// everything it opened.
pub fn build_app(settings: SparkSettings) -> Result<Router, spark_storage::StorageError> {
    let agent_turn_backend =
        spark_workspace::WorkspaceConversationService::default_agent_turn_backend(&settings);
    build_app_with_runtime_handler_runner_factory_and_agent_turn_backend(
        settings,
        attractor_api::default_runtime_handler_runner_factory(),
        agent_turn_backend,
    )
}

pub fn build_app_with_rust_llm_client(
    settings: SparkSettings,
    client: unified_llm_adapter::Client,
) -> Result<Router, spark_storage::StorageError> {
    let agent_turn_backend: Arc<dyn AgentTurnBackend> =
        Arc::new(RustLlmAgentTurnBackend::new(client.clone()));
    build_app_with_runtime_handler_runner_factory_and_agent_turn_backend(
        settings,
        attractor_api::rust_llm_runtime_handler_runner_factory(client),
        agent_turn_backend,
    )
}

/// Native settings writes share the HTTP app's existing live transport.
pub fn build_app_with_rust_llm_client_and_settings_notifications(
    settings: SparkSettings,
    client: unified_llm_adapter::Client,
) -> Result<(Router, impl Fn(&str) + Send + Sync + 'static), spark_storage::StorageError> {
    let (app, publisher) = build_app_with_live_hub(
        settings,
        attractor_api::rust_llm_runtime_handler_runner_factory(client.clone()),
        Arc::new(RustLlmAgentTurnBackend::new(client)),
    )?;
    Ok((app, move |revision: &str| {
        publisher.publish_settings_change(
            "workspace",
            "desktop",
            None,
            serde_json::json!(revision),
        );
    }))
}

pub fn build_app_with_agent_turn_backend(
    settings: SparkSettings,
    agent_turn_backend: Arc<dyn AgentTurnBackend>,
) -> Result<Router, spark_storage::StorageError> {
    build_app_with_runtime_handler_runner_factory_and_agent_turn_backend(
        settings,
        attractor_api::default_runtime_handler_runner_factory(),
        agent_turn_backend,
    )
}

pub fn build_app_with_runtime_handler_runner_factory(
    settings: SparkSettings,
    runtime_handler_runner_factory: attractor_api::RuntimeHandlerRunnerFactory,
) -> Result<Router, spark_storage::StorageError> {
    let agent_turn_backend =
        spark_workspace::WorkspaceConversationService::default_agent_turn_backend(&settings);
    build_app_with_runtime_handler_runner_factory_and_agent_turn_backend(
        settings,
        runtime_handler_runner_factory,
        agent_turn_backend,
    )
}

pub fn build_app_with_runtime_handler_runner_factory_and_agent_turn_backend(
    settings: SparkSettings,
    runtime_handler_runner_factory: attractor_api::RuntimeHandlerRunnerFactory,
    agent_turn_backend: Arc<dyn AgentTurnBackend>,
) -> Result<Router, spark_storage::StorageError> {
    build_app_with_live_hub(settings, runtime_handler_runner_factory, agent_turn_backend)
        .map(|(app, _)| app)
}

/// Builds the app on a conversation store its caller already opened (and so
/// already holds the home lock), for callers that also use that store
/// directly, such as tests. The store must belong to the settings' Spark
/// home; a mismatch is refused before any startup or recovery work.
pub fn build_app_with_conversation_store(
    settings: SparkSettings,
    conversation_store: Arc<spark_storage::ConversationStore>,
    agent_turn_backend: Arc<dyn AgentTurnBackend>,
) -> Result<Router, spark_storage::StorageError> {
    let normalized =
        |path: &std::path::Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if normalized(&settings.data_dir) != normalized(conversation_store.home_dir()) {
        return Err(spark_storage::StorageError::InvalidConversationState {
            path: settings.data_dir.clone(),
            reason: format!(
                "The conversation store belongs to Spark home {}, not {}.",
                conversation_store.home_dir().display(),
                settings.data_dir.display()
            ),
        });
    }
    Ok(build_app_on_store(
        settings,
        conversation_store,
        attractor_api::default_runtime_handler_runner_factory(),
        agent_turn_backend,
    )
    .0)
}

fn build_app_with_live_hub(
    settings: SparkSettings,
    runtime_handler_runner_factory: attractor_api::RuntimeHandlerRunnerFactory,
    agent_turn_backend: Arc<dyn AgentTurnBackend>,
) -> Result<(Router, Arc<WorkspaceLiveHub>), spark_storage::StorageError> {
    // Before any recovery or request: the home lock and its database.
    let conversation_store = spark_storage::ConversationStore::open(&settings.data_dir)?;
    Ok(build_app_on_store(
        settings,
        conversation_store,
        runtime_handler_runner_factory,
        agent_turn_backend,
    ))
}

fn build_app_on_store(
    settings: SparkSettings,
    conversation_store: Arc<spark_storage::ConversationStore>,
    runtime_handler_runner_factory: attractor_api::RuntimeHandlerRunnerFactory,
    agent_turn_backend: Arc<dyn AgentTurnBackend>,
) -> (Router, Arc<WorkspaceLiveHub>) {
    let live_hub = Arc::new(WorkspaceLiveHub::new(&conversation_store));
    // Every committed conversation write wakes the subscribers it concerns.
    let notified_hub = Arc::downgrade(&live_hub);
    conversation_store.set_commit_listener(Arc::new(move |notice| {
        if let Some(hub) = notified_hub.upgrade() {
            hub.notify_commit(notice);
        }
    }));
    let settings = Arc::new(settings);
    let (run_event_observer, run_event_publisher) =
        RunEventPublisher::spawn(settings.clone(), live_hub.clone());
    let state = HttpAppState {
        claude_connection: Arc::new(Mutex::new(Default::default())),
        codex_connection: Arc::new(Mutex::new(Default::default())),
        settings,
        conversation_store,
        live_hub: live_hub.clone(),
        runtime_handler_runner_factory,
        agent_turn_backend,
        run_event_observer,
        trigger_source_loop: None,
        run_event_publisher,
    };
    let state = state.with_trigger_source_loop();
    // Runs execute as threads, so any previous process's restart orphaned its
    // non-terminal runs; re-arm them before serving traffic. Resumption is
    // detached — this does not block startup.
    {
        let mut service =
            attractor_api::AttractorApiService::new_with_runtime_handler_runner_factory(
                (*state.settings).clone(),
                state.runtime_handler_runner_factory.clone(),
            );
        if let Some(observer) = state.run_event_observer.0.clone() {
            service = service.with_run_event_observer(observer);
        }
        if let Err(error) = spark_workspace::WorkspaceProjectService::new((*state.settings).clone())
            .ensure_home_project()
        {
            eprintln!("Unable to register the Home project: {error}");
        }
        // Triggers removed while no server ran lose their activation; the
        // source loop reconciles again before each dispatch.
        match spark_workspace::WorkspaceTriggerService::new(
            (*state.settings).clone(),
            state.conversation_store.clone(),
        )
        .reconcile_triggers()
        {
            Ok(issues) => {
                for issue in issues {
                    eprintln!(
                        "Trigger {} in {} cannot run: {}",
                        issue.trigger_id.as_deref().unwrap_or("definitions"),
                        issue.project_path,
                        issue.error
                    );
                }
            }
            Err(error) => eprintln!("Unable to reconcile triggers: {error}"),
        }
        let _ = service.recover_interrupted_runs();
        // Likewise every chat turn the previous process left in flight.
        let _ = spark_workspace::WorkspaceConversationService::new(
            (*state.settings).clone(),
            state.conversation_store.clone(),
        )
        .settle_orphaned_turns();
        let live_hub = state.live_hub.clone();
        spark_workspace::inbox::install_runtime(
            &state.settings,
            spark_workspace::inbox::InboxRuntime {
                agent_turn_backend: state.agent_turn_backend.clone(),
                publish: Arc::new(move |envelope| live_hub.publish(envelope)),
            },
        );
        // Then every conversation's inbox: runs that reported while no
        // server ran, and events still waiting for a turn.
        if let Err(error) =
            spark_workspace::inbox::recover(&state.settings, &state.conversation_store)
        {
            eprintln!("Unable to recover conversation inboxes: {error}");
        }
    }
    let app = Router::new()
        .route("/", get(serve_index))
        .route("/favicon.ico", get(serve_favicon))
        .route("/assets/{*asset_path}", get(serve_asset))
        .route("/workspace", any(redirect_workspace_mount))
        .nest(
            "/workspace/api",
            workspace::router().fallback(workspace_api_fallback),
        )
        .route("/attractor", any(redirect_attractor_mount))
        .route(
            "/attractor/api/llm-profiles",
            get(attractor_dispatch).patch(workspace::patch_llm_profiles),
        )
        .route(
            "/attractor/api/execution-placement-settings",
            get(attractor_dispatch).patch(workspace::patch_execution_profiles),
        )
        .route("/attractor/{*path}", any(attractor_dispatch))
        .fallback(product_fallback)
        .with_state(state);
    (app, live_hub)
}

#[derive(Clone)]
pub(crate) struct HttpAppState {
    claude_connection: Arc<Mutex<spark_agent_adapter::claude_code::auth::ClaudeCodeConnection>>,
    codex_connection: Arc<Mutex<codex_auth::ConnectionState>>,
    settings: Arc<SparkSettings>,
    conversation_store: Arc<spark_storage::ConversationStore>,
    live_hub: Arc<WorkspaceLiveHub>,
    runtime_handler_runner_factory: attractor_api::RuntimeHandlerRunnerFactory,
    agent_turn_backend: Arc<dyn AgentTurnBackend>,
    run_event_observer: RunEventObserverHandle,
    #[allow(dead_code)]
    trigger_source_loop: Option<Arc<TriggerSourceLoop>>,
    #[allow(dead_code)]
    run_event_publisher: Option<Arc<RunEventPublisher>>,
}

/// The app-wide run-event observer handed to every execution surface; fires
/// into the coalescing publisher so background runs stream live updates.
#[derive(Clone, Default)]
pub(crate) struct RunEventObserverHandle(pub(crate) Option<attractor_api::RunEventObserver>);

impl FromRef<HttpAppState> for RunEventObserverHandle {
    fn from_ref(input: &HttpAppState) -> Self {
        input.run_event_observer.clone()
    }
}

impl FromRef<HttpAppState> for Arc<SparkSettings> {
    fn from_ref(input: &HttpAppState) -> Self {
        input.settings.clone()
    }
}

impl FromRef<HttpAppState> for Arc<spark_storage::ConversationStore> {
    fn from_ref(input: &HttpAppState) -> Self {
        input.conversation_store.clone()
    }
}

impl FromRef<HttpAppState> for Arc<WorkspaceLiveHub> {
    fn from_ref(input: &HttpAppState) -> Self {
        input.live_hub.clone()
    }
}

impl FromRef<HttpAppState> for attractor_api::RuntimeHandlerRunnerFactory {
    fn from_ref(input: &HttpAppState) -> Self {
        input.runtime_handler_runner_factory.clone()
    }
}

impl FromRef<HttpAppState> for Arc<dyn AgentTurnBackend> {
    fn from_ref(input: &HttpAppState) -> Self {
        input.agent_turn_backend.clone()
    }
}

/// Notifications a subscriber's queue holds before it overflows.
const SUBSCRIBER_QUEUE_ENVELOPES: usize = 256;
/// Serialized bytes a subscriber's queue holds before it overflows.
const SUBSCRIBER_QUEUE_BYTES: usize = 2 * 1024 * 1024;

/// The app's live transport: one bounded, filtered queue per subscriber, so a
/// slow subscriber overflows alone. It carries the app's conversation store so
/// run-event publication can hand terminal runs to their owning
/// conversations without opening another.
pub(crate) struct WorkspaceLiveHub {
    subscribers: Mutex<Vec<std::sync::Weak<LiveSubscriber>>>,
    execution_cursors: Mutex<HashMap<String, ExecutionTranscriptCursor>>,
    /// Thinking in progress and when it began, for clients that subscribe
    /// meanwhile. In memory only, keyed by resource kind and id.
    active_thinking: Mutex<HashMap<(String, String), (std::time::Instant, LiveEnvelope)>>,
    /// The latest whole-text frame of each open segment, keyed by
    /// conversation, owning turn and segment, for clients that subscribe or
    /// lag meanwhile. Display-only and in memory; commit notices retire the
    /// frames of segments that closed and executions that settled.
    segment_frames: Mutex<HashMap<(String, String, String), LiveEnvelope>>,
    /// Not owned: background tasks and installed runtimes hold the hub, and
    /// must not keep the home locked after the app is dropped.
    conversation_store: std::sync::Weak<spark_storage::ConversationStore>,
}

/// What a subscriber reads next.
#[derive(Debug)]
pub(crate) enum Received {
    Envelope(LiveEnvelope),
    /// The queue overflowed and these many queued notifications were dropped;
    /// the subscriber resynchronizes from durable state.
    Lagged(#[cfg_attr(not(test), allow(dead_code))] u64),
    /// Committed conversation rows changed in its scopes: a wakeup to read
    /// them, not the rows themselves.
    Changed,
}

/// One subscriber's queue. Publication never blocks: a full queue drops a
/// transient token frame first, and otherwise drains itself and raises the
/// out-of-band overflow count, which no queued item can displace.
pub(crate) struct LiveSubscriber {
    /// `None` receives every envelope.
    query: Option<spark_workspace::live::LiveQuery>,
    queue: Mutex<(std::collections::VecDeque<(LiveEnvelope, usize)>, usize)>,
    lagged: std::sync::atomic::AtomicU64,
    changed: std::sync::atomic::AtomicBool,
    notify: tokio::sync::Notify,
}

impl LiveSubscriber {
    fn push(&self, envelope: LiveEnvelope) {
        let is_transient = |event_type: &str| {
            event_type == "conversation.stream_delta"
                || event_type == spark_workspace::live::NODE_THINKING_EVENT_TYPE
        };
        let transient = is_transient(&envelope.event_type);
        let bytes = serde_json::to_vec(&envelope).map_or(0, |bytes| bytes.len());
        {
            let mut queue = self
                .queue
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let full = |queue: &(std::collections::VecDeque<(LiveEnvelope, usize)>, usize)| {
                queue.0.len() >= SUBSCRIBER_QUEUE_ENVELOPES
                    || queue.1 + bytes > SUBSCRIBER_QUEUE_BYTES
            };
            while full(&queue) {
                if transient {
                    return;
                }
                let Some(index) = queue
                    .0
                    .iter()
                    .position(|(queued, _)| is_transient(&queued.event_type))
                else {
                    let dropped = queue.0.len() as u64 + 1;
                    queue.0.clear();
                    queue.1 = 0;
                    self.lagged
                        .fetch_add(dropped, std::sync::atomic::Ordering::SeqCst);
                    drop(queue);
                    self.notify.notify_one();
                    return;
                };
                if let Some((_, dropped)) = queue.0.remove(index) {
                    queue.1 -= dropped;
                }
            }
            queue.0.push_back((envelope, bytes));
            queue.1 += bytes;
        }
        self.notify.notify_one();
    }

    fn wake_changed(&self) {
        self.changed
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.notify.notify_one();
    }

    pub(crate) fn try_recv(&self) -> Option<Received> {
        let lagged = self.lagged.swap(0, std::sync::atomic::Ordering::SeqCst);
        if lagged > 0 {
            return Some(Received::Lagged(lagged));
        }
        let next = {
            let mut queue = self
                .queue
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let next = queue.0.pop_front();
            if let Some((_, bytes)) = &next {
                queue.1 -= bytes;
            }
            next
        };
        if let Some((envelope, _)) = next {
            return Some(Received::Envelope(envelope));
        }
        self.changed
            .swap(false, std::sync::atomic::Ordering::SeqCst)
            .then_some(Received::Changed)
    }

    pub(crate) async fn recv(&self) -> Received {
        loop {
            let notified = self.notify.notified();
            if let Some(received) = self.try_recv() {
                return received;
            }
            notified.await;
        }
    }
}

impl WorkspaceLiveHub {
    fn new(conversation_store: &Arc<spark_storage::ConversationStore>) -> Self {
        Self {
            subscribers: Mutex::new(Vec::new()),
            execution_cursors: Mutex::new(HashMap::new()),
            active_thinking: Mutex::new(HashMap::new()),
            segment_frames: Mutex::new(HashMap::new()),
            conversation_store: Arc::downgrade(conversation_store),
        }
    }

    pub(crate) fn conversation_store(&self) -> Option<Arc<spark_storage::ConversationStore>> {
        self.conversation_store.upgrade()
    }

    /// A queue receiving every envelope.
    #[cfg(test)]
    pub(crate) fn subscribe(&self) -> Arc<LiveSubscriber> {
        self.subscribe_with(None)
    }

    /// A queue receiving the envelopes and conversation wakeups `query` follows.
    pub(crate) fn subscribe_with(
        &self,
        query: Option<spark_workspace::live::LiveQuery>,
    ) -> Arc<LiveSubscriber> {
        let subscriber = Arc::new(LiveSubscriber {
            query,
            queue: Mutex::new((std::collections::VecDeque::new(), 0)),
            lagged: Default::default(),
            changed: Default::default(),
            notify: tokio::sync::Notify::new(),
        });
        let mut subscribers = self
            .subscribers
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        subscribers.retain(|subscriber| subscriber.strong_count() > 0);
        subscribers.push(Arc::downgrade(&subscriber));
        subscriber
    }

    fn live_subscribers(&self) -> Vec<Arc<LiveSubscriber>> {
        self.subscribers
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .iter()
            .filter_map(std::sync::Weak::upgrade)
            .collect()
    }

    /// Wake the subscribers whose conversation scopes a committed write
    /// touched. They read the changed rows from the database themselves.
    pub(crate) fn notify_commit(&self, notice: &spark_storage::conversation::CommitNotice) {
        if !notice.closed_segments.is_empty() || !notice.settled_executions.is_empty() {
            // Retired before the wakeup, so a catch-up that reads the closed
            // row never replays its frame.
            self.segment_frames
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .retain(|(conversation, turn, segment), _| {
                    !notice
                        .closed_segments
                        .iter()
                        .any(|(closed, _, id)| closed == conversation && id == segment)
                        && !notice
                            .settled_executions
                            .iter()
                            .any(|(settled, owner)| settled == conversation && owner == turn)
                });
        }
        for subscriber in self.live_subscribers() {
            let Some(query) = subscriber.query.as_ref() else {
                continue;
            };
            let relevant = notice.conversations.iter().any(|(id, project_path)| {
                query.conversation_id.as_deref() == Some(id.as_str())
                    || (query.include_conversations
                        && query
                            .conversations_project_path
                            .as_deref()
                            .is_none_or(|filter| filter == project_path))
            });
            if relevant {
                subscriber.wake_changed();
            }
        }
    }

    pub(crate) fn publish_settings_change(
        &self,
        scope: &str,
        section: &str,
        project_path: Option<String>,
        revision: serde_json::Value,
    ) {
        if scope == "workspace" {
            spark_workspace::models::invalidate_model_discovery(section);
        }
        self.publish(LiveEnvelope {
            event_type: "settings.changed".into(),
            project_path,
            resource: spark_workspace::LiveResource {
                kind: "settings".into(),
                id: Some(scope.into()),
            },
            cursor: None,
            reason: None,
            payload: serde_json::json!({"scope": scope, "section": section, "revision": revision}),
        });
    }

    pub(crate) fn publish(&self, envelope: LiveEnvelope) {
        self.track_thinking(&envelope);
        self.track_segment_frame(&envelope);
        for subscriber in self.live_subscribers() {
            let matches = subscriber.query.as_ref().is_none_or(|query| {
                spark_workspace::live::envelope_matches_query(&envelope, query)
            });
            if matches {
                subscriber.push(envelope.clone());
            }
        }
    }

    fn track_thinking(&self, envelope: &LiveEnvelope) {
        if envelope.event_type != spark_workspace::live::NODE_THINKING_EVENT_TYPE
            && envelope.payload["delta_kind"] != "thinking"
        {
            return;
        }
        let Some(id) = envelope.resource.id.clone() else {
            return;
        };
        let key = (envelope.resource.kind.clone(), id);
        let mut active = self
            .active_thinking
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if envelope.payload["active"] == true {
            let started = active
                .get(&key)
                .map_or_else(std::time::Instant::now, |(started, _)| *started);
            active.insert(key, (started, envelope.clone()));
        } else {
            active.remove(&key);
        }
    }

    fn track_segment_frame(&self, envelope: &LiveEnvelope) {
        let payload = &envelope.payload;
        if envelope.event_type != "conversation.stream_delta"
            || payload["delta_kind"] != "segment_text"
        {
            return;
        }
        let (Some(conversation), Some(turn), Some(segment)) = (
            envelope.resource.id.clone(),
            payload["turn_id"].as_str(),
            payload["segment_id"].as_str(),
        ) else {
            return;
        };
        let sequence = |envelope: &LiveEnvelope| envelope.payload["stream_sequence"].as_u64();
        let mut frames = self
            .segment_frames
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let key = (conversation, turn.to_string(), segment.to_string());
        if frames
            .get(&key)
            .is_none_or(|kept| sequence(kept) < sequence(envelope))
        {
            frames.insert(key, envelope.clone());
        }
    }

    /// The latest frames of the open segments `query` covers, in sequence
    /// order. Sent after a committed baseline, whose closed rows fence any
    /// that closed meanwhile.
    pub(crate) fn segment_frame_envelopes(
        &self,
        query: &spark_workspace::live::LiveQuery,
    ) -> Vec<LiveEnvelope> {
        let mut frames: Vec<LiveEnvelope> = self
            .segment_frames
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .values()
            .filter(|envelope| spark_workspace::live::envelope_matches_query(envelope, query))
            .cloned()
            .collect();
        frames.sort_by_key(|envelope| envelope.payload["stream_sequence"].as_u64());
        frames
    }

    /// The thinking in progress that `query` covers, with how long it has run,
    /// so a client that just subscribed shows what it missed the start of.
    /// Thinking that already ended is simply absent.
    pub(crate) fn active_thinking_envelopes(
        &self,
        query: &spark_workspace::live::LiveQuery,
    ) -> Vec<LiveEnvelope> {
        self.active_thinking
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .values()
            .filter(|(_, envelope)| spark_workspace::live::envelope_matches_query(envelope, query))
            .map(|(started, envelope)| {
                let mut envelope = envelope.clone();
                envelope.payload["elapsed_ms"] = json!(started.elapsed().as_millis() as u64);
                envelope
            })
            .collect()
    }

    /// A finished run's agents have stopped, whatever they last reported.
    fn forget_run_thinking(&self, run_id: &str) {
        self.active_thinking
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .retain(|_, (_, envelope)| {
                envelope.event_type != spark_workspace::live::NODE_THINKING_EVENT_TYPE
                    || envelope.payload["run_id"] != run_id
            });
    }

    fn publish_execution_transcripts(&self, settings: &SparkSettings, run_id: &str) {
        let mut cursors = self
            .execution_cursors
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Ok(envelopes) = execution_transcript_envelopes(settings, run_id, &mut cursors) {
            for envelope in envelopes {
                self.publish(envelope);
            }
        }
    }
}

struct RunEventPublisher {
    cancellation: CancellationToken,
    handle: JoinHandle<()>,
}

impl RunEventPublisher {
    /// Spawns the coalescing publisher and returns the observer that feeds
    /// it. Without a tokio runtime (pure-sync callers) both are inert.
    fn spawn(
        settings: Arc<SparkSettings>,
        live_hub: Arc<WorkspaceLiveHub>,
    ) -> (RunEventObserverHandle, Option<Arc<Self>>) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return (RunEventObserverHandle(None), None);
        };
        let (sender, receiver) = mpsc::unbounded_channel::<String>();
        let observer: attractor_api::RunEventObserver = Arc::new(move |run_id: &str| {
            let _ = sender.send(run_id.to_string());
        });
        let cancellation = CancellationToken::new();
        let task_cancellation = cancellation.clone();
        let handle = handle.spawn(run_event_publisher_loop(
            settings,
            live_hub,
            receiver,
            task_cancellation,
            observer.clone(),
        ));
        (
            RunEventObserverHandle(Some(observer)),
            Some(Arc::new(Self {
                cancellation,
                handle,
            })),
        )
    }
}

impl Drop for RunEventPublisher {
    fn drop(&mut self) {
        self.cancellation.cancel();
        self.handle.abort();
    }
}

/// Drains run-event notifications, coalescing bursts per run (~120ms) before
/// publishing journal deltas, run.upserts, and terminal trigger events
/// through publish_live_run_after.
async fn run_event_publisher_loop(
    settings: Arc<SparkSettings>,
    live_hub: Arc<WorkspaceLiveHub>,
    mut receiver: mpsc::UnboundedReceiver<String>,
    cancellation: CancellationToken,
    run_event_observer: attractor_api::RunEventObserver,
) {
    let mut last_published_sequence: HashMap<String, u64> = HashMap::new();
    let mut usage_accumulators: HashMap<String, RunUsageAccumulator> = HashMap::new();
    loop {
        let first = tokio::select! {
            _ = cancellation.cancelled() => return,
            received = receiver.recv() => received,
        };
        let Some(first) = first else {
            return;
        };
        let mut pending: BTreeSet<String> = BTreeSet::new();
        pending.insert(first);
        // Coalesce the burst: chatty executors notify per journal append.
        tokio::select! {
            _ = cancellation.cancelled() => return,
            _ = time::sleep(Duration::from_millis(120)) => {}
        }
        while let Ok(run_id) = receiver.try_recv() {
            pending.insert(run_id);
        }
        for run_id in pending {
            let before_sequence = last_published_sequence.get(&run_id).copied();
            let publish_settings = settings.clone();
            let publish_hub = live_hub.clone();
            let publish_run_id = run_id.clone();
            let publish_observer = run_event_observer.clone();
            let mut usage = usage_accumulators.remove(&run_id);
            let published = tokio::task::spawn_blocking(move || {
                let sequence = publish_live_run_after_incremental_with_observer(
                    &publish_settings,
                    &publish_hub,
                    &publish_run_id,
                    before_sequence,
                    &mut usage,
                    Some(publish_observer),
                );
                (sequence, usage)
            })
            .await
            .ok();
            let Some((published_through, usage)) = published else {
                continue;
            };
            // ponytail: retain one cursor per observed run until publisher shutdown.
            if let Some(sequence) = published_through {
                last_published_sequence.insert(run_id.clone(), sequence);
            }
            if let Some(usage) = usage {
                usage_accumulators.insert(run_id, usage);
            }
        }
    }
}

struct TriggerSourceLoop {
    cancellation: CancellationToken,
    handle: JoinHandle<()>,
}

impl TriggerSourceLoop {
    fn spawn(
        settings: Arc<SparkSettings>,
        live_hub: Arc<WorkspaceLiveHub>,
        run_event_observer: RunEventObserverHandle,
    ) -> Option<Arc<Self>> {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return None;
        };
        let cancellation = CancellationToken::new();
        let task_cancellation = cancellation.clone();
        let handle = handle.spawn(run_trigger_source_loop(
            settings,
            live_hub,
            task_cancellation,
            run_event_observer,
        ));
        Some(Arc::new(Self {
            cancellation,
            handle,
        }))
    }
}

impl Drop for TriggerSourceLoop {
    fn drop(&mut self) {
        self.cancellation.cancel();
        self.handle.abort();
    }
}

impl HttpAppState {
    fn with_trigger_source_loop(mut self) -> Self {
        self.trigger_source_loop = TriggerSourceLoop::spawn(
            self.settings.clone(),
            self.live_hub.clone(),
            self.run_event_observer.clone(),
        );
        self
    }
}

pub struct WorkspaceApiError(pub WorkspaceError);

impl IntoResponse for WorkspaceApiError {
    fn into_response(self) -> Response {
        let status =
            StatusCode::from_u16(self.0.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = match self.0.code() {
            Some(code) => json!({ "code": code, "detail": self.0.detail() }),
            None => json!({ "detail": self.0.detail() }),
        };
        (status, Json(body)).into_response()
    }
}

impl From<WorkspaceError> for WorkspaceApiError {
    fn from(value: WorkspaceError) -> Self {
        Self(value)
    }
}

async fn workspace_api_fallback() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "detail": "Not Found" })),
    )
}

async fn serve_index(State(settings): State<Arc<SparkSettings>>) -> Response {
    let Some(resource) = spark_assets::frontend::load_index(&settings) else {
        return json_detail(StatusCode::NOT_FOUND, "UI index not found");
    };
    serve_resource(resource, "text/html; charset=utf-8")
}

async fn serve_favicon(State(settings): State<Arc<SparkSettings>>) -> Response {
    let Some(resource) = spark_assets::frontend::load_favicon(&settings) else {
        return json_detail(StatusCode::NOT_FOUND, "Asset not found");
    };
    serve_resource(resource, "image/png")
}

async fn serve_asset(
    State(settings): State<Arc<SparkSettings>>,
    Path(asset_path): Path<String>,
) -> Response {
    match spark_assets::frontend::load_asset(&settings, &format!("assets/{asset_path}")) {
        Ok(Some(resource)) => {
            let content_type = content_type_for_asset(resource.logical_path());
            serve_resource(resource, content_type)
        }
        Ok(None) | Err(_) => json_detail(StatusCode::NOT_FOUND, "Asset not found"),
    }
}

fn serve_resource(resource: spark_assets::ResourceFile, content_type: &'static str) -> Response {
    let mut response = (StatusCode::OK, Bytes::copy_from_slice(resource.bytes())).into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
}

async fn product_fallback() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "detail": "Not Found" })),
    )
}

async fn redirect_workspace_mount(headers: HeaderMap, uri: Uri) -> Response {
    redirect_to_mount_slash(headers, &uri, "/workspace/")
}

async fn redirect_attractor_mount(headers: HeaderMap, uri: Uri) -> Response {
    redirect_to_mount_slash(headers, &uri, "/attractor/")
}

fn redirect_to_mount_slash(headers: HeaderMap, uri: &Uri, target_path: &'static str) -> Response {
    let mut target = target_path.to_string();
    if let Some(query) = uri.query() {
        target.push('?');
        target.push_str(query);
    }
    let location = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(|host| format!("http://{host}{target}"))
        .unwrap_or(target);

    let mut response = StatusCode::TEMPORARY_REDIRECT.into_response();
    if let Ok(location) = HeaderValue::from_str(&location) {
        response.headers_mut().insert(header::LOCATION, location);
    }
    response
}

fn json_detail(status: StatusCode, detail: &'static str) -> Response {
    (status, Json(json!({ "detail": detail }))).into_response()
}

fn content_type_for_asset(path: &str) -> &'static str {
    match std::path::Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
    {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("json") | Some("map") => "application/json",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    }
}

async fn attractor_dispatch(
    State(state): State<HttpAppState>,
    method: Method,
    uri: Uri,
    body: Bytes,
) -> Response {
    let settings = state.settings.clone();
    let live_hub = state.live_hub.clone();
    let path = uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or_else(|| uri.path());
    let path_run_id = is_mutating_method(&method)
        .then(|| pipeline_id_from_attractor_path(path))
        .flatten();
    let path_run_sequence = path_run_id
        .as_deref()
        .and_then(|run_id| latest_run_sequence(&settings, run_id).ok().flatten());
    let body = String::from_utf8_lossy(&body);
    let response = attractor_api::handle_attractor_request_with_options(
        method.as_str(),
        path,
        &body,
        (*settings).clone(),
        state.runtime_handler_runner_factory.clone(),
        state.run_event_observer.0.clone(),
    );
    if is_mutating_method(&method) && response.status_code < 400 {
        publish_attractor_live_updates(
            &settings,
            &live_hub,
            path_run_id.as_deref(),
            path_run_sequence,
            &response.body,
        );
    }
    let content_type = response.content_type.clone();
    let status =
        StatusCode::from_u16(response.status_code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response = if content_type.starts_with("application/json") {
        (status, Json(response.body)).into_response()
    } else {
        let body = response
            .body
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| response.body.to_string());
        (status, body).into_response()
    };
    if let Ok(content_type) = HeaderValue::from_str(&content_type) {
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, content_type);
    }
    if response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/event-stream"))
    {
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        response
            .headers_mut()
            .insert(header::CONNECTION, HeaderValue::from_static("keep-alive"));
    }
    response
}

/// Publishes orchestration envelopes and independently owned execution
/// transcript upserts. Returns the run journal's newest local sequence.
pub(crate) fn publish_live_run_after(
    settings: &SparkSettings,
    live_hub: &WorkspaceLiveHub,
    run_id: &str,
    before_sequence: Option<u64>,
) -> Option<u64> {
    let mut usage = None;
    publish_live_run_after_incremental(settings, live_hub, run_id, before_sequence, &mut usage)
}

fn publish_live_run_after_incremental(
    settings: &SparkSettings,
    live_hub: &WorkspaceLiveHub,
    run_id: &str,
    before_sequence: Option<u64>,
    usage: &mut Option<RunUsageAccumulator>,
) -> Option<u64> {
    publish_live_run_after_incremental_with_observer(
        settings,
        live_hub,
        run_id,
        before_sequence,
        usage,
        None,
    )
}

fn publish_live_run_after_incremental_with_observer(
    settings: &SparkSettings,
    live_hub: &WorkspaceLiveHub,
    run_id: &str,
    before_sequence: Option<u64>,
    usage: &mut Option<RunUsageAccumulator>,
    run_event_observer: Option<attractor_api::RunEventObserver>,
) -> Option<u64> {
    publish_live_run_after_incremental_with(
        settings,
        live_hub,
        run_id,
        before_sequence,
        usage,
        run_event_observer,
        || full_run_usage_accumulator(settings, run_id).ok(),
    )
}

fn publish_live_run_after_incremental_with(
    settings: &SparkSettings,
    live_hub: &WorkspaceLiveHub,
    run_id: &str,
    before_sequence: Option<u64>,
    usage: &mut Option<RunUsageAccumulator>,
    run_event_observer: Option<attractor_api::RunEventObserver>,
    mut initialize_full: impl FnMut() -> Option<RunUsageAccumulator>,
) -> Option<u64> {
    let publication = run_live_publication(settings, run_id, before_sequence)
        .ok()
        .flatten();
    let (new_run_envelopes, latest_sequence) = match publication {
        Some(publication) => {
            let mut initialized_from_full_projection = false;
            if usage.is_none() {
                *usage = if publication.starts_at_sequence_zero {
                    Some(RunUsageAccumulator::new(&publication.fallback_model))
                } else {
                    initialized_from_full_projection = true;
                    initialize_full()
                };
            }
            if !initialized_from_full_projection {
                if let Some(usage) = usage.as_mut() {
                    usage.apply(&publication.newly_read_entries);
                }
            }
            (publication.envelopes, publication.latest_sequence)
        }
        None => (Vec::new(), None),
    };
    for envelope in &new_run_envelopes {
        live_hub.publish(envelope.clone());
    }
    live_hub.publish_execution_transcripts(settings, run_id);
    for envelope in spark_workspace::live::node_thinking_envelopes(settings, run_id) {
        live_hub.publish(envelope);
    }
    *usage = full_run_usage_accumulator(settings, run_id).ok();
    let breakdown = usage.as_ref().and_then(|usage| usage.breakdown());
    if let Ok(Some(envelope)) = run_upsert_envelope_with_usage(settings, run_id, breakdown.as_ref())
    {
        live_hub.publish(envelope);
    }
    // Best-effort: workflow log projection must never fail the publish path.
    if let Ok(entries) =
        spark_workspace::project_run_milestones(settings, run_id, &new_run_envelopes)
    {
        for entry in &entries {
            live_hub.publish(spark_workspace::workflow_log_envelope(entry));
        }
    }
    publish_terminal_run_trigger_events(settings, live_hub, run_id, run_event_observer);
    if spark_workspace::live::evict_run_live_cache_if_terminal(settings, run_id) {
        live_hub.forget_run_thinking(run_id);
        *usage = None;
    }
    latest_sequence
}

#[cfg(test)]
mod live_queue_tests {
    use super::*;

    fn envelope(event_type: &str) -> LiveEnvelope {
        LiveEnvelope {
            event_type: event_type.to_string(),
            project_path: None,
            resource: spark_workspace::LiveResource {
                kind: "conversation".to_string(),
                id: Some("chat".to_string()),
            },
            cursor: None,
            payload: json!({}),
            reason: None,
        }
    }

    fn hub() -> (tempfile::TempDir, WorkspaceLiveHub) {
        let temp = tempfile::tempdir().unwrap();
        let store = spark_storage::ConversationStore::open(temp.path()).unwrap();
        let hub = WorkspaceLiveHub::new(&store);
        (temp, hub)
    }

    fn query(raw: spark_workspace::live::RawLiveQuery) -> spark_workspace::live::LiveQuery {
        spark_workspace::live::validate_live_query(raw).unwrap()
    }

    fn chat_thinking(conversation: &str, active: bool) -> LiveEnvelope {
        spark_workspace::live::conversation_event_envelope(
            conversation,
            "/p",
            json!({"delta_kind": "thinking", "turn_id": "t", "active": active}),
        )
    }

    fn node_thinking(run: &str, active: bool) -> LiveEnvelope {
        let mut envelope = envelope(spark_workspace::live::NODE_THINKING_EVENT_TYPE);
        envelope.resource.kind = "node_execution".to_string();
        envelope.resource.id = Some(format!("{run}:task:1:1"));
        envelope.payload = json!({"run_id": run, "active": active});
        envelope
    }

    /// A chat reopened mid-thought gets its thinking back, however silent, with
    /// its true start; thinking that ended while it was away does not return.
    #[test]
    fn subscribing_restores_only_the_thinking_still_in_progress() {
        let (_temp, hub) = hub();
        let chat = query(spark_workspace::live::RawLiveQuery {
            conversation_id: Some("chat".to_string()),
            conversation_project_path: Some("/p".to_string()),
            ..Default::default()
        });
        hub.publish(chat_thinking("chat", true));
        hub.publish(chat_thinking("other", true));
        std::thread::sleep(Duration::from_millis(5));
        hub.publish(chat_thinking("chat", true));

        let restored = hub.active_thinking_envelopes(&chat);
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].resource.id.as_deref(), Some("chat"));
        assert_eq!(restored[0].event_type, "conversation.stream_delta");
        assert!(restored[0].payload["elapsed_ms"].as_u64().unwrap() >= 5);

        hub.publish(chat_thinking("chat", false));
        assert!(hub.active_thinking_envelopes(&chat).is_empty());
    }

    /// The stream's opening frames carry the restored thinking, and no more than that.
    #[tokio::test]
    async fn the_live_stream_opens_with_the_thinking_still_in_progress() {
        use futures_util::StreamExt;
        let temp = tempfile::tempdir().unwrap();
        let settings = Arc::new(live_lag_settings(temp.path()));
        let store = spark_storage::ConversationStore::open(temp.path()).unwrap();
        let hub = Arc::new(WorkspaceLiveHub::new(&store));
        let opening = |store: Arc<spark_storage::ConversationStore>| {
            let (settings, hub) = (settings.clone(), hub.clone());
            async move {
                let response = workspace::workspace_live_events(
                    State(settings),
                    State(store),
                    State(hub),
                    Ok(axum::extract::Query(spark_workspace::live::RawLiveQuery {
                        conversation_id: Some("chat".to_string()),
                        conversation_project_path: Some("/p".to_string()),
                        ..Default::default()
                    })),
                )
                .await
                .map_err(|_| "live events failed")
                .unwrap();
                let mut body = response.into_body().into_data_stream();
                let mut frames = String::new();
                while let Ok(Some(Ok(frame))) =
                    tokio::time::timeout(Duration::from_millis(300), body.next()).await
                {
                    frames.push_str(&String::from_utf8_lossy(&frame));
                }
                frames
            }
        };

        hub.publish(chat_thinking("chat", true));
        let frames = opening(store.clone()).await;
        assert!(frames.contains("\"delta_kind\":\"thinking\""), "{frames}");
        assert!(frames.contains("elapsed_ms"), "{frames}");

        hub.publish(chat_thinking("chat", false));
        let frames = opening(store.clone()).await;
        assert!(!frames.contains("\"delta_kind\":\"thinking\""), "{frames}");
    }

    fn segment_frame(
        conversation: &str,
        project_path: &str,
        (turn, segment): (&str, &str),
        sequence: u64,
        text: &str,
    ) -> LiveEnvelope {
        spark_workspace::live::conversation_event_envelope(
            conversation,
            project_path,
            json!({"type": "stream_delta", "delta_kind": "segment_text",
                "conversation_id": conversation, "turn_id": turn, "segment_id": segment,
                "stream_sequence": sequence, "base_revision": 1, "text": text}),
        )
    }

    fn notice(
        closed: &[(&str, &str, &str)],
        settled: &[(&str, &str)],
    ) -> spark_storage::conversation::CommitNotice {
        spark_storage::conversation::CommitNotice {
            revision: 1,
            conversations: Vec::new(),
            closed_segments: closed
                .iter()
                .map(|(c, t, s)| (c.to_string(), t.to_string(), s.to_string()))
                .collect(),
            settled_executions: settled
                .iter()
                .map(|(c, t)| (c.to_string(), t.to_string()))
                .collect(),
        }
    }

    /// The hub keeps each open segment's latest frame for its query, in
    /// sequence order, until a commit ends the segment or settles or removes
    /// its execution. Other commits, a question-only return among them, keep it.
    #[test]
    fn open_segment_frames_replay_until_their_segment_ends_or_execution_settles() {
        let (_temp, hub) = hub();
        let chat = query(spark_workspace::live::RawLiveQuery {
            conversation_id: Some("chat".to_string()),
            conversation_project_path: Some("/p".to_string()),
            ..Default::default()
        });
        let texts = |hub: &WorkspaceLiveHub| -> Vec<String> {
            hub.segment_frame_envelopes(&chat)
                .iter()
                .map(|envelope| envelope.payload["text"].as_str().unwrap().to_string())
                .collect()
        };
        hub.publish(segment_frame("chat", "/p", ("t", "a"), 1, "He"));
        hub.publish(segment_frame("chat", "/p", ("t", "b"), 2, "Th"));
        hub.publish(segment_frame("chat", "/p", ("t", "a"), 3, "Hello"));
        hub.publish(segment_frame("chat", "/p", ("t", "a"), 1, "He"));
        hub.publish(segment_frame("other", "/p", ("t", "a"), 4, "Elsewhere"));
        assert_eq!(texts(&hub), ["Th", "Hello"]);

        hub.notify_commit(&notice(&[], &[]));
        hub.notify_commit(&notice(&[("other", "t", "b")], &[("other", "t")]));
        assert_eq!(texts(&hub), ["Th", "Hello"]);
        hub.notify_commit(&notice(&[("chat", "t", "a")], &[]));
        assert_eq!(texts(&hub), ["Th"]);
        hub.notify_commit(&notice(&[], &[("chat", "t")]));
        assert!(texts(&hub).is_empty());
    }

    /// A quiet open segment's text reaches a new stream right after its
    /// committed rows, and a stream that lagged again after its catch-up,
    /// with no further provider delta.
    #[tokio::test]
    async fn new_and_lagged_streams_replay_quiet_open_text_after_their_rows() {
        use futures_util::StreamExt;
        let temp = tempfile::tempdir().unwrap();
        let settings = Arc::new(live_lag_settings(temp.path()));
        let project = spark_storage::ProjectRegistry::new(temp.path())
            .ensure_project_paths(&temp.path().join("project").to_string_lossy())
            .unwrap()
            .project_path;
        let store = spark_storage::ConversationStore::open(temp.path()).unwrap();
        let hub = Arc::new(WorkspaceLiveHub::new(&store));
        let repository = spark_storage::ConversationRepository::new(store.clone());
        let (_, accepted) = repository
            .accept_message(
                "chat",
                &project,
                spark_storage::conversation::MessageInput {
                    admission: spark_storage::conversation::Admission::Start,
                    user_turn_id: "turn-user".to_string(),
                    content: "Hello".to_string(),
                    attachments: None,
                    client_request_sha256: None,
                    expected_settings_revision: 0,
                    settings: Default::default(),
                    held: false,
                    assistant_turn_id: "turn-assistant".to_string(),
                    execution_settings: json!({}),
                },
            )
            .unwrap();
        assert!(matches!(
            accepted,
            spark_storage::conversation::Accepted::Started { .. }
        ));
        let (segment, _) = repository
            .record_provider_event(
                "chat",
                &project,
                "turn-assistant",
                &serde_json::from_value(json!({"kind": "content_delta",
                    "channel": "assistant", "content_delta": "Quiet",
                    "source": {"item_id": "item-1"}}))
                .unwrap(),
                &Default::default(),
            )
            .unwrap()
            .opened
            .unwrap();
        hub.publish(segment_frame(
            "chat",
            &project,
            ("turn-assistant", &segment),
            1,
            "Quiet text",
        ));

        let response = workspace::workspace_live_events(
            State(settings),
            State(store),
            State(hub.clone()),
            Ok(axum::extract::Query(spark_workspace::live::RawLiveQuery {
                conversation_id: Some("chat".to_string()),
                conversation_project_path: Some(project.clone()),
                include_settings: Some("true".to_string()),
                ..Default::default()
            })),
        )
        .await
        .map_err(|_| "live events failed")
        .unwrap();
        let mut stream = response.into_body().into_data_stream();
        let mut frames = Vec::<Value>::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        let mut until = async |frames: &mut Vec<Value>, done: &dyn Fn(&Value) -> bool| loop {
            let frame = tokio::time::timeout_at(deadline, stream.next())
                .await
                .unwrap_or_else(|_| panic!("no such frame in {frames:?}"))
                .unwrap()
                .unwrap();
            let frame = String::from_utf8(frame.to_vec()).unwrap();
            let Some(data) = frame.strip_prefix("data: ") else {
                continue;
            };
            let frame: Value = serde_json::from_str(data.trim()).unwrap();
            frames.push(frame.clone());
            if done(&frame) {
                return frames.len() - 1;
            }
        };
        let replayed = |frame: &Value| frame["payload"]["text"] == "Quiet text";

        let replay = until(&mut frames, &replayed).await;
        let rows = frames
            .iter()
            .position(|frame| frame["resource"]["kind"] == "conversation")
            .unwrap();
        assert!(rows < replay, "{frames:?}");
        // The committed rows hold the open segment, not its text.
        assert!(
            !frames[rows].to_string().contains("Quiet"),
            "{}",
            frames[rows]
        );

        for _ in 0..=SUBSCRIBER_QUEUE_ENVELOPES {
            hub.publish_settings_change("workspace", "agents", None, json!("1"));
        }
        let resync = until(&mut frames, &|frame| frame["type"] == "resync_required").await;
        let replay = until(&mut frames, &replayed).await;
        assert!(resync < replay);
        assert_eq!(frames[replay]["payload"]["segment_id"], segment.as_str());
    }

    #[test]
    fn a_run_reconnect_restores_its_thinking_until_it_ends_or_the_run_finishes() {
        let (_temp, hub) = hub();
        let run = query(spark_workspace::live::RawLiveQuery {
            run_id: Some("run".to_string()),
            ..Default::default()
        });
        hub.publish(node_thinking("run", true));
        hub.publish(node_thinking("other", true));
        assert_eq!(hub.active_thinking_envelopes(&run).len(), 1);
        hub.publish(node_thinking("run", false));
        assert!(hub.active_thinking_envelopes(&run).is_empty());

        hub.publish(node_thinking("run", true));
        hub.forget_run_thinking("run");
        assert!(hub.active_thinking_envelopes(&run).is_empty());
    }

    /// One slow tab overflows alone: its queue drains and it is told to
    /// resynchronize, while a tab that keeps up loses nothing.
    #[test]
    fn a_slow_subscriber_overflows_alone() {
        let (_temp, hub) = hub();
        let slow = hub.subscribe();
        let fast = hub.subscribe();
        let total = SUBSCRIBER_QUEUE_ENVELOPES + 10;
        let mut fast_received = 0;
        for _ in 0..total {
            hub.publish(envelope("run.journal_entry"));
            while let Some(Received::Envelope(_)) = fast.try_recv() {
                fast_received += 1;
            }
        }
        assert_eq!(fast_received, total);
        let mut lagged = 0;
        let mut received = 0;
        while let Some(item) = slow.try_recv() {
            match item {
                Received::Lagged(count) => lagged += count,
                Received::Envelope(_) => received += 1,
                Received::Changed => {}
            }
        }
        assert!(lagged > 0);
        // The overflow signal comes first and no frame is both dropped and kept.
        assert_eq!(lagged as usize + received, total);
    }

    /// End to end through the live route: a stream whose queue overflowed
    /// loses no summary change. With commit notices silenced, only the lag
    /// itself can make it catch up, and it does so straight after its resync.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_lagged_stream_catches_up_on_every_summary_change() {
        use futures_util::StreamExt;
        use tower::ServiceExt;

        let temp = tempfile::tempdir().unwrap();
        // The Home project is HOME; nothing else in this binary reads it.
        std::env::set_var("HOME", temp.path());
        let settings = live_lag_settings(temp.path());
        let project = temp.path().join("project").to_string_lossy().into_owned();
        let backend =
            spark_workspace::WorkspaceConversationService::default_agent_turn_backend(&settings);
        let (app, hub) = build_app_with_live_hub(
            settings,
            attractor_api::default_runtime_handler_runner_factory(),
            backend,
        )
        .unwrap();
        let store = hub.conversation_store().unwrap();
        let repository = spark_storage::ConversationRepository::new(store.clone());
        let commit = |id: &str| repository.touch_conversation(id, &project).unwrap();
        commit("chat-before");
        store.set_commit_listener(Arc::new(|_| {}));

        let uri = format!(
            "/workspace/api/live/events?include_settings=true&include_conversations=true&conversations_project_path={}",
            project.replace('/', "%2F")
        );
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri(uri)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        // Unread: its initial state is taken, and its heartbeat has not started.
        let mut stream = response.into_body().into_data_stream();
        commit("chat-during-the-lag");
        for _ in 0..=SUBSCRIBER_QUEUE_ENVELOPES {
            hub.publish_settings_change("workspace", "agents", None, json!("1"));
        }

        let mut next_frame = async || -> Value {
            let frame = tokio::time::timeout(Duration::from_secs(120), stream.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let frame = String::from_utf8(frame.to_vec()).unwrap();
            let data = frame
                .strip_prefix("data: ")
                .unwrap_or_else(|| panic!("not an event: {frame}"));
            serde_json::from_str(data.trim()).unwrap()
        };
        let snapshot = next_frame().await;
        assert_eq!(snapshot["type"], "conversations.snapshot", "{snapshot}");
        let ids = |envelope: &Value| -> Vec<String> {
            envelope["payload"]["rows"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["id"].as_str().unwrap().to_string())
                .collect()
        };
        assert!(
            ids(&snapshot).contains(&"chat-before".to_string()),
            "{snapshot}"
        );
        assert!(
            !ids(&snapshot).contains(&"chat-during-the-lag".to_string()),
            "{snapshot}"
        );
        let resync = next_frame().await;
        assert_eq!(resync["type"], "resync_required", "{resync}");
        let changes = next_frame().await;
        assert_eq!(changes["type"], "conversations.changes", "{changes}");
        assert_eq!(ids(&changes), ["chat-during-the-lag"]);
    }

    fn live_lag_settings(root: &std::path::Path) -> SparkSettings {
        let data = root.join("data");
        std::fs::create_dir_all(root.join("project")).unwrap();
        SparkSettings {
            startup_sources: Default::default(),
            connections: Default::default(),
            providers: Default::default(),
            agents: Default::default(),
            project_root: root.to_path_buf(),
            data_dir: data.clone(),
            config_dir: data.join("config"),
            runtime_dir: data.join("runtime"),
            logs_dir: data.join("logs"),
            workspace_dir: data.join("workspace"),
            projects_dir: data.join("workspace/projects"),
            attractor_dir: data.join("attractor"),
            runs_dir: data.join("attractor/runs"),
            ui_dir: None,
            project_roots: vec![root.to_path_buf()],
        }
    }

    /// A full queue drops transient token frames before durable ones, and
    /// drops an incoming token frame rather than overflowing.
    #[test]
    fn transient_frames_are_dropped_first() {
        for transient in ["conversation.stream_delta", "run.thinking"] {
            let (_temp, hub) = hub();
            let subscriber = hub.subscribe();
            for _ in 0..SUBSCRIBER_QUEUE_ENVELOPES {
                hub.publish(envelope(transient));
            }
            hub.publish(envelope(transient));
            hub.publish(envelope("run.upsert"));
            let mut types = Vec::new();
            while let Some(item) = subscriber.try_recv() {
                match item {
                    Received::Envelope(envelope) => types.push(envelope.event_type),
                    other => panic!("unexpected {other:?}"),
                }
            }
            assert_eq!(types.len(), SUBSCRIBER_QUEUE_ENVELOPES);
            assert_eq!(types.last().map(String::as_str), Some("run.upsert"));
        }
    }

    /// Commit wakeups are out of band: they coalesce, survive a full queue,
    /// and reach only subscribers whose scopes the commit touched.
    #[test]
    fn commit_wakeups_coalesce_and_follow_scopes() {
        let (_temp, hub) = hub();
        let query = |raw: spark_workspace::live::RawLiveQuery| {
            Some(spark_workspace::live::validate_live_query(raw).unwrap())
        };
        let chat = hub.subscribe_with(query(spark_workspace::live::RawLiveQuery {
            conversation_id: Some("chat".into()),
            conversation_project_path: Some("/p".into()),
            include_runs_overview: Some("true".into()),
            ..Default::default()
        }));
        let other_project = hub.subscribe_with(query(spark_workspace::live::RawLiveQuery {
            include_conversations: Some("true".into()),
            conversations_project_path: Some("/q".into()),
            ..Default::default()
        }));
        for _ in 0..SUBSCRIBER_QUEUE_ENVELOPES + 5 {
            hub.publish(LiveEnvelope {
                resource: spark_workspace::LiveResource {
                    kind: "runs_overview".into(),
                    id: None,
                },
                ..envelope("run.upsert")
            });
        }
        for revision in 1..=3 {
            hub.notify_commit(&spark_storage::conversation::CommitNotice {
                revision,
                conversations: vec![("chat".into(), "/p".into())],
                ..Default::default()
            });
        }
        let mut changed = 0;
        let mut lagged = false;
        while let Some(item) = chat.try_recv() {
            match item {
                Received::Changed => changed += 1,
                Received::Lagged(_) => lagged = true,
                Received::Envelope(_) => {}
            }
        }
        assert!(lagged);
        assert_eq!(changed, 1);
        assert!(other_project.try_recv().is_none());
    }
}

#[cfg(test)]
mod incremental_usage_tests {
    use super::*;
    use attractor_core::{RawRuntimeEvent, RunRecord};
    use attractor_runtime::{CreateRunRequest, RunStore};
    use serde_json::json;
    use std::fs;

    fn test_settings(root: &std::path::Path) -> SparkSettings {
        let data = root.join("data");
        SparkSettings {
            startup_sources: Default::default(),
            connections: Default::default(),
            providers: Default::default(),
            agents: Default::default(),
            project_root: root.to_path_buf(),
            data_dir: data.clone(),
            config_dir: data.join("config"),
            runtime_dir: data.join("runtime"),
            logs_dir: data.join("logs"),
            workspace_dir: data.join("workspace"),
            projects_dir: data.join("workspace/projects"),
            attractor_dir: data.join("attractor"),
            runs_dir: data.join("attractor/runs"),
            ui_dir: None,
            project_roots: vec![root.to_path_buf()],
        }
    }

    fn event(run_id: &str, node: &str, event_type: &str, total: u64) -> RawRuntimeEvent {
        let mut event = RawRuntimeEvent::new("CodergenAdapter", run_id);
        event.payload = serde_json::from_value(json!({
            "node_id": node,
            "adapter_event_type": event_type,
            "payload": {"model": "gpt-5", "token_usage": {"input_tokens": total - 1, "output_tokens": 1}}
        })).expect("payload");
        event
    }

    fn upsert(receiver: &LiveSubscriber) -> Value {
        loop {
            if let Some(Received::Envelope(envelope)) = receiver.try_recv() {
                if envelope.event_type == "run.upsert" {
                    return envelope.payload["run"].clone();
                }
            } else {
                panic!("published envelope");
            }
        }
    }

    fn append_execution_usage(paths: &attractor_runtime::RunRootPaths, event: RawRuntimeEvent) {
        let root = paths.logs_dir().join("work/executions/1-0");
        fs::create_dir_all(&root).expect("execution root");
        fs::write(root.join("status.json"), r#"{"outcome":"running"}"#).expect("status");
        let event_type = event.payload["adapter_event_type"].clone();
        let mut payload = event.payload["payload"].clone();
        payload["node_id"] = event.payload["node_id"].clone();
        spark_storage::ActivityRepository::new(root)
            .append_event(json!({"event_type": event_type, "payload": payload}), "now")
            .expect("execution usage");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn failed_run_notifications_do_not_replay_history() {
        let temp = tempfile::tempdir().unwrap();
        let settings = Arc::new(test_settings(temp.path()));
        let store = RunStore::for_settings(&settings);
        let mut record = RunRecord::new("failed-live", temp.path().to_string_lossy().into_owned());
        record.status = "failed".into();
        let paths = store
            .create_run(CreateRunRequest {
                record,
                ..Default::default()
            })
            .unwrap();
        for _ in 0..300 {
            store
                .append_event(&paths, RawRuntimeEvent::new("Log", "failed-live"))
                .unwrap();
        }
        let hub = Arc::new(WorkspaceLiveHub::new(
            &spark_storage::ConversationStore::open(&settings.data_dir).unwrap(),
        ));
        let receiver = hub.subscribe();
        let (sender, notifications) = mpsc::unbounded_channel();
        let cancellation = CancellationToken::new();
        let task = tokio::spawn(run_event_publisher_loop(
            settings.clone(),
            hub,
            notifications,
            cancellation.clone(),
            Arc::new(|_| {}),
        ));
        // Count dropped queue entries too: the initial history exceeds the live queue.
        async fn batch(receiver: &LiveSubscriber) -> u64 {
            let mut count = 0;
            loop {
                match time::timeout(Duration::from_secs(5), receiver.recv())
                    .await
                    .unwrap()
                {
                    Received::Envelope(envelope) if envelope.event_type == "run.upsert" => {
                        return count
                    }
                    Received::Envelope(envelope) if envelope.event_type == "run.journal_entry" => {
                        count += 1
                    }
                    Received::Lagged(n) => count += n,
                    _ => {}
                }
            }
        }
        sender.send("failed-live".into()).unwrap();
        assert_eq!(
            batch(&receiver).await,
            latest_run_sequence(&settings, "failed-live")
                .unwrap()
                .unwrap()
        );
        for _ in 0..3 {
            sender.send("failed-live".into()).unwrap();
            assert_eq!(
                batch(&receiver).await,
                0,
                "terminal usage eviction must retain publication progress"
            );
        }
        let history = fs::read(paths.events_jsonl()).unwrap();
        // Publication errors and empty journals must not erase the previous cursor.
        for contents in [b"invalid json\n".as_slice(), b"".as_slice()] {
            fs::write(paths.events_jsonl(), contents).unwrap();
            assert!(run_live_publication(&settings, "failed-live", None)
                .ok()
                .flatten()
                .and_then(|publication| publication.latest_sequence)
                .is_none());
            sender.send("failed-live".into()).unwrap();
            assert_eq!(batch(&receiver).await, 0);
            fs::write(paths.events_jsonl(), &history).unwrap();
            sender.send("failed-live".into()).unwrap();
            assert_eq!(
                batch(&receiver).await,
                0,
                "publication failure must preserve cursor"
            );
        }
        store
            .append_event(&paths, RawRuntimeEvent::new("Log", "failed-live"))
            .unwrap();
        sender.send("failed-live".into()).unwrap();
        assert_eq!(batch(&receiver).await, 1);
        sender.send("failed-live".into()).unwrap();
        assert_eq!(batch(&receiver).await, 0);
        cancellation.cancel();
        task.await.unwrap();
    }

    #[test]
    fn real_publisher_overlays_only_new_usage_and_evicts_terminal_state() {
        let temp = tempfile::tempdir().expect("tempdir");
        let settings = test_settings(temp.path());
        let project = temp.path().join("project");
        fs::create_dir_all(&project).expect("project");
        let store = RunStore::for_settings(&settings);
        let mut record = RunRecord::new("live-usage", project.to_string_lossy().into_owned());
        record.model = "gpt-5".into();
        record.status = "running".into();
        let paths = store
            .create_run(CreateRunRequest {
                record,
                ..Default::default()
            })
            .expect("run");
        let hub = WorkspaceLiveHub::new(
            &spark_storage::ConversationStore::open(&settings.data_dir).unwrap(),
        );
        let receiver = hub.subscribe();
        let mut usage = None;

        append_execution_usage(
            &paths,
            event("live-usage", "work", "codex_app_server_session_event", 10),
        );
        let cursor =
            publish_live_run_after_incremental(&settings, &hub, "live-usage", None, &mut usage);
        let first = upsert(&receiver);
        assert_eq!(first["token_usage"], 10);
        assert_eq!(
            first["token_usage_breakdown"]["by_model"]["gpt-5"]["total_tokens"],
            10
        );
        assert_eq!(first["estimated_model_cost"]["status"], "estimated");
        assert_eq!(
            store.read_run_record(&paths).unwrap().unwrap().token_usage,
            None,
            "live overlay must not write run.json"
        );

        // Re-publishing the same cursor applies no entry twice.
        publish_live_run_after_incremental(&settings, &hub, "live-usage", cursor, &mut usage);
        assert_eq!(upsert(&receiver)["token_usage"], 10);
        append_execution_usage(
            &paths,
            event("live-usage", "work", "codex_app_server_session_event", 20),
        );
        let cursor =
            publish_live_run_after_incremental(&settings, &hub, "live-usage", cursor, &mut usage);
        assert_eq!(upsert(&receiver)["token_usage"], 20);

        // A fresh publisher (reconnect/restart) reconstructs the same full projection.
        let mut restarted = None;
        publish_live_run_after_incremental(&settings, &hub, "live-usage", None, &mut restarted);
        assert_eq!(upsert(&receiver)["token_usage"], 20);

        store
            .update_run_record("live-usage", |record| {
                record.status = "completed".into();
                record.token_usage = Some(20);
            })
            .expect("terminal");
        publish_live_run_after_incremental(&settings, &hub, "live-usage", cursor, &mut usage);
        assert!(usage.is_none(), "terminal publication evicts accumulator");
        assert_eq!(
            store.read_run_record(&paths).unwrap().unwrap().token_usage,
            Some(20)
        );
    }

    #[test]
    fn mid_journal_cursor_initializes_full_projection_once() {
        let temp = tempfile::tempdir().expect("tempdir");
        let settings = test_settings(temp.path());
        let project = temp.path().join("project");
        fs::create_dir_all(&project).expect("project");
        let store = RunStore::for_settings(&settings);
        let mut record = RunRecord::new("cursor-usage", project.to_string_lossy().into_owned());
        record.model = "gpt-5".into();
        record.status = "running".into();
        let paths = store
            .create_run(CreateRunRequest {
                record,
                ..Default::default()
            })
            .expect("run");
        append_execution_usage(
            &paths,
            event("cursor-usage", "work", "codex_app_server_session_event", 7),
        );
        for _ in 0..3 {
            let mut noise = RawRuntimeEvent::new("Log", "cursor-usage");
            noise.payload.insert("message".into(), json!("noise"));
            store.append_event(&paths, noise).expect("noise");
        }
        let hub = WorkspaceLiveHub::new(
            &spark_storage::ConversationStore::open(&settings.data_dir).unwrap(),
        );
        let mut usage = None;
        let mut initializations = 0;
        let mut initialize = || {
            initializations += 1;
            full_run_usage_accumulator(&settings, "cursor-usage").ok()
        };
        let cursor = publish_live_run_after_incremental_with(
            &settings,
            &hub,
            "cursor-usage",
            Some(1),
            &mut usage,
            None,
            &mut initialize,
        );
        publish_live_run_after_incremental_with(
            &settings,
            &hub,
            "cursor-usage",
            cursor,
            &mut usage,
            None,
            &mut initialize,
        );
        drop(initialize);
        assert_eq!(initializations, 1);
        assert_eq!(
            usage
                .and_then(|usage| usage.breakdown())
                .unwrap()
                .totals
                .total_tokens,
            7
        );
    }
}

async fn run_trigger_source_loop(
    settings: Arc<SparkSettings>,
    live_hub: Arc<WorkspaceLiveHub>,
    cancellation: CancellationToken,
    run_event_observer: RunEventObserverHandle,
) {
    let mut interval = time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => break,
            _ = interval.tick() => {
                let Some(store) = live_hub.conversation_store() else {
                    break;
                };
                let mut service = WorkspaceTriggerService::new((*settings).clone(), store);
                if let Some(observer) = &run_event_observer.0 {
                    service = service.with_run_event_observer(observer.clone());
                }
                if let Ok(outcomes) = service.process_due_trigger_sources().await {
                    publish_trigger_activation_outcomes(&settings, &live_hub, outcomes);
                }
            }
        }
    }
}

fn publish_terminal_run_trigger_events(
    settings: &SparkSettings,
    live_hub: &WorkspaceLiveHub,
    run_id: &str,
    run_event_observer: Option<attractor_api::RunEventObserver>,
) {
    let Some(store) = live_hub.conversation_store() else {
        return;
    };
    let mut service = WorkspaceTriggerService::new(settings.clone(), store.clone());
    if let Some(observer) = run_event_observer.clone() {
        service = service.with_run_event_observer(observer);
    }
    if let Ok(outcomes) = service.emit_terminal_flow_event_for_run(run_id) {
        publish_trigger_activation_outcomes(settings, live_hub, outcomes);
    }
    // Inbox delivery rides the observed publish path only, so every run a
    // conversation launches is itself observed and reports back.
    if run_event_observer.is_some() {
        let _ = spark_workspace::inbox::deliver_run_events(settings, &store, run_id);
    }
}

pub(crate) fn publish_trigger_activation_outcomes(
    settings: &SparkSettings,
    live_hub: &WorkspaceLiveHub,
    outcomes: Vec<spark_workspace::TriggerActivationOutcome>,
) {
    let mut run_ids = BTreeSet::new();
    for outcome in outcomes {
        if let Ok(value) = serde_json::to_value(&outcome.trigger) {
            live_hub.publish(trigger_upsert_envelope(&value));
        }
        if let Some(run_id) = outcome.run_id {
            run_ids.insert(run_id);
        }
    }
    for run_id in run_ids {
        publish_live_run_after(settings, live_hub, &run_id, None);
    }
}

fn publish_attractor_live_updates(
    settings: &SparkSettings,
    live_hub: &WorkspaceLiveHub,
    path_run_id: Option<&str>,
    path_run_sequence: Option<u64>,
    body: &Value,
) {
    let mut run_ids = BTreeSet::new();
    if let Some(run_id) = path_run_id {
        run_ids.insert(run_id.to_string());
    }
    collect_run_ids_from_value(body, &mut run_ids);
    for run_id in run_ids {
        let before_sequence = if Some(run_id.as_str()) == path_run_id {
            path_run_sequence
        } else {
            None
        };
        publish_live_run_after(settings, live_hub, &run_id, before_sequence);
    }
}

fn collect_run_ids_from_value(value: &Value, run_ids: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if matches!(
                    key.as_str(),
                    "run_id" | "pipeline_id" | "source_run_id" | "result_run_id" | "target_run_id"
                ) {
                    if let Some(run_id) = value.as_str().filter(|value| !value.trim().is_empty()) {
                        run_ids.insert(run_id.to_string());
                    }
                }
                collect_run_ids_from_value(value, run_ids);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_run_ids_from_value(value, run_ids);
            }
        }
        _ => {}
    }
}

fn pipeline_id_from_attractor_path(path: &str) -> Option<String> {
    let path = path.split('?').next().unwrap_or(path);
    let rest = path.strip_prefix("/attractor/pipelines/")?;
    let pipeline_id = rest.split('/').next().unwrap_or_default().trim();
    (!pipeline_id.is_empty()).then(|| pipeline_id.to_string())
}

fn is_mutating_method(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}
