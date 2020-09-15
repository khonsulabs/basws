use crypto::mac::Mac;
use std::convert::TryInto;

pub fn nonce() -> [u8; 32] {
    rand::random()
}

pub fn compute_challenge(private_key: &[u8], nonce: &[u8]) -> [u8; 32] {
    let mut hasher = crypto::hmac::Hmac::new(crypto::sha2::Sha256::new(), private_key);
    hasher.input(nonce);
    let slice = hasher.result().code().to_vec().into_boxed_slice();
    let array: Box<[u8; 32]> = match slice.try_into() {
        Ok(array) => array,
        Err(_) => unreachable!(),
    };
    *array
}
