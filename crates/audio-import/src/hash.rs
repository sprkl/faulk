//! The SHA-256 of a file, in a stream: the name of its blob (project model § 3.1).

use sha2::Digest;

/// A SHA-256 computed piece by piece, so a file of any size is hashed in bounded memory.
#[derive(Clone, Default)]
pub struct Sha256(sha2::Sha256);

impl Sha256 {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    /// The hash of everything given so far.
    pub fn finish(self) -> project::Hash {
        project::Hash(self.0.finalize().into())
    }
}
