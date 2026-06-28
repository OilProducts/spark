use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use crate::env::{ProviderConfig, ProviderEnvironment, PROVIDER_REGISTRATION_ORDER};
use crate::errors::{AdapterError, AdapterErrorKind};
use crate::events::StreamEvents;
use crate::http_transport::NativeHttpTransport;
use crate::middleware::{run_complete_chain, run_stream_chain, Middleware};
use crate::native::NativeProviderAdapter;
use crate::openai_compatible::{LiteLLMAdapter, OpenAICompatibleAdapter, OpenRouterAdapter};
use crate::profiles::{load_llm_profiles, LlmProfile, LlmProfileEnvironment};
use crate::request::{Request, Response};
use crate::resolution::ActiveLlmProfile;
use crate::timeouts::check_abort;

pub trait ProviderAdapter: Send + Sync {
    fn name(&self) -> &str;

    fn complete(&self, request: Request) -> Result<Response, AdapterError>;

    fn stream(&self, request: Request) -> Result<StreamEvents, AdapterError>;

    fn initialize(&self) -> Result<(), AdapterError> {
        Ok(())
    }

    fn close(&self) -> Result<(), AdapterError> {
        Ok(())
    }

    fn supports_tool_choice(&self, _mode: &str) -> bool {
        false
    }
}

#[derive(Clone, Default)]
pub struct Client {
    providers: BTreeMap<String, Arc<dyn ProviderAdapter>>,
    provider_order: Vec<String>,
    profile_routes: BTreeMap<String, ProfileRoute>,
    profile_order: Vec<String>,
    default_provider: Option<String>,
    middleware: Vec<Arc<dyn Middleware>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmProfileRoute {
    pub id: String,
    pub provider: String,
    pub default_model: Option<String>,
}

impl LlmProfileRoute {
    pub fn active_profile(&self) -> ActiveLlmProfile {
        ActiveLlmProfile::new(self.provider.clone(), self.default_model.clone())
    }
}

#[derive(Clone)]
struct ProfileRoute {
    profile: LlmProfileRoute,
    adapter: Result<Arc<dyn ProviderAdapter>, AdapterError>,
}

struct ResolvedProvider<'a> {
    provider_name: &'a str,
    adapter: &'a Arc<dyn ProviderAdapter>,
}

impl Client {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_adapters<I>(
        adapters: I,
        default_provider: Option<&str>,
    ) -> Result<Self, AdapterError>
    where
        I: IntoIterator<Item = Arc<dyn ProviderAdapter>>,
    {
        Self::from_provider_entries(
            adapters
                .into_iter()
                .map(|adapter| (adapter.name().to_string(), adapter)),
            default_provider,
        )
    }

    pub fn from_providers(
        providers: BTreeMap<String, Arc<dyn ProviderAdapter>>,
        default_provider: Option<&str>,
    ) -> Result<Self, AdapterError> {
        Self::from_provider_entries(providers, default_provider)
    }

    pub fn from_env() -> Result<Self, AdapterError> {
        Self::from_env_with_default(None)
    }

    pub fn from_env_with_default(default_provider: Option<&str>) -> Result<Self, AdapterError> {
        let env = std::env::vars().collect::<BTreeMap<_, _>>();
        Self::from_env_map(&env, default_provider)
    }

    pub fn from_env_map(
        env: &BTreeMap<String, String>,
        default_provider: Option<&str>,
    ) -> Result<Self, AdapterError> {
        let environment = ProviderEnvironment::from_env_map(env, default_provider);
        let mut providers = Vec::new();
        for provider in PROVIDER_REGISTRATION_ORDER {
            let Some(config) = environment.providers.get(provider) else {
                continue;
            };
            providers.push((provider.to_string(), configured_adapter(config)?));
        }

        Self::from_provider_entries(providers, environment.default_provider.as_deref())
    }

    pub fn from_env_and_profiles(
        config_dir: impl AsRef<Path>,
        default_provider: Option<&str>,
    ) -> Result<Self, AdapterError> {
        let env = std::env::vars().collect::<BTreeMap<_, _>>();
        Self::from_env_map_and_profiles(&env, config_dir, default_provider)
    }

