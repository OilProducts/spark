use spark_agent_adapter::acp::{self, AcpDiscovery};
use spark_common::agent_settings::{NativeAgentSettings, SessionConfig};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use spark_common::settings::SparkSettings;

use crate::errors::{WorkspaceError, WorkspaceResult};

static PROVIDER_MODELS_CACHE: Mutex<
    Option<unified_llm_adapter::model_discovery::ProviderModelCache>,
> = Mutex::new(None);
type NativeModels = Result<Vec<ChatModelMetadata>, String>;
/// A native harness's models and its setup status, as one check found them.
type NativeCheck = (NativeModels, &'static str);
static CODEX_MODELS_CACHE: Checks<(), NativeAgentSettings, NativeCheck> = Checks::new();
static CLAUDE_CODE_MODELS_CACHE: Checks<(), NativeAgentSettings, NativeCheck> = Checks::new();
/// ACP discoveries by work area and harness ID, for its launch generation
/// and implementation version.
static ACP_DISCOVERY_CACHE: Checks<(PathBuf, String), (String, &'static str), AcpDiscovery> =
    Checks::new();

/// Called with a provider, such as `codex` or `acp:<id>`, once a background
/// check of it finished and its result is kept.
pub type CheckFinished = Arc<dyn Fn(&str) + Send + Sync>;

/// Harness checks, one per harness: the last finished result for its
/// registration, failures included, and the check running for it, if any.
/// Checks run in the background, so a listing never waits on a harness
/// process, and no lock is held while one runs.
struct Checks<H, R, V>(Mutex<Vec<Check<H, R, V>>>);

struct Check<H, R, V> {
    harness: H,
    registration: R,
    result: Option<V>,
    /// The running check's number. A check whose number is gone when it
    /// finishes, because its entry was cleared or replaced, is discarded.
    running: Option<u64>,
}

impl<H, R, V> Checks<H, R, V>
where
    H: PartialEq + Send + 'static,
    R: PartialEq + Send + 'static,
    V: Clone + Send + 'static,
{
    const fn new() -> Self {
        Self(Mutex::new(Vec::new()))
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Check<H, R, V>>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `harness`'s last result for `registration`, or `None` while it has
    /// none. When no check is running and there is no result, or `refresh`
    /// (Check again) asks for a new one, starts `run` in the background and
    /// calls `finished` once its result replaces the last. A running check is
    /// shared, by Check again too.
    fn result(
        &'static self,
        harness: H,
        registration: R,
        refresh: bool,
        run: impl FnOnce() -> V + Send + 'static,
        finished: impl FnOnce() + Send + 'static,
    ) -> Option<V> {
        let mut checks = self.lock();
        // A changed registration drops the old one's result and check.
        checks.retain(|check| check.harness != harness || check.registration == registration);
        let index = match checks.iter().position(|check| check.harness == harness) {
            Some(index) => index,
            None => {
                checks.push(Check {
                    harness,
                    registration,
                    result: None,
                    running: None,
                });
                checks.len() - 1
            }
        };
        let check = &mut checks[index];
        if check.running.is_none() && (check.result.is_none() || refresh) {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let number = NEXT.fetch_add(1, Ordering::Relaxed);
            check.running = Some(number);
            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));
                let mut checks = self.lock();
                let Some(check) = checks
                    .iter_mut()
                    .find(|check| check.running == Some(number))
                else {
                    return;
                };
                check.running = None;
                // A check that panicked leaves the next listing to start another.
                if let Ok(result) = result {
                    check.result = Some(result);
                    drop(checks);
                    finished();
                }
            });
        }
        check.result.clone()
    }

    fn clear(&self) {
        self.lock().clear();
    }

    /// Forgets the checks `keep` rejects, discarding those still running.
    fn retain(&self, keep: impl Fn(&H, &R) -> bool) {
        self.lock()
            .retain(|check| keep(&check.harness, &check.registration));
    }
}

