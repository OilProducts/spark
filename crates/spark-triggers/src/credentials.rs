use rand::Rng;
use sha2::{Digest, Sha256};

const WEBHOOK_KEY_CHARS: usize = 16;
const WEBHOOK_SECRET_CHARS: usize = 32;
const TOKEN_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";

pub fn generate_webhook_credentials(webhook_key: Option<&str>) -> (String, String, String) {
    let key = webhook_key
        .map(str::to_string)
        .unwrap_or_else(|| token(WEBHOOK_KEY_CHARS));
    let secret = token(WEBHOOK_SECRET_CHARS);
    let secret_hash = sha256_hex(&secret);
    (key, secret, secret_hash)
}

pub fn verify_webhook_secret(secret_hash: &str, provided_secret: &str) -> bool {
    let expected = sha256_hex(provided_secret);
    constant_time_eq(secret_hash.trim().as_bytes(), expected.as_bytes())
}

pub fn sha256_hex(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn token(len: usize) -> String {
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| {
            let index = rng.gen_range(0..TOKEN_ALPHABET.len());
            TOKEN_ALPHABET[index] as char
        })
        .collect()
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right.iter())
        .fold(0u8, |acc, (left, right)| acc | (left ^ right))
        == 0
}
