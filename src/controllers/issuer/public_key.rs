//! The public key a developer pastes, checked before it goes to the admin API.
//!
//! The server checks the key in depth (curve points, RSA modulus size). These checks catch
//! the common mistakes early, refuse private keys, and drop every member the admin API does
//! not read.

use serde_json::{Map, Value};

use crate::admin_api::{KeyType, PublicJwk, SigningAlgorithm};
use crate::util::errors::{self, BoxedAppError};

/// Members that only private and symmetric keys have (RFC 7518 §6).
const PRIVATE_MEMBERS: [&str; 8] = ["d", "p", "q", "dp", "dq", "qi", "oth", "k"];

const PRIVATE_KEY: &str = "This is a private key. The portal did not store it, but its \
                           private half has left your computer: make a new key pair and \
                           paste only its public key.";

/// What a public key for one algorithm looks like.
struct KeyShape {
    kty: KeyType,
    kty_name: &'static str,
    crv: Option<&'static str>,
    members: &'static [&'static str],
    description: &'static str,
}

fn shape(alg: SigningAlgorithm) -> KeyShape {
    match alg {
        SigningAlgorithm::Es256 => KeyShape {
            kty: KeyType::Ec,
            kty_name: "EC",
            crv: Some("P-256"),
            members: &["x", "y"],
            description: "an EC key on the P-256 curve, with \"x\" and \"y\"",
        },
        SigningAlgorithm::Es384 => KeyShape {
            kty: KeyType::Ec,
            kty_name: "EC",
            crv: Some("P-384"),
            members: &["x", "y"],
            description: "an EC key on the P-384 curve, with \"x\" and \"y\"",
        },
        SigningAlgorithm::EdDsa => KeyShape {
            kty: KeyType::Okp,
            kty_name: "OKP",
            crv: Some("Ed25519"),
            members: &["x"],
            description: "an OKP key on the Ed25519 curve, with \"x\"",
        },
        SigningAlgorithm::Ps256 => KeyShape {
            kty: KeyType::Rsa,
            kty_name: "RSA",
            crv: None,
            members: &["n", "e"],
            description: "an RSA key, with \"n\" and \"e\"",
        },
    }
}

/// Reads the public JSON Web Key of an `alg` key pair.
pub(super) fn parse(value: &Value, alg: SigningAlgorithm) -> Result<PublicJwk, BoxedAppError> {
    if holds_private_key(value) {
        return Err(errors::unprocessable(PRIVATE_KEY));
    }
    let jwk = value.as_object().ok_or_else(|| {
        errors::unprocessable("The public key must be a JSON Web Key: a JSON object.")
    })?;
    if jwk.get("use").is_some_and(|key_use| key_use != "sig") {
        return Err(errors::unprocessable("The key's \"use\" must be \"sig\"."));
    }
    if jwk
        .get("alg")
        .is_some_and(|key_alg| *key_alg != alg.to_string())
    {
        return Err(errors::unprocessable(format!(
            "The key's \"alg\" must be {alg}, the algorithm chosen for the issuer."
        )));
    }

    let shape = shape(alg);
    let fits = member(jwk, "kty") == Some(shape.kty_name)
        && member(jwk, "crv") == shape.crv
        && shape
            .members
            .iter()
            .all(|name| member(jwk, name).is_some_and(is_base64url));
    if !fits {
        return Err(errors::unprocessable(format!(
            "For {alg}, the public key must be {}.",
            shape.description
        )));
    }

    let kept = |name: &str| {
        shape
            .members
            .contains(&name)
            .then(|| member(jwk, name).map(str::to_owned))
            .flatten()
    };
    Ok(PublicJwk {
        kty: shape.kty,
        crv: shape.crv.map(str::to_owned),
        x: kept("x"),
        y: kept("y"),
        n: kept("n"),
        e: kept("e"),
    })
}

/// Whether `value` holds a private key: as a JSON Web Key, in a JWK Set, or in PEM.
fn holds_private_key(value: &Value) -> bool {
    match value {
        Value::String(text) => text.contains("PRIVATE KEY"),
        Value::Object(object) => {
            PRIVATE_MEMBERS
                .iter()
                .any(|name| object.contains_key(*name))
                || object
                    .get("keys")
                    .and_then(Value::as_array)
                    .is_some_and(|keys| keys.iter().any(holds_private_key))
        }
        _ => false,
    }
}

fn member<'a>(jwk: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    jwk.get(name).and_then(Value::as_str)
}