pub fn invalidate_model_discovery(section: &str) {
    match section {
        "providers" | "llm_profiles" => {
            if let Some(cache) = PROVIDER_MODELS_CACHE
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_mut()
            {
                cache.clear();
            }
        }
        "agents" => {
            CODEX_MODELS_CACHE.clear();
            CLAUDE_CODE_MODELS_CACHE.clear();
            ACP_DISCOVERY_CACHE.clear();
        }
        "codex" => CODEX_MODELS_CACHE.clear(),
        "claude_code" => CLAUDE_CODE_MODELS_CACHE.clear(),
        _ => {}
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatModelMetadata {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_profile: Option<String>,
    pub id: String,
    pub display: String,
    pub is_default: bool,
    pub supported_reasoning_efforts: Vec<String>,
    pub default_reasoning_effort: Option<String>,
    #[serde(default)]
    pub supported_thinking: Vec<String>,
    #[serde(default)]
    pub supported_reasoning_modes: Vec<String>,
    #[serde(default)]
    pub supported_reasoning_summaries: Vec<String>,

    #[serde(default)]
    pub reasoning_unverified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatModelProviderStatus {
    /// `available`, `unavailable`, or for a harness `checking` while its
    /// first check for its registration runs.
    pub status: String,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// For a harness without a discovery: `ready`, `not_installed`,
    /// `unreachable` or `misconfigured`. A native harness is `ready` once its
    /// program answers; its sign-in is reported by its own connection status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_status: Option<String>,
    /// What discovery found for an ACP harness.
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub harness: Option<AcpDiscovery>,
}

/// The chooser's models and provider statuses, with ACP harnesses probed in
/// `work_area`, where their work would run. Each harness shows its last
/// check, or `checking` while its first runs in the background; `finished`
/// hears when one ends. `refresh` checks the listed harnesses again, each
/// showing its last result until the new one is in.
pub fn chat_models(
    settings: &SparkSettings,
    work_area: &Path,
    refresh: bool,
    finished: &CheckFinished,
) -> WorkspaceResult<Value> {
    let native = native_configuration(settings)?;
    let codex = codex_chat_models(&native, refresh, finished);
    listing(settings, work_area, codex, refresh, finished)
}

/// [`chat_models`] with Codex's check given.
pub fn chat_models_with_codex_result(
    settings: &SparkSettings,
    work_area: &Path,
    codex_result: NativeModels,
) -> WorkspaceResult<Value> {
    let setup = native_setup(&codex_result, &native_configuration(settings)?.codex_binary);
    listing(
        settings,
        work_area,
        Some((codex_result, setup)),
        false,
        &(Arc::new(|_: &str| {}) as CheckFinished),
    )
}

/// A native harness's setup status as its check found it.
fn native_setup(result: &NativeModels, program: &Option<String>) -> &'static str {
    match result {
        Ok(_) => "ready",
        Err(_) if !acp::program_found(program.as_deref().unwrap_or_default(), None) => {
            "not_installed"
        }
        Err(_) => "unreachable",
    }
}

/// `None` checks are still running.
fn listing(
    settings: &SparkSettings,
    work_area: &Path,
    codex: Option<NativeCheck>,
    refresh: bool,
    finished: &CheckFinished,
) -> WorkspaceResult<Value> {
    let mut models = Vec::new();
    let mut providers = serde_json::Map::new();
    let mut add = |provider: &str,
                   label: Option<&str>,
                   result: Option<NativeModels>,
                   setup_status: Option<&str>,
                   harness: Option<AcpDiscovery>| {
        let (status, error) = match result {
            None => ("checking", None),
            Some(Ok(discovered)) => {
                models.extend(discovered);
                ("available", None)
            }
            Some(Err(error)) => ("unavailable", Some(error)),
        };
        let status = ChatModelProviderStatus {
            status: status.into(),
            error,
            label: label.map(str::to_string),
            setup_status: setup_status.map(str::to_string),
            harness,
        };
        providers.insert(provider.into(), serde_json::json!(status));
    };
    let native = native_configuration(settings)?;
    let claude = claude_code_chat_models(&native, refresh, finished);
    for (provider, label, check) in [
        ("codex", "Codex", codex),
        ("claude-code", "Claude Code", claude),
    ] {
        let (result, setup) = check.unzip();
        add(provider, Some(label), result, setup, None);
    }
    let configuration = spark_storage::settings::read_execution_configuration(
        &settings.config_dir,
        &spark_common::paths::ProcessEnvironment,
    )?;
    for (id, harness, discovery) in
        acp_discoveries(&configuration.agents, work_area, refresh, finished)
    {
        let provider = format!("acp:{id}");
        let Some(discovery) = discovery else {
            add(&provider, Some(&harness.name), None, None, None);
            continue;
        };
        let result = Some(match discovery.setup_status.as_str() {
            "ready" => Ok(acp_chat_models(&provider, &discovery.config_options)),
            _ => Err(discovery.error.clone().unwrap_or_default()),
        });
        add(
            &provider,
            Some(&harness.name),
            result,
            None,
            Some(discovery),
        );
    }
    for (id, misconfigured) in &configuration.misconfigured_acp_harnesses {
        add(
            &format!("acp:{id}"),
            Some(&misconfigured.name),
            Some(Err(misconfigured.error.clone())),
            Some("misconfigured"),
            None,
        );
    }
    let environment = unified_llm_adapter::ProviderEnvironment::from_env_map(
        &configuration
            .providers
            .execution_environment(&std::env::vars().collect()),
        None,
    );
    let mut cache = PROVIDER_MODELS_CACHE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    for provider in ["anthropic", "openai", "gemini", "openrouter", "litellm"] {
        if let Some(config) = environment.providers.get(provider) {
            add(
                provider,
                None,
                Some(
                    cache
                        .get_or_insert_with(Default::default)
                        .models(config, &unified_llm_adapter::NativeHttpTransport::new())
                        .map(|models| models.into_iter().map(unified_chat_model).collect()),
                ),
                None,
                None,
            );
        }
    }
    models.extend(configured_profile_chat_models(settings)?);
    Ok(serde_json::json!({
        "models": models,
        "provider_reasoning_efforts": unified_llm_adapter::ModelCatalog::development().provider_reasoning_efforts,
        "providers": providers,
    }))
}

/// Signs in to ACP harness `id` in `work_area` with `method_id`, one of the
/// methods its check offered, and reports what it then found: its
/// `setup_status` and `error`.
pub fn acp_sign_in(
    settings: &SparkSettings,
    work_area: &Path,
    id: &str,
    method_id: &str,
) -> WorkspaceResult<Value> {
    let configuration = spark_storage::settings::read_execution_configuration(
        &settings.config_dir,
        &spark_common::paths::ProcessEnvironment,
    )?;
    let harness =
        configuration.agents.acp_harnesses.get(id).ok_or_else(|| {
            WorkspaceError::NotFound(format!("ACP harness {id} is not registered."))
        })?;
    let discovery = acp::sign_in(
        id,
        harness,
        configuration.agents.environment_inheritance,
        work_area,
        method_id,
    );
    Ok(serde_json::json!({"setup_status": discovery.setup_status, "error": discovery.error}))
}

/// Each registered harness with its last discovery in `work_area`, or
/// `None` while its first is running.
fn acp_discoveries<'a>(
    agents: &'a SessionConfig,
    work_area: &Path,
    refresh: bool,
    finished: &CheckFinished,
) -> Vec<(
    &'a String,
    &'a spark_common::agent_settings::AcpHarness,
    Option<AcpDiscovery>,
)> {
    // One check per directory however it is named, such as Home through a
    // symlinked HOME, as sessions' acknowledged options are kept.
    let work_area = std::fs::canonicalize(work_area).unwrap_or_else(|_| work_area.to_path_buf());
    let generations = agents
        .acp_harnesses
        .iter()
        .map(|(id, harness)| {
            let generation = acp::launch_generation(harness, agents.environment_inheritance);
            (id, generation)
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    // Checks of a removed or changed registration go in every work area, so
    // none is kept or announced, and restoring it checks it afresh. Only the
    // listed work area is checked again.
    ACP_DISCOVERY_CACHE.retain(|(_, id), (generation, _)| generations.get(id) == Some(generation));
    agents
        .acp_harnesses
        .iter()
        .map(|(id, harness)| {
            let generation = &generations[id];
            let run = {
                let (id, harness, work_area) =
                    (id.clone(), harness.clone(), work_area.to_path_buf());
                let inheritance = agents.environment_inheritance;
                move || {
                    let mut discovery = acp::discover(&id, &harness, inheritance, &work_area);
                    if discovery.setup_status == "unreachable" && !acp_program_found(&harness) {
                        discovery.setup_status = "not_installed".into();
                    }
                    discovery
                }
            };
            let finished = {
                let (finished, provider) = (finished.clone(), format!("acp:{id}"));
                move || finished(&provider)
            };
            let discovery = ACP_DISCOVERY_CACHE
                .result(
                    (work_area.to_path_buf(), id.clone()),
                    (generation.clone(), env!("CARGO_PKG_VERSION")),
                    refresh,
                    run,
                    finished,
                )
                .map(|mut discovery| {
                    // A session's acknowledged state supersedes the probe's.
                    if let Some(options) = acp::acknowledged_options(&work_area, id, &generation)
                        .filter(|_| discovery.setup_status == "ready")
                    {
                        discovery.config_options = options;
                        discovery.modes = None;
                    }
                    discovery
                });
            (id, harness, discovery)
        })
        .collect()
}

fn acp_program_found(harness: &spark_common::agent_settings::AcpHarness) -> bool {
    acp::program_found(
        &harness.executable,
        harness.env.get("PATH").map(String::as_str),
    )
}

/// An ACP harness's models are its model selector's values, in order. With
/// no selector, one "Harness default" entry leaves the choice to the harness.
/// Efforts are its thought-level value IDs, exactly as offered.
fn acp_chat_models(provider: &str, options: &[Value]) -> Vec<ChatModelMetadata> {
    let thought_level = acp::select_option(options, "thought_level");
    let efforts = thought_level
        .map(|option| {
            acp::select_values(option)
                .into_iter()
                .filter_map(|value| value["value"].as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let default_effort = thought_level
        .and_then(|option| option["currentValue"].as_str())
        .map(str::to_string);
    let model = |id: &str, display: &str, is_default: bool| ChatModelMetadata {
        provider: provider.to_string(),
        llm_profile: None,
        id: id.to_string(),
        display: display.to_string(),
        is_default,
        supported_reasoning_efforts: efforts.clone(),
        default_reasoning_effort: default_effort.clone(),
        supported_thinking: vec![],
        supported_reasoning_modes: vec![],
        supported_reasoning_summaries: vec![],
        reasoning_unverified: false,
    };
    match acp::select_option(options, "model") {
        Some(selector) => acp::select_values(selector)
            .into_iter()
            .filter_map(|value| {
                let id = value["value"].as_str()?;
                let display = value["name"].as_str().unwrap_or(id);
                Some(model(id, display, selector["currentValue"] == id))
            })
            .collect(),
        None => vec![model("", "Harness default", true)],
    }
}

pub fn native_configuration(settings: &SparkSettings) -> WorkspaceResult<NativeAgentSettings> {
    let mut configuration = spark_storage::settings::read_execution_configuration(
        &settings.config_dir,
        &spark_common::paths::ProcessEnvironment,
    )?;
    configuration
        .agents
        .native
        .retain_startup_paths(&settings.agents.native);
    spark_agent_adapter::config::capture_native_binaries(&mut configuration.agents);
    Ok(configuration.agents.native)
}

/// A native harness's last model check, or `None` while its first runs.
fn native_chat_models(
    cache: &'static Checks<(), NativeAgentSettings, NativeCheck>,
    provider: &'static str,
    native: &NativeAgentSettings,
    program: fn(&NativeAgentSettings) -> &Option<String>,
    refresh: bool,
    finished: &CheckFinished,
    run: fn(&NativeAgentSettings) -> NativeModels,
) -> Option<NativeCheck> {
    let (captured, finished) = (native.clone(), finished.clone());
    cache.result(
        (),
        native.clone(),
        refresh,
        move || {
            let result = run(&captured);
            let setup = native_setup(&result, program(&captured));
            (result, setup)
        },
        move || finished(provider),
    )
}

fn claude_code_chat_models(
    native: &NativeAgentSettings,
    refresh: bool,
    finished: &CheckFinished,
) -> Option<NativeCheck> {
    // Only its own settings, so a change to Codex's leaves its check alone.
    let native = NativeAgentSettings {
        codex_binary: None,
        codex_runtime_root: None,
        codex_seed_dir: None,
        codex_jsonrpc_trace: None,
        ..native.clone()
    };
    native_chat_models(
        &CLAUDE_CODE_MODELS_CACHE,
        "claude-code",
        &native,
        |native| &native.claude_binary,
        refresh,
        finished,
        |native| {
            spark_agent_adapter::claude_code::list_available_claude_code_models_with_settings(Some(
                native,
            ))
            .map(claude_code_chat_models_from_metadata)
            .map_err(|error| error.message)
        },
    )
}

pub fn claude_code_chat_models_from_metadata(
    metadata: Vec<spark_agent_adapter::ClaudeCodeModelMetadata>,
) -> Vec<ChatModelMetadata> {
    metadata
        .into_iter()
        .map(|model| ChatModelMetadata {
            reasoning_unverified: false,
            llm_profile: None,
            provider: "claude-code".to_string(),
            // The blank id is the catalog's `default` pseudo-entry: no
            // --model flag, the CLI picks.
            is_default: model.id.is_empty(),
            id: model.id,
            display: model.display,
            // The adapter passes the chosen effort as `--effort`.
            supported_reasoning_efforts: model.supported_efforts,
            default_reasoning_effort: None,
            supported_thinking: vec![],
            supported_reasoning_modes: vec![],
            supported_reasoning_summaries: vec![],
        })
        .collect()
}

/// Codex models come from the local install itself (`model/list`), so the
/// chooser only offers what codex will actually serve.
fn codex_chat_models(
    native: &NativeAgentSettings,
    refresh: bool,
    finished: &CheckFinished,
) -> Option<NativeCheck> {
    // Only its own settings, so a change to Claude Code's leaves its check alone.
    let native = NativeAgentSettings {
        claude_binary: None,
        claude_config_dir: None,
        claude_permission_mode: None,
        ..native.clone()
    };
    native_chat_models(
        &CODEX_MODELS_CACHE,
        "codex",
        &native,
        |native| &native.codex_binary,
        refresh,
        finished,
        |native| {
            spark_agent_adapter::codex_app_server::list_available_codex_models_with_settings(Some(
                native,
            ))
            .map(codex_chat_models_from_metadata)
            .map_err(|error| format!("Codex model discovery failed: {error}"))
        },
    )
}

pub fn codex_chat_models_from_metadata(
    metadata: Vec<spark_agent_adapter::CodexModelMetadata>,
) -> Vec<ChatModelMetadata> {
    let has_default = metadata.iter().any(|model| model.is_default);
    metadata
        .into_iter()
        .enumerate()
        .map(|(index, model)| ChatModelMetadata {
            reasoning_unverified: false,
            llm_profile: None,
            provider: "codex".to_string(),
            display: model.display,
            is_default: model.is_default || (!has_default && index == 0),
            // Report only what Codex reports: no levels means Default only.
            supported_reasoning_efforts: model.supported_reasoning_efforts,
            default_reasoning_effort: model.default_reasoning_effort,
            supported_thinking: vec![],
            supported_reasoning_modes: vec![],
            supported_reasoning_summaries: vec!["auto".into(), "concise".into(), "detailed".into()],
            id: model.id,
        })
        .collect()
}

fn configured_profile_chat_models(
    settings: &SparkSettings,
) -> WorkspaceResult<Vec<ChatModelMetadata>> {
    let profiles = unified_llm_adapter::public_llm_profiles(&settings.config_dir)
        .map_err(|error| WorkspaceError::ServiceUnavailable(error.to_string()))?;
    let mut models = Vec::new();
    for profile in profiles {
        let Some(profile_id) = profile.get("id").and_then(Value::as_str) else {
            continue;
        };
        let provider = profile
            .get("provider")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("openai_compatible");
        let label = profile
            .get("label")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(profile_id);
        let default_model = profile.get("default_model").and_then(Value::as_str);
        let Some(profile_models) = profile.get("models").and_then(Value::as_array) else {
            continue;
        };
        for model in profile_models.iter().filter_map(Value::as_str) {
            models.push(ChatModelMetadata {
                reasoning_unverified: false,
                llm_profile: Some(profile_id.to_string()),
                provider: provider.to_string(),
                id: model.to_string(),
                display: format!("{label} / {model}"),
                is_default: default_model == Some(model),
                supported_reasoning_efforts: profile
                    .get("reasoning_efforts")
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok())
                    .unwrap_or_default(),
                default_reasoning_effort: None,
                supported_thinking: vec![],
                supported_reasoning_modes: vec![],
                supported_reasoning_summaries: vec![],
            });
        }
    }
    Ok(models)
}

fn unified_chat_model(
    discovered: unified_llm_adapter::model_discovery::DiscoveredModel,
) -> ChatModelMetadata {
    let model = discovered.info;
    ChatModelMetadata {
        reasoning_unverified: discovered.reasoning_unverified,
        llm_profile: None,
        provider: model.provider,
        id: model.id,
        display: model.display_name,
        is_default: false,
        supported_reasoning_efforts: model.reasoning_efforts,
        default_reasoning_effort: model.default_reasoning_effort,
        supported_thinking: model.supported_thinking,
        supported_reasoning_modes: model.supported_reasoning_modes,
        supported_reasoning_summaries: model.supported_reasoning_summaries,
    }
}
