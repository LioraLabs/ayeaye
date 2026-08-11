//! Pure Web Push encryption and VAPID signing.

use aes_gcm::{Aes128Gcm, KeyInit, aead::Aead};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hkdf::Hkdf;
use p256::{
    PublicKey, SecretKey,
    ecdh::diffie_hellman,
    ecdsa::{Signature, SigningKey, signature::Signer},
};
use sha2::Sha256;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    InvalidKey,
    InvalidInput,
    InvalidEndpoint,
    InvalidExpiry,
    Crypto,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Vapid {
    pub token: String,
    pub authorization: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Encrypted {
    pub body: Vec<u8>,
    pub content_encoding: &'static str,
}

pub fn vapid(
    endpoint: &str,
    expires_at: u64,
    now: u64,
    subject: &str,
    private_key: &[u8; 32],
) -> Result<Vapid, Error> {
    let endpoint = endpoint
        .strip_prefix("https://")
        .ok_or(Error::InvalidEndpoint)?;
    let authority = endpoint
        .split(['/', '?', '#'])
        .next()
        .filter(|value| !value.is_empty() && !value.contains('@'))
        .ok_or(Error::InvalidEndpoint)?;
    if expires_at <= now || expires_at - now > 86_400 || subject.is_empty() {
        return Err(Error::InvalidExpiry);
    }
    let authority = authority.strip_suffix(":443").unwrap_or(authority);
    let audience = format!("https://{authority}");
    let header = URL_SAFE_NO_PAD.encode(r#"{"typ":"JWT","alg":"ES256"}"#);
    let claims = URL_SAFE_NO_PAD.encode(format!(
        "{{\"aud\":{},\"exp\":{expires_at},\"sub\":{}}}",
        crate::json::string(&audience),
        crate::json::string(subject)
    ));
    let signing_input = format!("{header}.{claims}");
    let signing_key = SigningKey::from_slice(private_key).map_err(|_| Error::InvalidKey)?;
    let signature: Signature = signing_key.sign(signing_input.as_bytes());
    let token = format!(
        "{signing_input}.{}",
        URL_SAFE_NO_PAD.encode(signature.to_bytes())
    );
    let public = URL_SAFE_NO_PAD.encode(signing_key.verifying_key().to_encoded_point(false));
    Ok(Vapid {
        authorization: format!("vapid t={token},k={public}"),
        token,
    })
}

pub fn encrypt(
    plaintext: &[u8],
    user_public: &[u8],
    auth_secret: &[u8],
    salt: &[u8; 16],
    server_private: &[u8; 32],
) -> Result<Encrypted, Error> {
    if auth_secret.len() != 16 || plaintext.len() > 3993 {
        return Err(Error::InvalidInput);
    }
    let user_public = PublicKey::from_sec1_bytes(user_public).map_err(|_| Error::InvalidKey)?;
    let server_private = SecretKey::from_slice(server_private).map_err(|_| Error::InvalidKey)?;
    let server_public = server_private.public_key().to_sec1_bytes();
    let user_public_bytes = user_public.to_sec1_bytes();
    let shared = diffie_hellman(server_private.to_nonzero_scalar(), user_public.as_affine());

    let mut key_info = b"WebPush: info\0".to_vec();
    key_info.extend_from_slice(&user_public_bytes);
    key_info.extend_from_slice(&server_public);
    let mut ikm = [0; 32];
    Hkdf::<Sha256>::new(Some(auth_secret), shared.raw_secret_bytes().as_ref())
        .expand(&key_info, &mut ikm)
        .map_err(|_| Error::Crypto)?;

    let hkdf = Hkdf::<Sha256>::new(Some(salt), &ikm);
    let mut key = [0; 16];
    hkdf.expand(b"Content-Encoding: aes128gcm\0", &mut key)
        .map_err(|_| Error::Crypto)?;
    let mut nonce = [0; 12];
    hkdf.expand(b"Content-Encoding: nonce\0", &mut nonce)
        .map_err(|_| Error::Crypto)?;

    let mut record = plaintext.to_vec();
    record.push(2);
    let nonce = aes_gcm::Nonce::from(nonce);
    let ciphertext = Aes128Gcm::new_from_slice(&key)
        .map_err(|_| Error::Crypto)?
        .encrypt(&nonce, record.as_slice())
        .map_err(|_| Error::Crypto)?;

    let mut body = Vec::with_capacity(86 + ciphertext.len());
    body.extend_from_slice(salt);
    body.extend_from_slice(&4096_u32.to_be_bytes());
    body.push(server_public.len() as u8);
    body.extend_from_slice(&server_public);
    body.extend_from_slice(&ciphertext);
    Ok(Encrypted {
        body,
        content_encoding: "aes128gcm",
    })
}

#[cfg(test)]
mod tests {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

    use p256::{
        SecretKey,
        ecdsa::{Signature, VerifyingKey, signature::Verifier},
    };

    use super::{Error, encrypt, vapid};

    fn b64(value: &str) -> Vec<u8> {
        URL_SAFE_NO_PAD
            .decode(value.replace(char::is_whitespace, ""))
            .unwrap()
    }

    // AYEAYE-82 — RFC 8291 section 5 and appendix A.
    #[test]
    fn encryption_matches_the_published_vector() {
        let salt: [u8; 16] = b64("DGv6ra1nlYgDCS1FRnbzlw").try_into().unwrap();
        let private: [u8; 32] = b64("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw")
            .try_into()
            .unwrap();
        let encrypted = encrypt(
            b"When I grow up, I want to be a watermelon",
            &b64("BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4"),
            &b64("BTBZMqHH6r4Tts7J_aSIgg"),
            &salt,
            &private,
        )
        .unwrap();

        assert_eq!(encrypted.content_encoding, "aes128gcm");
        assert_eq!(
            URL_SAFE_NO_PAD.encode(encrypted.body),
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN"
        );
    }

    // AYEAYE-82 — RFC 8292 sections 2 and 3.
    #[test]
    fn vapid_token_has_bound_claims_and_a_verifiable_es256_signature() {
        let private = [7; 32];
        let signed = vapid(
            "https://push.example.net/p/subscription",
            1_453_523_768,
            1_453_500_000,
            "mailto:push@example.com",
            &private,
        )
        .unwrap();
        let parts: Vec<_> = signed.token.split('.').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(
            String::from_utf8(b64(parts[0])).unwrap(),
            r#"{"typ":"JWT","alg":"ES256"}"#
        );
        assert_eq!(
            String::from_utf8(b64(parts[1])).unwrap(),
            r#"{"aud":"https://push.example.net","exp":1453523768,"sub":"mailto:push@example.com"}"#
        );

        let key = SecretKey::from_slice(&private).unwrap();
        let verifying = VerifyingKey::from(&key.public_key());
        let signature = Signature::from_slice(&b64(parts[2])).unwrap();
        verifying
            .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
            .unwrap();
        assert_eq!(
            signed.authorization,
            format!(
                "vapid t={},k={}",
                signed.token,
                URL_SAFE_NO_PAD.encode(key.public_key().to_sec1_bytes())
            )
        );
        assert_eq!(
            vapid(
                "https://push.example.net/p/subscription",
                1_453_586_401,
                1_453_500_000,
                "mailto:push@example.com",
                &private,
            ),
            Err(Error::InvalidExpiry)
        );
    }
}