    pub fn from_env_map_and_profiles(
        env: &BTreeMap<String, String>,
        config_dir: impl AsRef<Path>,
        default_provider: Option<&str>,
    ) -> Result<Self, AdapterError> {
        let environment = ProviderEnvironment::from_env_map(env, None);
        let mut providers = Vec::new();
        for provider in PROVIDER_REGISTRATION_ORDER {
            let Some(config) = environment.providers.get(provider) else {
                continue;
            };
            providers.push((provider.to_string(), configured_adapter(config)?));
        }

        let mut client = Self::from_provider_entries(providers, None)?;
        client.add_llm_profiles(load_llm_profiles(config_dir)?, env)?;
        let default_provider = default_provider
            .map(normalize_provider_name)
            .or(environment.default_provider);
        client.set_default_provider_name(default_provider.as_deref())?;
        Ok(client)
    }

    pub fn with_llm_profile_adapter(
        mut self,
        profile_id: impl AsRef<str>,
        active_profile: ActiveLlmProfile,
        adapter: Arc<dyn ProviderAdapter>,
    ) -> Result<Self, AdapterError> {
        self.insert_llm_profile_route(
            profile_id.as_ref(),
            active_profile.provider,
            active_profile.default_model,
            Ok(adapter),
        )?;
        Ok(self)
    }

    fn from_provider_entries<I>(
        providers: I,
        default_provider: Option<&str>,
    ) -> Result<Self, AdapterError>
    where
        I: IntoIterator<Item = (String, Arc<dyn ProviderAdapter>)>,
    {
        let mut normalized_providers = BTreeMap::new();
        let mut provider_order = Vec::new();
        for (name, adapter) in providers {
            let normalized_name = normalize_provider_name(&name);
            if normalized_name.is_empty() {
                return Err(configuration_error("Provider name must not be empty"));
            }
            if normalized_providers.contains_key(&normalized_name) {
                return Err(configuration_error(format!(
                    "Duplicate provider {normalized_name:?}"
                )));
            }
            provider_order.push(normalized_name.clone());
            normalized_providers.insert(normalized_name, adapter);
        }
        let providers = normalized_providers;
        for adapter in providers.values() {
            adapter.initialize()?;
        }

        let mut client = Self {
            providers,
            provider_order,
            profile_routes: BTreeMap::new(),
            profile_order: Vec::new(),
            default_provider: None,
            middleware: Vec::new(),
        };
        client.set_default_provider_name(default_provider)?;
        Ok(client)
    }

    pub fn with_middleware<I>(mut self, middleware: I) -> Self
    where
        I: IntoIterator<Item = Arc<dyn Middleware>>,
    {
        self.middleware.extend(middleware);
        self
    }

    pub fn add_middleware(&mut self, middleware: Arc<dyn Middleware>) {
        self.middleware.push(middleware);
    }

    pub fn default_provider(&self) -> Option<&str> {
        self.default_provider.as_deref()
    }

    pub fn with_default_provider(
        mut self,
        default_provider: Option<&str>,
    ) -> Result<Self, AdapterError> {
        self.set_default_provider_name(default_provider)?;
        Ok(self)
    }

    pub fn provider_names(&self) -> impl Iterator<Item = &str> {
        self.provider_order.iter().map(String::as_str)
    }

    pub fn llm_profile(&self, profile_id: &str) -> Option<LlmProfileRoute> {
        self.profile_routes
            .get(&normalize_provider_name(profile_id))
            .map(|route| route.profile.clone())
    }

    pub fn require_llm_profile(&self, profile_id: &str) -> Result<LlmProfileRoute, AdapterError> {
        let normalized_id = profile_id.trim();
        if normalized_id.is_empty() {
            return Err(configuration_error("LLM profile id is required."));
        }
        self.llm_profile(normalized_id).ok_or_else(|| {
            configuration_error(format!("LLM profile '{normalized_id}' was not found."))
        })
    }

    pub fn routed_provider_for_selector(&self, provider: &str) -> Option<String> {
        let normalized = normalize_provider_name(provider);
        if normalized.is_empty() {
            return None;
        }
        self.profile_routes
            .get(&normalized)
            .map(|route| route.profile.provider.clone())
            .or(Some(normalized))
    }

