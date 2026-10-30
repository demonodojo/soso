//! Criptografía post-cuántica para el cliente TLS (ML-KEM híbrido, ML-DSA-65).

mod kx;
mod mldsa;
mod sigs;
#[cfg(all(test, feature = "std"))]
mod tls_handshake;

use alloc::vec::Vec;

use rustls::crypto::CryptoProvider;

pub use kx::X25519MLKEM768;

/// Provider ring o rustcrypto con KEX híbrido y verificación ML-DSA-65.
pub fn enhanced_provider(base: CryptoProvider) -> CryptoProvider {
    let mut kx_groups = Vec::with_capacity(1 + base.kx_groups.len());
    kx_groups.push(X25519MLKEM768);
    kx_groups.extend(base.kx_groups);
    CryptoProvider {
        cipher_suites: base.cipher_suites,
        kx_groups,
        signature_verification_algorithms: sigs::with_mldsa65(base.signature_verification_algorithms),
        secure_random: base.secure_random,
        key_provider: base.key_provider,
    }
}
