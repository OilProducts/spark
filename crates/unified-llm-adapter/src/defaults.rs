use std::sync::{OnceLock, RwLock};

use crate::client::Client;

static DEFAULT_CLIENT: OnceLock<RwLock<Option<Client>>> = OnceLock::new();

pub fn get_default_client() -> Client {
    let slot = default_client_slot();
    if let Some(client) = slot.read().expect("default client lock").clone() {
        return client;
    }

    let mut guard = slot.write().expect("default client lock");
    if let Some(client) = guard.clone() {
        return client;
    }

    let client = Client::from_env().expect(
        "environment-backed default client should be constructible without explicit default",
    );
    *guard = Some(client.clone());
    client
}

pub fn default_client() -> Client {
    get_default_client()
}

pub fn set_default_client(client: Option<Client>) {
    *default_client_slot().write().expect("default client lock") = client;
}

fn default_client_slot() -> &'static RwLock<Option<Client>> {
    DEFAULT_CLIENT.get_or_init(|| RwLock::new(None))
}