    pub fn complete(&self, request: Request) -> Result<Response, AdapterError> {
        check_abort(request.abort_signal.as_ref())?;
        request
            .validate_for_client()
            .map_err(invalid_request_error)?;
        let resolved = self.resolve_provider(request.provider.as_deref())?;
        let provider_name = resolved.provider_name.to_string();
        let adapter = Arc::clone(resolved.adapter);
        let mut request = request;
        request.provider = Some(provider_name.clone());
        run_complete_chain(request, &self.middleware, move |mut request| {
            check_abort(request.abort_signal.as_ref())?;
            request.provider = Some(provider_name.clone());
            request
                .validate_for_client()
                .map_err(invalid_request_error)?;
            adapter.complete(request)
        })
    }

    pub fn stream(&self, request: Request) -> Result<StreamEvents, AdapterError> {
        check_abort(request.abort_signal.as_ref())?;
        request
            .validate_for_client()
            .map_err(invalid_request_error)?;
        let resolved = self.resolve_provider(request.provider.as_deref())?;
        let provider_name = resolved.provider_name.to_string();
        let adapter = Arc::clone(resolved.adapter);
        let mut request = request;
        request.provider = Some(provider_name.clone());
        run_stream_chain(request, &self.middleware, move |mut request| {
            check_abort(request.abort_signal.as_ref())?;
            request.provider = Some(provider_name.clone());
            request
                .validate_for_client()
                .map_err(invalid_request_error)?;
            adapter.stream(request)
        })
    }

    pub fn supports_tool_choice(
        &self,
        mode: &str,
        provider: Option<&str>,
    ) -> Result<bool, AdapterError> {
        let resolved = self.resolve_provider(provider)?;
        Ok(resolved.adapter.supports_tool_choice(mode))
    }

