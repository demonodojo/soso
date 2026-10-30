//! Handshake TLS local cliente/servidor con el provider post-cuántico.

#[cfg(all(test, feature = "std"))]
mod tests {
    extern crate std;

    use alloc::sync::Arc;
    use alloc::vec;
    use alloc::vec::Vec;
    use core::time::Duration;
    use std::io::Cursor;

    use rcgen::generate_simple_self_signed;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
    use rustls::time_provider::TimeProvider;
    use rustls::{
        ClientConfig, ClientConnection, NamedGroup, RootCertStore, ServerConfig,
        ServerConnection,
    };

    use super::super::enhanced_provider;

    #[derive(Debug)]
    struct TestTime;

    impl TimeProvider for TestTime {
        fn current_time(&self) -> Option<UnixTime> {
            Some(UnixTime::since_unix_epoch(Duration::from_secs(1_700_000_000)))
        }
    }

    fn pump(client: &mut ClientConnection, server: &mut ServerConnection) {
        for _ in 0..64 {
            client.process_new_packets().unwrap();
            server.process_new_packets().unwrap();

            let mut to_server = Vec::new();
            if client.wants_write() {
                client.write_tls(&mut to_server).unwrap();
            }
            if !to_server.is_empty() {
                let mut rd = Cursor::new(to_server);
                server.read_tls(&mut rd).unwrap();
            }

            let mut to_client = Vec::new();
            if server.wants_write() {
                server.write_tls(&mut to_client).unwrap();
            }
            if !to_client.is_empty() {
                let mut rd = Cursor::new(to_client);
                client.read_tls(&mut rd).unwrap();
            }

            if !client.is_handshaking() && !server.is_handshaking() {
                return;
            }
        }
        panic!("handshake no completó a tiempo");
    }

    #[test]
    fn negotiates_x25519_mlkem768_group() {
        #[cfg(feature = "crypto-ring")]
        let base = rustls::crypto::ring::default_provider();
        #[cfg(all(feature = "crypto-rust", not(feature = "crypto-ring")))]
        let base = rustls_rustcrypto::provider();

        let provider = Arc::new(enhanced_provider(base));
        let cert = generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let cert_der = CertificateDer::from(cert.cert);
        let key_der = PrivateKeyDer::try_from(cert.key_pair.serialize_der()).unwrap();

        let time = Arc::new(TestTime);
        let server_config = ServerConfig::builder_with_details(provider.clone(), time.clone())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert_der.clone()], key_der)
            .unwrap();

        let mut roots = RootCertStore::empty();
        roots.add(cert_der).unwrap();

        let client_config = ClientConfig::builder_with_details(provider, time)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();

        let sn = ServerName::try_from("localhost").unwrap();
        let mut client = ClientConnection::new(Arc::new(client_config), sn).unwrap();
        let mut server = ServerConnection::new(Arc::new(server_config)).unwrap();
        pump(&mut client, &mut server);

        let group = client
            .negotiated_key_exchange_group()
            .expect("grupo KX negociado");
        assert_eq!(group.name(), NamedGroup::X25519MLKEM768);
    }
}
