use crypto::mac::Mac;
use rand::distributions::Standard;
use rand::prelude::*;

pub fn nonce() -> Vec<u8> {
    let rng = thread_rng();
    rng.sample_iter(Standard).take(32).collect()
}

pub fn compute_challenge(private_key: &[u8], nonce: &[u8]) -> Vec<u8> {
    let mut hasher = crypto::hmac::Hmac::new(crypto::sha2::Sha256::new(), private_key);
    hasher.input(nonce);
    hasher.result().code().to_vec()
}
