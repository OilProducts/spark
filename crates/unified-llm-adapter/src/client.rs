use std::collections::BTreeMap;
use std::sync::Arc;

use crate::env::{ProviderConfig, ProviderEnvironment, PROVIDER_REGISTRATION_ORDER};
use crate::errors::{AdapterError, AdapterErrorKind};
use crate::events::StreamEvents;
use crate::middleware::{run_complete_chain, run_stream_chain, Middleware};
use crate::request::{Request, Response};

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
    default_provider: Option<String>,
    middleware: Vec<Arc<dyn Middleware>>,
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
        let providers = PROVIDER_REGISTRATION_ORDER
            .into_iter()
            .filter_map(|provider| {
                environment.providers.get(provider).map(|config| {
                    (
                        provider.to_string(),
                        Arc::new(ConfiguredProviderAdapter::new(config.clone()))
                            as Arc<dyn ProviderAdapter>,
                    )
                })
            });

        Self::from_provider_entries(providers, environment.default_provider.as_deref())
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
        let default_provider = default_provider.map(normalize_provider_name);

        if let Some(provider) = default_provider.as_deref() {
            if !providers.contains_key(provider) {
                return Err(configuration_error(format!(
                    "Unknown default provider {provider:?}"
                )));
            }
        }

        for adapter in providers.values() {
            adapter.initialize()?;
        }

        Ok(Self {
            providers,
            provider_order,
            default_provider,
            middleware: Vec::new(),
        })
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

    pub fn provider_names(&self) -> impl Iterator<Item = &str> {
        self.provider_order.iter().map(String::as_str)
    }

    pub fn complete(&self, request: Request) -> Result<Response, AdapterError> {
        request
            .validate_for_client()
            .map_err(invalid_request_error)?;
        let (provider_name, adapter) = self.resolve_provider(request.provider.as_deref())?;
        let provider_name = provider_name.to_string();
        let adapter = Arc::clone(adapter);
        let mut request = request;
        request.provider = Some(provider_name.clone());
        run_complete_chain(request, &self.middleware, move |mut request| {
            request.provider = Some(provider_name.clone());
            adapter.complete(request)
        })
    }

    pub fn stream(&self, request: Request) -> Result<StreamEvents, AdapterError> {
        request
            .validate_for_client()
            .map_err(invalid_request_error)?;
        let (provider_name, adapter) = self.resolve_provider(request.provider.as_deref())?;
        let provider_name = provider_name.to_string();
        let adapter = Arc::clone(adapter);
        let mut request = request;
        request.provider = Some(provider_name.clone());
        run_stream_chain(request, &self.middleware, move |mut request| {
            request.provider = Some(provider_name.clone());
            adapter.stream(request)
        })
    }

    pub fn supports_tool_choice(
        &self,
        mode: &str,
        provider: Option<&str>,
    ) -> Result<bool, AdapterError> {
        let (_, adapter) = self.resolve_provider(provider)?;
        Ok(adapter.supports_tool_choice(mode))
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
        first_error.map_or(Ok(()), Err)
    }

    fn resolve_provider(
        &self,
        request_provider: Option<&str>,
    ) -> Result<(&str, &Arc<dyn ProviderAdapter>), AdapterError> {
        let provider = request_provider
            .map(normalize_provider_name)
            .or_else(|| self.default_provider.clone())
            .ok_or_else(|| {
                configuration_error(
                    "No provider configured; set request.provider or Client.default_provider",
                )
            })?;

        self.providers
            .get_key_value(&provider)
            .map(|(name, adapter)| (name.as_str(), adapter))
            .ok_or_else(|| configuration_error(format!("Unknown provider {provider:?}")))
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
