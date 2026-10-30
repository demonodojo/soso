//! Grupo TLS `X25519MLKEM768` (híbrido ML-KEM-768 + X25519).

use alloc::boxed::Box;
use alloc::vec::Vec;

use ml_kem::{
    kem::{Decapsulate, EncapsulationKey, KeyExport},
    B32, Ciphertext, DecapsulationKey, MlKem768, Seed,
};
use rustls::crypto::{
    ActiveKeyExchange, CompletedKeyExchange, SharedSecret, SupportedKxGroup,
};
use rustls::{Error, NamedGroup, PeerMisbehaved, ProtocolVersion};
use rand_core::{CryptoRng, RngCore};
use x25519_dalek::{EphemeralSecret, PublicKey};

struct GetrandomRng;

impl RngCore for GetrandomRng {
    fn next_u32(&mut self) -> u32 {
        let mut b = [0u8; 4];
        self.fill_bytes(&mut b);
        u32::from_le_bytes(b)
    }

    fn next_u64(&mut self) -> u64 {
        let mut b = [0u8; 8];
        self.fill_bytes(&mut b);
        u64::from_le_bytes(b)
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        getrandom::getrandom(dest).expect("RNG");
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

impl CryptoRng for GetrandomRng {}

const X25519_LEN: usize = 32;
const MLKEM768_PK: usize = 1184;
const MLKEM768_CT: usize = 1088;

fn fill_random(out: &mut [u8]) -> Result<(), Error> {
    getrandom::getrandom(out).map_err(|_| Error::FailedToGetRandomBytes)
}

#[derive(Clone, Copy, Debug)]
struct Layout {
    classical_share_len: usize,
    pq_client_share_len: usize,
    pq_server_share_len: usize,
    pq_first: bool,
}

impl Layout {
    const X25519_MLKEM768: Self = Self {
        classical_share_len: X25519_LEN,
        pq_client_share_len: MLKEM768_PK,
        pq_server_share_len: MLKEM768_CT,
        pq_first: true,
    };

    fn split<'a>(&self, share: &'a [u8], pq_len: usize) -> Option<(&'a [u8], &'a [u8])> {
        if share.len() != self.classical_share_len + pq_len {
            return None;
        }
        Some(if self.pq_first {
            let (pq, cl) = share.split_at(pq_len);
            (pq, cl)
        } else {
            let (cl, pq) = share.split_at(self.classical_share_len);
            (pq, cl)
        })
    }

    fn concat(&self, pq: &[u8], classical: &[u8]) -> Vec<u8> {
        if self.pq_first {
            [pq, classical].concat()
        } else {
            [classical, pq].concat()
        }
    }
}

#[derive(Debug)]
struct MlKem768Tls;

impl SupportedKxGroup for MlKem768Tls {
    fn start(&self) -> Result<Box<dyn ActiveKeyExchange>, Error> {
        let mut seed = Seed::default();
        fill_random(seed.as_mut())?;
        let dk = DecapsulationKey::<MlKem768>::from_seed(seed);
        let ek = dk.encapsulation_key();
        let pub_key = ek.to_bytes();
        Ok(Box::new(ActiveMlKem { dk, pub_key: pub_key.to_vec() }))
    }

    fn start_and_complete(&self, client_share: &[u8]) -> Result<CompletedKeyExchange, Error> {
        let key: ml_kem::kem::Key<EncapsulationKey<MlKem768>> = client_share
            .try_into()
            .map_err(|_| Error::from(PeerMisbehaved::InvalidKeyShare))?;
        let ek = EncapsulationKey::<MlKem768>::new(&key)
            .map_err(|_| Error::from(PeerMisbehaved::InvalidKeyShare))?;
        let mut m = B32::default();
        fill_random(m.as_mut())?;
        let (ct, ss) = ek.encapsulate_deterministic(&m);
        let ct_bytes: [u8; MLKEM768_CT] = ct.into();
        Ok(CompletedKeyExchange {
            group: NamedGroup::MLKEM768,
            pub_key: ct_bytes.to_vec(),
            secret: SharedSecret::from(ss.as_slice()),
        })
    }

    fn name(&self) -> NamedGroup {
        NamedGroup::MLKEM768
    }

    fn usable_for_version(&self, version: ProtocolVersion) -> bool {
        version == ProtocolVersion::TLSv1_3
    }
}

struct ActiveMlKem {
    dk: DecapsulationKey<MlKem768>,
    pub_key: Vec<u8>,
}

impl ActiveKeyExchange for ActiveMlKem {
    fn complete(self: Box<Self>, peer_pub_key: &[u8]) -> Result<SharedSecret, Error> {
        let ct: &Ciphertext<MlKem768> = peer_pub_key
            .try_into()
            .map_err(|_| Error::from(PeerMisbehaved::InvalidKeyShare))?;
        let ss = self.dk.decapsulate(ct);
        Ok(SharedSecret::from(ss.as_slice()))
    }

    fn pub_key(&self) -> &[u8] {
        &self.pub_key
    }

    fn group(&self) -> NamedGroup {
        NamedGroup::MLKEM768
    }
}

#[derive(Debug)]
struct X25519Tls;

impl SupportedKxGroup for X25519Tls {
    fn start(&self) -> Result<Box<dyn ActiveKeyExchange>, Error> {
        let secret = EphemeralSecret::random_from_rng(GetrandomRng);
        let pub_key = PublicKey::from(&secret);
        Ok(Box::new(ActiveX25519 {
            secret,
            pub_key: *pub_key.as_bytes(),
        }))
    }

