pub mod crypto;
pub mod identity;
pub mod protocol;
pub mod types;

pub use crypto::calculate_sas_pin;
pub use identity::{DeviceIdentity, NodeId, Platform};
pub use protocol::{Message, MessageCodec, PROTOCOL_VERSION};
pub use types::{TransportType, TrustStatus};

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;
    use tokio_util::codec::{Decoder, Encoder};

    #[test]
    fn test_identity_generation_and_signing() {
        let identity = DeviceIdentity::generate("TestNode".to_string());
        assert!(identity.node_id().as_str().starts_with("tshr_"));
        assert_eq!(identity.node_name(), "TestNode");

        let msg = b"Hello T.U.S.H.E.R mesh";
        let sig = identity.sign(msg);
        assert!(identity.verify(msg, &sig).is_ok());
    }

    #[test]
    fn test_protocol_codec_roundtrip() {
        let mut codec = MessageCodec::default();
        let mut buffer = BytesMut::new();

        let identity = DeviceIdentity::generate("NodeA".to_string());
        let original_msg = Message::Hello {
            version: PROTOCOL_VERSION,
            node_id: identity.node_id().clone(),
            node_name: identity.node_name().to_string(),
            platform: Platform::Windows,
            public_key_hex: identity.public_key_hex(),
            listen_port: 42424,
        };

        codec.encode(original_msg.clone(), &mut buffer).unwrap();
        let decoded = codec.decode(&mut buffer).unwrap().expect("should decode");
        assert_eq!(original_msg, decoded);
    }
}
