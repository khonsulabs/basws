use lz4_flex::{compress_prepend_size, decompress_size_prepended};
use serde::{de::DeserializeOwned, Serialize};

pub fn compress<S: Serialize>(value: S) -> Vec<u8> {
    let serialized = serde_cbor::to_vec(&value).unwrap();
    compress_prepend_size(&serialized)
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cbor serialization error {0:?}")]
    Cbor(#[from] serde_cbor::Error),
    #[error("decompression error {0:?}")]
    Decompress(#[from] lz4_flex::block::DecompressError),
}

pub fn decompress<D: DeserializeOwned>(bytes: &[u8]) -> Result<D, Error> {
    let serialized = decompress_size_prepended(bytes)?;
    serde_cbor::from_slice(&serialized).map_err(Error::from)
}
