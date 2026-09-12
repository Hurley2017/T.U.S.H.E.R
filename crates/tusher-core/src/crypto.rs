use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Computes a user-friendly 6-digit Short Authentication String (SAS)
/// verification code from a shared ephemeral pairing token and the two public keys.
pub fn calculate_sas_pin(ephemeral_token: &str, pubkey_a: &[u8], pubkey_b: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(ephemeral_token.as_bytes())
        .expect("HMAC can take key of any size");

    // Order public keys deterministically
    if pubkey_a <= pubkey_b {
        mac.update(pubkey_a);
        mac.update(pubkey_b);
    } else {
        mac.update(pubkey_b);
        mac.update(pubkey_a);
    }

    let result = mac.finalize().into_bytes();
    // Derive a 6-digit integer from the first 4 bytes
    let num = u32::from_be_bytes([result[0], result[1], result[2], result[3]]) % 1_000_000;
    format!("{:03} {:03}", num / 1000, num % 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sas_pin_deterministic() {
        let token = "test_pairing_secret_12345";
        let pk_a = [1u8; 32];
        let pk_b = [2u8; 32];

        let pin1 = calculate_sas_pin(token, &pk_a, &pk_b);
        let pin2 = calculate_sas_pin(token, &pk_b, &pk_a);

        assert_eq!(pin1, pin2, "SAS PIN must be symmetrical regardless of order");
        assert_eq!(pin1.len(), 7, "SAS PIN format must be 'XXX XXX'");
    }
}