    fn start_and_complete(&self, client_share: &[u8]) -> Result<CompletedKeyExchange, Error> {
        let mut arr = [0u8; 32];
        arr.copy_from_slice(
            client_share
                .get(..X25519_LEN)
                .ok_or(Error::from(PeerMisbehaved::InvalidKeyShare))?,
        );
        let their = PublicKey::from(arr);
        let ours = EphemeralSecret::random_from_rng(GetrandomRng);
        let pub_key = PublicKey::from(&ours);
        let secret = ours.diffie_hellman(&their);
        Ok(CompletedKeyExchange {
            group: NamedGroup::X25519,
            pub_key: pub_key.as_bytes().to_vec(),
            secret: SharedSecret::from(secret.as_bytes().as_slice()),
        })
    }

    fn name(&self) -> NamedGroup {
        NamedGroup::X25519
    }
}

struct ActiveX25519 {
    secret: EphemeralSecret,
    pub_key: [u8; 32],
}

impl ActiveKeyExchange for ActiveX25519 {
    fn complete(self: Box<Self>, peer_pub_key: &[u8]) -> Result<SharedSecret, Error> {
        let mut arr = [0u8; 32];
        arr.copy_from_slice(
            peer_pub_key
                .get(..X25519_LEN)
                .ok_or(Error::from(PeerMisbehaved::InvalidKeyShare))?,
        );
        let their = PublicKey::from(arr);
        let secret = self.secret.diffie_hellman(&their);
        Ok(SharedSecret::from(secret.as_bytes().as_slice()))
    }

    fn pub_key(&self) -> &[u8] {
        &self.pub_key
    }

    fn group(&self) -> NamedGroup {
        NamedGroup::X25519
    }
}

#[derive(Debug)]
struct Hybrid {
    classical: &'static dyn SupportedKxGroup,
    post_quantum: &'static dyn SupportedKxGroup,
    name: NamedGroup,
    layout: Layout,
}

impl SupportedKxGroup for Hybrid {
    fn start(&self) -> Result<Box<dyn ActiveKeyExchange>, Error> {
        let classical = self.classical.start()?;
        let post_quantum = self.post_quantum.start()?;
        let combined_pub_key = self.layout.concat(post_quantum.pub_key(), classical.pub_key());
        Ok(Box::new(ActiveHybrid {
            classical,
            post_quantum,
            name: self.name,
            layout: self.layout,
            combined_pub_key,
        }))
    }

    fn start_and_complete(&self, client_share: &[u8]) -> Result<CompletedKeyExchange, Error> {
        let (pq_share, cl_share) = self
            .layout
            .split(client_share, self.layout.pq_client_share_len)
            .ok_or(Error::from(PeerMisbehaved::InvalidKeyShare))?;
        let cl = self.classical.start_and_complete(cl_share)?;
        let pq = self.post_quantum.start_and_complete(pq_share)?;
        let combined_pub_key = self.layout.concat(&pq.pub_key, &cl.pub_key);
        let secret = self.layout.concat(
            pq.secret.secret_bytes(),
            cl.secret.secret_bytes(),
        );
        Ok(CompletedKeyExchange {
            group: self.name,
            pub_key: combined_pub_key,
            secret: SharedSecret::from(secret),
        })
    }

    fn name(&self) -> NamedGroup {
        self.name
    }

    fn usable_for_version(&self, version: ProtocolVersion) -> bool {
        version == ProtocolVersion::TLSv1_3
    }
}

struct ActiveHybrid {
    classical: Box<dyn ActiveKeyExchange>,
    post_quantum: Box<dyn ActiveKeyExchange>,
    name: NamedGroup,
    layout: Layout,
    combined_pub_key: Vec<u8>,
}

impl ActiveKeyExchange for ActiveHybrid {
    fn complete(self: Box<Self>, peer_pub_key: &[u8]) -> Result<SharedSecret, Error> {
        let (pq_share, cl_share) = self
            .layout
            .split(peer_pub_key, self.layout.pq_server_share_len)
            .ok_or(Error::from(PeerMisbehaved::InvalidKeyShare))?;
        let cl = self.classical.complete(cl_share)?;
        let pq = self.post_quantum.complete(pq_share)?;
        Ok(SharedSecret::from(
            self.layout
                .concat(pq.secret_bytes(), cl.secret_bytes()),
        ))
    }

    fn pub_key(&self) -> &[u8] {
        &self.combined_pub_key
    }

    fn group(&self) -> NamedGroup {
        self.name
    }
}

static MLKEM768: MlKem768Tls = MlKem768Tls;
static X25519: X25519Tls = X25519Tls;

static HYBRID_X25519_MLKEM768: Hybrid = Hybrid {
    classical: &X25519,
    post_quantum: &MLKEM768,
    name: NamedGroup::X25519MLKEM768,
    layout: Layout::X25519_MLKEM768,
};

/// Grupo híbrido preferido por rustls 0.23 (`0x11EC`).
pub static X25519MLKEM768: &dyn SupportedKxGroup = &HYBRID_X25519_MLKEM768;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hybrid_pub_key_len() {
        let kx = HYBRID_X25519_MLKEM768.start().expect("start");
        assert_eq!(kx.pub_key().len(), MLKEM768_PK + X25519_LEN);
        assert_eq!(kx.group(), NamedGroup::X25519MLKEM768);
    }
}
