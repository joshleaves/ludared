//! Hashing operations shared by codecs.

#[cfg(test)]
mod tests;

use sha2::Digest;

/// Returns the SHA-256 digest of `data` as raw bytes.
pub(crate) fn sha256(data: &[u8]) -> [u8; 32] {
  let mut hasher = sha2::Sha256::new();
  hasher.update(data);
  hasher.finalize().into()
}
