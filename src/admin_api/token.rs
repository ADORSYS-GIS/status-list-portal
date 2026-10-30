//! Admin tokens: short-lived JWTs that authenticate the portal to the admin API.
//!
//! The server finds the portal's public key in its registry file by the token's `kid`, and
//! takes the portal's scopes from the same entry, never from the token.

use std::fs;
use std::time::Duration;

use anyhow::Context;
use jsonwebtoken::{EncodingKey, Header, encode, get_current_timestamp};
use serde::Serialize;

use super::SigningAlgorithm;
use crate::config;

const TOKEN_TYPE: &str = "statuslist-admin+jwt";
/// Well within the server's limit on `exp - iat`, 300 seconds by default.
const TOKEN_LIFETIME: Duration = Duration::from_secs(60);

/// Signs a fresh admin token for each call.
pub(super) struct TokenSigner {
    header: Header,
    key: EncodingKey,
    principal: String,
    audience: String,
}

#[derive(Serialize)]
struct Claims<'a> {
    iss: &'a str,
    aud: &'a str,
    iat: u64,
    exp: u64,
}

impl TokenSigner {
    /// Loads the signing key and checks that it signs with the configured algorithm.
    pub(super) fn new(config: &config::AdminApi) -> anyhow::Result<Self> {
        let path = config.signing_key_file.display();
        let algorithm = config.algorithm;
        let pem = fs::read(&config.signing_key_file)
            .with_context(|| format!("failed to read the admin API signing key {path}"))?;
        let key = encoding_key(algorithm, &pem)
            .with_context(|| format!("{path} is not a PEM private key for {algorithm}"))?;
        let header = Header {
            typ: Some(TOKEN_TYPE.to_owned()),
            kid: Some(config.key_id.clone()),
            ..Header::new(algorithm.into())
        };
        let signer = Self {
            header,
            key,
            principal: config.principal.clone(),
            audience: config.audience.clone(),
        };
        // Some mismatches, such as a P-384 key for ES256, only show when signing.
        signer
            .sign()
            .with_context(|| format!("{path} cannot sign {algorithm} tokens"))?;
        Ok(signer)
    }

    pub(super) fn sign(&self) -> jsonwebtoken::errors::Result<String> {
        let iat = get_current_timestamp();
        let claims = Claims {
            iss: &self.principal,
            aud: &self.audience,
            iat,
            exp: iat + TOKEN_LIFETIME.as_secs(),
        };
        encode(&self.header, &claims, &self.key)
    }
}

/// Reads a PKCS#8 PEM private key for `algorithm`.
pub(super) fn encoding_key(
    algorithm: SigningAlgorithm,
    pem: &[u8],
) -> jsonwebtoken::errors::Result<EncodingKey> {
    match algorithm {
        SigningAlgorithm::Es256 | SigningAlgorithm::Es384 => EncodingKey::from_ec_pem(pem),
        SigningAlgorithm::EdDsa => EncodingKey::from_ed_pem(pem),
        SigningAlgorithm::Ps256 => EncodingKey::from_rsa_pem(pem),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use jsonwebtoken::jwk::Jwk;
    use jsonwebtoken::{DecodingKey, Validation, decode, decode_header};
    use serde_json::Value;
    use url::Url;

    use super::*;

    const ALGORITHMS: [SigningAlgorithm; 4] = [
        SigningAlgorithm::Es256,
        SigningAlgorithm::Es384,
        SigningAlgorithm::EdDsa,
        SigningAlgorithm::Ps256,
    ];

    /// The test key for `algorithm`, in `test_data/admin-keys`.
    fn key_file(algorithm: SigningAlgorithm) -> PathBuf {
        let name = match algorithm {
            SigningAlgorithm::Es256 => "es256.pem",
            SigningAlgorithm::Es384 => "es384.pem",
            SigningAlgorithm::EdDsa => "eddsa.pem",
            SigningAlgorithm::Ps256 => "ps256.pem",
        };
        [env!("CARGO_MANIFEST_DIR"), "test_data", "admin-keys", name]
            .iter()
            .collect()
    }

    fn admin_config(algorithm: SigningAlgorithm, signing_key_file: PathBuf) -> config::AdminApi {
        config::AdminApi {
            url: Url::parse("http://localhost:8002").unwrap(),
            principal: "portal".to_string(),
            audience: "status-list-admin".to_string(),
            signing_key_file,
            key_id: "portal-1".to_string(),
            algorithm,
        }
    }

    fn startup_error(config: &config::AdminApi) -> String {
        let error = TokenSigner::new(config)
            .err()
            .expect("the key was accepted");
        format!("{error:#}")
    }

    /// Checks `token` the way the server does, with the public half of `algorithm`'s test key.
    #[track_caller]
    fn verify(token: &str, algorithm: SigningAlgorithm) -> Value {
        let pem = fs::read(key_file(algorithm)).unwrap();
        let jwk = Jwk::from_encoding_key(&encoding_key(algorithm, &pem).unwrap(), algorithm.into())
            .unwrap();
        let mut validation = Validation::new(algorithm.into());
        validation.set_audience(&["status-list-admin"]);
        validation.set_issuer(&["portal"]);
        validation.set_required_spec_claims(&["iss", "aud", "iat", "exp"]);

        let header = decode_header(token).unwrap();
        assert_eq!(header.typ.as_deref(), Some(TOKEN_TYPE));
        assert_eq!(header.kid.as_deref(), Some("portal-1"));
        let public_key = DecodingKey::from_jwk(&jwk).unwrap();
        decode::<Value>(token, &public_key, &validation)
            .unwrap()
            .claims
    }

    #[test]
    fn tokens_last_a_minute() {
        let algorithm = SigningAlgorithm::Es256;
        let signer = TokenSigner::new(&admin_config(algorithm, key_file(algorithm))).unwrap();

        let claims = verify(&signer.sign().unwrap(), algorithm);

        let lifetime = claims["exp"].as_u64().unwrap() - claims["iat"].as_u64().unwrap();
        assert_eq!(lifetime, TOKEN_LIFETIME.as_secs());
    }

    #[test]
    fn every_algorithm_the_server_accepts_signs_tokens() {
        for algorithm in ALGORITHMS {
            let config = admin_config(algorithm, key_file(algorithm));

            let signer =
                TokenSigner::new(&config).unwrap_or_else(|error| panic!("{algorithm}: {error:#}"));

            verify(&signer.sign().unwrap(), algorithm);
        }
    }

    #[test]
    fn a_key_for_another_algorithm_is_refused() {
        for (algorithm, key) in [
            // Same key type, other curve: only signing tells them apart.
            (SigningAlgorithm::Es384, SigningAlgorithm::Es256),
            (SigningAlgorithm::Es256, SigningAlgorithm::EdDsa),
            (SigningAlgorithm::EdDsa, SigningAlgorithm::Ps256),
            (SigningAlgorithm::Ps256, SigningAlgorithm::Es256),
        ] {
            let error = startup_error(&admin_config(algorithm, key_file(key)));

            let file = key_file(key).display().to_string();
            assert!(error.contains(&file), "{algorithm} with {key}: {error}");
            assert!(error.contains(&algorithm.to_string()), "{error}");
        }
    }

    #[test]
    fn a_missing_key_file_is_named() {
        let config = admin_config(SigningAlgorithm::Es256, "/nowhere/key.pem".into());

        let error = startup_error(&config);

        assert!(error.contains("/nowhere/key.pem"), "{error}");
    }
}
