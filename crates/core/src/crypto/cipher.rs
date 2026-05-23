/// AEAD ciphertext: layout is `nonce(12) || ct(N) || tag(16)`.
///
/// Storage adapters write `as_bytes()` verbatim. `account_id` is bound at
/// construction time as AAD and must match at decrypt; it is **not** stored
/// in the ciphertext payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ciphertext(Vec<u8>);

impl Ciphertext {
    /// Returns the on-disk byte representation.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Wrap raw bytes read from storage. No validation here — invalid byte
    /// strings surface as `Error::DecryptionFailed` when the cipher attempts
    /// to decrypt them. `pub` because storage adapters (in separate crates)
    /// need to construct a `Ciphertext` from on-disk bytes before calling
    /// `CipherService::decrypt`.
    #[must_use]
    pub fn from_raw(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Internal constructor used by `CipherService::encrypt`.
    pub(crate) fn from_parts(nonce: [u8; 12], mut ct_and_tag: Vec<u8>) -> Self {
        let mut out = Vec::with_capacity(12 + ct_and_tag.len());
        out.extend_from_slice(&nonce);
        out.append(&mut ct_and_tag);
        Self(out)
    }

    pub(crate) fn nonce(&self) -> Option<&[u8; 12]> {
        if self.0.len() < 12 {
            return None;
        }
        <&[u8; 12]>::try_from(&self.0[..12]).ok()
    }

    pub(crate) fn ciphertext_and_tag(&self) -> &[u8] {
        if self.0.len() < 12 { &[] } else { &self.0[12..] }
    }
}
