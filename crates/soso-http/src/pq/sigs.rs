//! Algoritmos de verificación webpki con ML-DSA-65 añadido.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use rustls::crypto::WebPkiSupportedAlgorithms;
use rustls::SignatureScheme;
use rustls_pki_types::SignatureVerificationAlgorithm;

use super::mldsa::ML_DSA_65;

/// Copia los algoritmos del provider base y antepone ML-DSA-65.
pub fn with_mldsa65(base: WebPkiSupportedAlgorithms) -> WebPkiSupportedAlgorithms {
    let mldsa_list: &'static [&'static dyn SignatureVerificationAlgorithm] =
        Box::leak(
            vec![&ML_DSA_65 as &dyn SignatureVerificationAlgorithm].into_boxed_slice(),
        );

    let mut all = Vec::with_capacity(1 + base.all.len());
    all.push(&ML_DSA_65 as &dyn SignatureVerificationAlgorithm);
    all.extend_from_slice(base.all);
    let all = all.leak();

    let mut mapping =
        Vec::<(SignatureScheme, &'static [&'static dyn SignatureVerificationAlgorithm])>::with_capacity(
            1 + base.mapping.len(),
        );
    mapping.push((SignatureScheme::ML_DSA_65, mldsa_list));
    for &(scheme, algs) in base.mapping {
        mapping.push((scheme, algs));
    }
    let mapping = mapping.leak();

    WebPkiSupportedAlgorithms { all, mapping }
}
