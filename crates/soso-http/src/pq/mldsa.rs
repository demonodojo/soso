//! Verificación de certificados firmados con ML-DSA-65 (FIPS 204).

use ml_dsa::{
    EncodedSignature, EncodedVerifyingKey, MlDsa65, Signature, VerifyingKey,
};
use rustls_pki_types::{alg_id, AlgorithmIdentifier, InvalidSignature, SignatureVerificationAlgorithm};
use signature3::Verifier as _;

#[derive(Debug)]
pub(crate) struct MlDsa65Verification;

impl SignatureVerificationAlgorithm for MlDsa65Verification {
    fn public_key_alg_id(&self) -> AlgorithmIdentifier {
        alg_id::ML_DSA_65
    }

    fn signature_alg_id(&self) -> AlgorithmIdentifier {
        alg_id::ML_DSA_65
    }

    fn verify_signature(
        &self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), InvalidSignature> {
        const PK: usize = 1952;
        const SIG: usize = 3309;
        if public_key.len() != PK || signature.len() != SIG {
            return Err(InvalidSignature);
        }
        let mut enc_pk = EncodedVerifyingKey::<MlDsa65>::default();
        enc_pk.copy_from_slice(public_key);
        let vk = VerifyingKey::decode(&enc_pk);
        let mut enc_sig = EncodedSignature::<MlDsa65>::default();
        enc_sig.copy_from_slice(signature);
        let sig = Signature::<MlDsa65>::decode(&enc_sig).ok_or(InvalidSignature)?;
        vk.verify(message, &sig).map_err(|_| InvalidSignature)
    }

    fn fips(&self) -> bool {
        false
    }
}

pub(crate) static ML_DSA_65: MlDsa65Verification = MlDsa65Verification;

#[cfg(test)]
mod tests {
    use super::*;
    use ml_dsa::{KeyExport, Keypair, MlDsa65, Seed, SigningKey};
    use signature3::Signer as _;

    #[test]
    fn verify_accepts_valid_signature() {
        let mut seed = Seed::default();
        getrandom::getrandom(seed.as_mut()).expect("rng");
        let sk = SigningKey::<MlDsa65>::from_seed(&seed);
        let msg = b"cert-tbs";
        let sig = sk.try_sign(msg).expect("sign");
        let enc = sk.verifying_key().to_bytes();
        ML_DSA_65
            .verify_signature(enc.as_slice(), msg, sig.encode().as_slice())
            .expect("verify");
    }
}
