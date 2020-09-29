use sha2::{Digest, Sha256};
use std::convert::TryInto;

pub fn nonce() -> [u8; 32] {
    rand::random()
}

pub fn compute_challenge(private_key: &[u8], nonce: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(nonce);
    hasher.update(private_key);
    hasher.finalize().try_into().unwrap()
}
