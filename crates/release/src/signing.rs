//! Signed documents from the toolbox's publisher: the remote catalog and the aggregated release
//! feed (docs/architecture.md § 12). An envelope is one JSON object:
//!
//! ```json
//! {"schema": 1, "kind": "catalog", "signer": "ed25519:<public key, base64>",
//!  "signature": "<64 bytes, base64>", "payload": "<the document's text>"}
//! ```
//!
//! The signature is Ed25519 over `"artcraft-toolbox <kind> v1\n"` followed by the payload's
//! bytes, so a signed feed can never pass as a catalog. The toolbox pins one public key (the
//! built-in catalog's `[remote] public_key`); `signer` must be that key, so a document signed by
//! anyone else is refused before its payload is looked at.

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::{Error, Result, excerpt};

/// The envelope format this build reads.
pub const SCHEMA: u32 = 1;
/// Largest envelope accepted (the aggregated feed of a whole suite is a few MB).
pub const MAX_ENVELOPE_BYTES: usize = 16 * 1024 * 1024;
const PUBLIC_PREFIX: &str = "ed25519:";
const SECRET_PREFIX: &str = "ed25519-secret:";

/// What a document is; part of what is signed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Catalog,
    Feed,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Catalog => "catalog",
            Kind::Feed => "feed",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "catalog" => Some(Kind::Catalog),
            "feed" => Some(Kind::Feed),
            _ => None,
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An Ed25519 public key, written `ed25519:<base64>`.
#[derive(Clone, PartialEq, Eq)]
pub struct PublicKey(VerifyingKey);

impl PublicKey {
    pub fn parse(s: &str) -> Result<PublicKey> {
        let s = s.trim();
        let b64 = s.strip_prefix(PUBLIC_PREFIX).ok_or_else(|| Error::BadKey(format!("`{}` doesn't start with `{PUBLIC_PREFIX}`", excerpt(s, 40))))?;
        let bytes = B64.decode(b64).map_err(|_| Error::BadKey("the public key is not base64".into()))?;
        let arr: [u8; 32] = bytes.as_slice().try_into().map_err(|_| Error::BadKey(format!("the public key is {} bytes, not 32", bytes.len())))?;
        let key = VerifyingKey::from_bytes(&arr).map_err(|_| Error::BadKey("the public key is not a valid Ed25519 key".into()))?;
        Ok(PublicKey(key))
    }

    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.to_bytes()
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{PUBLIC_PREFIX}{}", B64.encode(self.0.as_bytes()))
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({self})")
    }
}

/// An Ed25519 signing key, written `ed25519-secret:<base64 seed>`. Lives in the publisher's CI
/// secret (`cargo xtask feed`), never in the toolbox.
#[derive(Clone)]
pub struct SecretKey(SigningKey);

impl SecretKey {
    pub fn from_seed(seed: [u8; 32]) -> SecretKey {
        SecretKey(SigningKey::from_bytes(&seed))
    }

    pub fn parse(s: &str) -> Result<SecretKey> {
        let s = s.trim();
        let b64 = s.strip_prefix(SECRET_PREFIX).ok_or_else(|| Error::BadKey(format!("the secret key doesn't start with `{SECRET_PREFIX}`")))?;
        let bytes = B64.decode(b64).map_err(|_| Error::BadKey("the secret key is not base64".into()))?;
        let seed: [u8; 32] = bytes.as_slice().try_into().map_err(|_| Error::BadKey(format!("the secret key is {} bytes, not 32", bytes.len())))?;
        Ok(SecretKey::from_seed(seed))
    }

    pub fn public(&self) -> PublicKey {
        PublicKey(self.0.verifying_key())
    }
}

impl fmt::Display for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{SECRET_PREFIX}{}", B64.encode(self.0.to_bytes()))
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretKey(for {})", self.public())
    }
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    schema: u32,
    kind: String,
    signer: String,
    signature: String,
    payload: String,
}

/// Sign `payload` as a `kind` document: the envelope's JSON.
pub fn sign(secret: &SecretKey, kind: Kind, payload: &str) -> String {
    let signature = secret.0.sign(&message(kind, payload));
    let envelope = Envelope {
        schema: SCHEMA,
        kind: kind.as_str().into(),
        signer: secret.public().to_string(),
        signature: B64.encode(signature.to_bytes()),
        payload: payload.into(),
    };
    // A struct of strings and a number always serializes.
    serde_json::to_string(&envelope).unwrap_or_default()
}

/// The payload of `json`, if it is a `kind` document signed by `key`.
pub fn verify(json: &str, key: &PublicKey, kind: Kind) -> Result<String> {
    if json.len() > MAX_ENVELOPE_BYTES {
        return Err(Error::TooLarge { what: "signed document", len: json.len(), limit: MAX_ENVELOPE_BYTES });
    }
    let raw: Envelope = serde_json::from_str(json).map_err(|e| Error::BadEnvelope(format!("not a signed document ({e})")))?;
    if raw.schema != SCHEMA {
        return Err(Error::BadEnvelope(format!("schema {} is not supported (this build reads schema {SCHEMA})", raw.schema)));
    }
    if raw.kind != kind.as_str() {
        return Err(Error::BadEnvelope(format!("this is a signed `{}`, not a `{kind}`", excerpt(&raw.kind, 20))));
    }
    let signer = PublicKey::parse(&raw.signer).map_err(|e| Error::BadEnvelope(format!("signer: {e}")))?;
    if signer != *key {
        return Err(Error::BadSignature("signed by another key than this toolbox's publisher".into()));
    }
    let bytes = B64.decode(&raw.signature).map_err(|_| Error::BadEnvelope("the signature is not base64".into()))?;
    let bytes: [u8; 64] = bytes.as_slice().try_into().map_err(|_| Error::BadEnvelope(format!("the signature is {} bytes, not 64", bytes.len())))?;
    let signature = Signature::from_bytes(&bytes);
    key.0.verify_strict(&message(kind, &raw.payload), &signature).map_err(|_| Error::BadSignature("the signature doesn't match the document".into()))?;
    Ok(raw.payload)
}