    pub fn close(&self) -> Result<(), AdapterError> {
        let mut first_error = None;
        for provider_name in self.provider_order.iter().rev() {
            let Some(adapter) = self.providers.get(provider_name) else {
                continue;
            };
            if let Err(error) = adapter.close() {
                first_error.get_or_insert(error);
            }
        }
        for profile_id in self.profile_order.iter().rev() {
            let Some(route) = self.profile_routes.get(profile_id) else {
                continue;
            };
            if let Ok(adapter) = route.adapter.as_ref() {
                if let Err(error) = adapter.close() {
                    first_error.get_or_insert(error);
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn add_llm_profiles(
        &mut self,
        profiles: BTreeMap<String, LlmProfile>,
        env: &impl LlmProfileEnvironment,
    ) -> Result<(), AdapterError> {
        for profile in profiles.values() {
            let adapter = configured_profile_adapter(profile, env);
            self.insert_llm_profile_route(
                &profile.id,
                profile.provider.clone(),
                profile.default_model.clone(),
                adapter,
            )?;
        }
        Ok(())
    }

    fn insert_llm_profile_route(
        &mut self,
        profile_id: &str,
        provider: String,
        default_model: Option<String>,
        adapter: Result<Arc<dyn ProviderAdapter>, AdapterError>,
    ) -> Result<(), AdapterError> {
        let profile_id = profile_id.trim();
        if profile_id.is_empty() {
            return Err(configuration_error("LLM profile id is required."));
        }
        let route_key = normalize_provider_name(profile_id);
        if self.providers.contains_key(&route_key) {
            return Err(configuration_error(format!(
                "LLM profile '{profile_id}' conflicts with a configured provider"
            )));
        }
        if self.profile_routes.contains_key(&route_key) {
            return Err(configuration_error(format!(
                "Duplicate LLM profile {profile_id:?}"
            )));
        }
        let provider = normalize_provider_name(&provider);
        if provider.is_empty() {
            return Err(configuration_error(format!(
                "LLM profile '{profile_id}' provider must not be empty"
            )));
        }
        if let Ok(adapter) = adapter.as_ref() {
            adapter.initialize()?;
        }
        self.profile_order.push(route_key.clone());
        self.profile_routes.insert(
            route_key,
            ProfileRoute {
                profile: LlmProfileRoute {
                    id: profile_id.to_string(),
                    provider,
                    default_model,
                },
                adapter,
            },
        );
        Ok(())
    }

    fn set_default_provider_name(
        &mut self,
        default_provider: Option<&str>,
    ) -> Result<(), AdapterError> {
        let default_provider = default_provider.map(normalize_provider_name);
        if let Some(provider) = default_provider.as_deref() {
            if !self.providers.contains_key(provider) && !self.profile_routes.contains_key(provider)
            {
                return Err(configuration_error(format!(
                    "Unknown default provider {provider:?}"
                )));
            }
        }
        self.default_provider = default_provider;
        Ok(())
    }

    fn resolve_provider(
        &self,
        request_provider: Option<&str>,
    ) -> Result<ResolvedProvider<'_>, AdapterError> {
        let provider = request_provider
            .map(normalize_provider_name)
            .or_else(|| self.default_provider.clone())
            .ok_or_else(|| {
                configuration_error(
                    "No provider configured; set request.provider or Client.default_provider",
                )
            })?;

        if let Some(route) = self.profile_routes.get(&provider) {
            let adapter = route.adapter.as_ref().map_err(Clone::clone)?;
            return Ok(ResolvedProvider {
                provider_name: route.profile.provider.as_str(),
                adapter,
            });
        }

        self.providers
            .get_key_value(&provider)
            .map(|(name, adapter)| ResolvedProvider {
                provider_name: name.as_str(),
                adapter,
            })
            .ok_or_else(|| configuration_error(format!("Unknown provider {provider:?}")))
    }
}

fn configured_profile_adapter(
    profile: &LlmProfile,
    env: &impl LlmProfileEnvironment,
) -> Result<Arc<dyn ProviderAdapter>, AdapterError> {
    match profile.provider.as_str() {
        "openai_compatible" => Ok(Arc::new(OpenAICompatibleAdapter::new(
            "openai_compatible",
            profile.openai_compatible_request_config_with_env(env)?,
            Arc::new(NativeHttpTransport::new()),
        )?)),
        other => Err(configuration_error(format!(
            "LLM profile '{}' has unsupported provider '{other}'",
            profile.id
        ))),
    }
}

fn configured_adapter(config: &ProviderConfig) -> Result<Arc<dyn ProviderAdapter>, AdapterError> {
    match config.provider.as_str() {
        "openai" | "anthropic" | "gemini" => Ok(Arc::new(NativeProviderAdapter::new(
            config.provider.as_str(),
            config,
            Arc::new(NativeHttpTransport::new()),
        )?)),
        "openrouter" => Ok(Arc::new(OpenRouterAdapter::new(
            config,
            Arc::new(NativeHttpTransport::new()),
        )?)),
        "litellm" => Ok(Arc::new(LiteLLMAdapter::new(
            config,
            Arc::new(NativeHttpTransport::new()),
        )?)),
        "openai_compatible" => Ok(Arc::new(OpenAICompatibleAdapter::new(
            "openai_compatible",
            config,
            Arc::new(NativeHttpTransport::new()),
        )?)),
        _ => Ok(Arc::new(ConfiguredProviderAdapter::new(config.clone()))),
    }
}

#[derive(Debug, Clone)]
struct ConfiguredProviderAdapter {
    config: ProviderConfig,
}

impl ConfiguredProviderAdapter {
    fn new(config: ProviderConfig) -> Self {
        Self { config }
    }

    fn unavailable_error(&self) -> AdapterError {
        AdapterError::provider(
            AdapterErrorKind::Configuration,
            format!(
                "Provider '{}' is configured from the environment, but no Rust provider adapter is registered",
                self.config.provider
            ),
            Some(self.config.provider.clone()),
        )
    }
}

impl ProviderAdapter for ConfiguredProviderAdapter {
    fn name(&self) -> &str {
        &self.config.provider
    }

    fn complete(&self, _request: Request) -> Result<Response, AdapterError> {
        Err(self.unavailable_error())
    }

    fn stream(&self, _request: Request) -> Result<StreamEvents, AdapterError> {
        Err(self.unavailable_error())
    }
}

fn normalize_provider_name(provider: &str) -> String {
    provider.trim().to_ascii_lowercase()
}

fn configuration_error(message: impl Into<String>) -> AdapterError {
    AdapterError::new(AdapterErrorKind::Configuration, message)
}

fn invalid_request_error(message: impl Into<String>) -> AdapterError {
    AdapterError::new(AdapterErrorKind::InvalidRequest, message)
}
