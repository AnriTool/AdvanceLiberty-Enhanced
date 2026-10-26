//! Exact cartridge identity required by the installed native profile.
use sha2::{Digest, Sha256};
pub const USA: &str = "215c574d5afbda0aadbf89dc76b765cfaaeb35953eab63f7687d8bfe8a8e0f4d";
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn verify(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() == 16 * 1024 * 1024 && digest(bytes) == USA {
        Ok(())
    } else {
        Err(format!("Unsupported ROM. This build requires the unmodified USA cartridge dump.\nExpected SHA-256: {USA}\nProvided SHA-256: {}\nEuropean and patched ROMs are not compatible with this native profile.", digest(bytes)))
    }
}