/// What is signed: a domain prefix, then the payload.
fn message(kind: Kind, payload: &str) -> Vec<u8> {
    let mut m = format!("artcraft-toolbox {kind} v1\n").into_bytes();
    m.extend_from_slice(payload.as_bytes());
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> SecretKey {
        SecretKey::from_seed([7; 32])
    }

    #[test]
    fn keys_round_trip_as_text_and_the_secret_never_prints() {
        let secret = key();
        let public = secret.public();
        assert!(public.to_string().starts_with("ed25519:") && public.to_string().len() == 8 + 44);
        assert_eq!(PublicKey::parse(&public.to_string()).unwrap(), public);
        assert_eq!(PublicKey::parse(&format!("  {public}\n")).unwrap(), public);
        let again = SecretKey::parse(&secret.to_string()).unwrap();
        assert_eq!(again.public(), public);
        assert!(!format!("{secret:?}").contains(&secret.to_string()[15..]), "Debug must not leak the seed");
        for bad in ["", "ed25519:", "ed25519:not base64!", "ed25519:AAAA", "rsa:AAAA", &format!("ed25519-secret:{}", B64.encode([1u8; 16]))] {
            assert!(PublicKey::parse(bad).is_err() && SecretKey::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn signed_documents_verify_and_tampering_is_refused() {
        let secret = key();
        let public = secret.public();
        let payload = "schema = 1\n[[app]]\nid = \"x\"\n";
        let envelope = sign(&secret, Kind::Catalog, payload);
        assert_eq!(verify(&envelope, &public, Kind::Catalog).unwrap(), payload);
        // The kind is part of what is signed.
        let e = verify(&envelope, &public, Kind::Feed).unwrap_err().to_string();
        assert!(e.contains("not a `feed`"), "{e}");
        // Another publisher's key.
        let other = SecretKey::from_seed([9; 32]).public();
        assert!(matches!(verify(&envelope, &other, Kind::Catalog), Err(Error::BadSignature(_))));
        // A changed payload, a changed signature.
        let mut v: serde_json::Value = serde_json::from_str(&envelope).unwrap();
        v["payload"] = serde_json::Value::String(format!("{payload}# evil\n"));
        assert!(matches!(verify(&v.to_string(), &public, Kind::Catalog), Err(Error::BadSignature(_))));
        let mut v: serde_json::Value = serde_json::from_str(&envelope).unwrap();
        v["signature"] = serde_json::Value::String(B64.encode([0u8; 64]));
        assert!(matches!(verify(&v.to_string(), &public, Kind::Catalog), Err(Error::BadSignature(_))));
        // A signer that isn't the pinned key, even with a signature that key would make.
        let other_secret = SecretKey::from_seed([9; 32]);
        let foreign = sign(&other_secret, Kind::Catalog, payload);
        assert!(matches!(verify(&foreign, &public, Kind::Catalog), Err(Error::BadSignature(_))));
    }

    #[test]
    fn malformed_envelopes_are_errors_not_panics() {
        let public = key().public();
        let good = sign(&key(), Kind::Feed, "{}");
        let cases = [
            "".to_string(),
            "not json".into(),
            "[]".into(),
            "{}".into(),
            good.replace("\"schema\":1", "\"schema\":2"),
            good.replace("\"kind\":\"feed\"", "\"kind\":\"other\""),
            good.replace("\"signer\":\"ed25519:", "\"signer\":\"rsa:"),
            good.replace("\"signature\":\"", "\"signature\":\"!!"),
        ];
        for bad in cases {
            let e = verify(&bad, &public, Kind::Feed);
            assert!(matches!(e, Err(Error::BadEnvelope(_)) | Err(Error::BadKey(_)) | Err(Error::BadSignature(_))), "{bad}: {e:?}");
        }
        let mut v: serde_json::Value = serde_json::from_str(&good).unwrap();
        v["signature"] = serde_json::Value::String(B64.encode([0u8; 10]));
        assert!(verify(&v.to_string(), &public, Kind::Feed).unwrap_err().to_string().contains("not 64"));
        assert!(matches!(verify(&" ".repeat(MAX_ENVELOPE_BYTES + 1), &public, Kind::Feed), Err(Error::TooLarge { .. })));
        // Extra fields are ignored (a newer publisher may add some).
        let mut v: serde_json::Value = serde_json::from_str(&good).unwrap();
        v["comment"] = serde_json::Value::String("hello".into());
        assert_eq!(verify(&v.to_string(), &public, Kind::Feed).unwrap(), "{}");
    }
}