fn is_base64url(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

#[cfg(test)]
mod tests {
    use http::StatusCode;
    use serde_json::json;

    use super::*;

    fn es256_key() -> Value {
        json!({
            "kty": "EC",
            "crv": "P-256",
            "x": "GtkCNIXEFHQ8j_gwyoS0H9eZQGCI_Iaqm2wUymJvOfQ",
            "y": "D9yk2oGE-yZjX50UqD1j68vpRbEhM-WyuhAWrEPAcyI",
        })
    }

    #[track_caller]
    fn refuse(value: &Value, alg: SigningAlgorithm) {
        let error = parse(value, alg).expect_err("the key was accepted");
        assert_eq!(error.response().status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn keys_of_every_algorithm_are_read() {
        for (alg, key) in [
            (SigningAlgorithm::Es256, es256_key()),
            (
                SigningAlgorithm::Es384,
                json!({ "kty": "EC", "crv": "P-384", "x": "AAAA", "y": "BBBB" }),
            ),
            (
                SigningAlgorithm::EdDsa,
                json!({ "kty": "OKP", "crv": "Ed25519", "x": "AAAA" }),
            ),
            (
                SigningAlgorithm::Ps256,
                json!({ "kty": "RSA", "n": "AAAA", "e": "AQAB" }),
            ),
        ] {
            let jwk = parse(&key, alg).unwrap_or_else(|_| panic!("{alg} key refused"));

            assert_eq!(serde_json::to_value(jwk).unwrap(), key, "{alg}");
        }
    }

    #[test]
    fn members_the_admin_api_does_not_read_are_dropped() {
        let mut key = es256_key();
        key["kid"] = json!("my-key");
        key["alg"] = json!("ES256");
        key["use"] = json!("sig");
        key["x5c"] = json!(["MIIB"]);

        let jwk = parse(&key, SigningAlgorithm::Es256).unwrap();

        assert_eq!(serde_json::to_value(jwk).unwrap(), es256_key());
    }

    #[test]
    fn private_keys_are_refused() {
        for member in PRIVATE_MEMBERS {
            let mut key = es256_key();
            key[member] = json!("c2VjcmV0");

            refuse(&key, SigningAlgorithm::Es256);
        }
    }

    #[test]
    fn private_keys_in_other_forms_are_recognized() {
        let mut private_jwk = es256_key();
        private_jwk["d"] = json!("c2VjcmV0");
        for value in [
            json!("-----BEGIN PRIVATE KEY-----\nMIGHAgEA\n-----END PRIVATE KEY-----"),
            json!("-----BEGIN EC PRIVATE KEY-----\nMHcCAQEE\n-----END EC PRIVATE KEY-----"),
            json!({ "keys": [es256_key(), private_jwk] }),
        ] {
            assert!(holds_private_key(&value), "{value}");
            refuse(&value, SigningAlgorithm::Es256);
        }
        assert!(!holds_private_key(&es256_key()));
        assert!(!holds_private_key(&json!({ "keys": [es256_key()] })));
    }

    #[test]
    fn keys_for_another_algorithm_are_refused() {
        for (alg, key) in [
            (SigningAlgorithm::Es384, es256_key()),
            (
                SigningAlgorithm::Es256,
                json!({ "kty": "OKP", "crv": "Ed25519", "x": "AAAA" }),
            ),
            (SigningAlgorithm::Ps256, es256_key()),
            (
                SigningAlgorithm::Ps256,
                json!({ "kty": "RSA", "crv": "P-256", "n": "AAAA", "e": "AQAB" }),
            ),
        ] {
            refuse(&key, alg);
        }
    }

    #[test]
    fn incomplete_or_malformed_keys_are_refused() {
        for key in [
            json!("not an object"),
            json!({ "kty": "EC", "crv": "P-256", "x": "AAAA" }),
            json!({ "kty": "EC", "crv": "P-256", "x": "AAAA", "y": "" }),
            json!({ "kty": "EC", "crv": "P-256", "x": "AA+A", "y": "BBBB" }),
            json!({ "kty": "EC", "crv": "P-256", "x": 1, "y": "BBBB" }),
            json!({ "kty": "oct", "k": "c2VjcmV0" }),
        ] {
            refuse(&key, SigningAlgorithm::Es256);
        }
    }

    #[test]
    fn keys_naming_another_algorithm_are_refused() {
        let mut key = es256_key();
        key["alg"] = json!("ES384");

        refuse(&key, SigningAlgorithm::Es256);
    }

    #[test]
    fn keys_for_encryption_are_refused() {
        let mut key = es256_key();
        key["use"] = json!("enc");

        refuse(&key, SigningAlgorithm::Es256);
    }
}
