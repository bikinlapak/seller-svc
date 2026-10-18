//! Action tokens: the only way a suggestion becomes a write.
//!
//! Rule FR-18/FR-27: no AI path writes. The agent may only *sign* a proposal;
//! the write happens after the merchant taps, and the executor verifies the
//! token first.
//!
//! The token is an EdDSA (Ed25519) JWT so it stays readable during a demo and
//! while chasing a bug: paste it into any decoder and the claims are there.
//! Checked, in order:
//!
//! 1. signature, `aud` and `iss`,
//! 2. `exp` — expired tokens are rejected,
//! 3. `sub` — the token's tenant must match the caller,
//! 4. `jti` — single use; a second claim is rejected,
//! 5. `max_qty` — the final quantity may not exceed what the agent proposed.
//!
//! The order is deliberate: cheap and decisive checks first, and `jti` is only
//! marked used once everything else has passed.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

use crate::AppError;

const ISS: &str = "seller-svc";
const AUD: &str = "passtok-executor";

/// How long a suggestion token lives. Long enough to think it over while
/// opening the shop, not long enough to approve next week at last week's price.
pub const TTL_SECONDS: i64 = 60 * 60 * 12;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionClaims {
    pub iss: String,
    pub aud: String,
    /// Tenant (shop) that owns this suggestion.
    pub sub: String,
    /// Unique token id, used to enforce single use.
    pub jti: String,
    pub iat: i64,
    pub exp: i64,
    /// The permitted action, e.g. `create_po`.
    pub act: String,
    /// The suggestion this action is based on.
    pub sug: String,
    /// Upper bound on quantity. The merchant may go down, not up.
    pub max_qty: i64,
    pub unit: String,
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// Signing keys. Only the signer (the agent) holds the private half; the
/// executor needs the public key alone.
pub struct ActionKeys {
    enc: EncodingKey,
    dec: DecodingKey,
}

impl ActionKeys {
    /// `pkcs8` is the Ed25519 private key (DER), `public` the 32 public bytes.
    pub fn new(pkcs8: &[u8], public: &[u8]) -> Self {
        Self {
            enc: EncodingKey::from_ed_der(pkcs8),
            dec: DecodingKey::from_ed_der(public),
        }
    }

    /// Generate a throwaway key pair. Development and tests only: in staging
    /// and production the key comes from Secret Manager, so tokens do not all
    /// die on restart.
    pub fn ephemeral() -> Result<Self, AppError> {
        let rng = ring::rand::SystemRandom::new();
        let doc = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng)
            .map_err(|_| AppError::Config("could not generate an Ed25519 key".into()))?;
        let pair = ring::signature::Ed25519KeyPair::from_pkcs8(doc.as_ref())
            .map_err(|_| AppError::Config("generated Ed25519 key is invalid".into()))?;
        use ring::signature::KeyPair;
        Ok(Self::new(doc.as_ref(), pair.public_key().as_ref()))
    }

    pub fn sign(&self, claims: &ActionClaims) -> Result<String, AppError> {
        encode(&Header::new(Algorithm::EdDSA), claims, &self.enc)
            .map_err(|e| AppError::Config(format!("could not sign token: {e}")))
    }

    /// Verify signature, `exp`, `aud` and `iss`. Tenant, single use and the
    /// quantity cap are [`TokenGuard::claim`]'s job.
    pub fn verify(&self, token: &str) -> Result<ActionClaims, AppError> {
        let mut v = Validation::new(Algorithm::EdDSA);
        v.set_audience(&[AUD]);
        v.set_issuer(&[ISS]);
        v.set_required_spec_claims(&["exp", "aud", "iss", "sub"]);

        decode::<ActionClaims>(token, &self.dec, &v)
            .map(|d| d.claims)
            .map_err(|e| match e.kind() {
                jsonwebtoken::errors::ErrorKind::ExpiredSignature => AppError::TokenExpired,
                _ => AppError::TokenInvalid,
            })
    }
}

/// Build the claims for one suggestion.
pub fn suggestion_claims(tenant: &str, suggestion: &str, max_qty: i64, unit: &str) -> ActionClaims {
    action_claims(tenant, "create_po", suggestion, max_qty, unit)
}

/// Claims for any card action: `create_po`, `markdown`, `create_special`,
/// `record_waste`, `donate`. The executor dispatches on `act`.
pub fn action_claims(
    tenant: &str,
    act: &str,
    suggestion: &str,
    max_qty: i64,
    unit: &str,
) -> ActionClaims {
    let iat = now();
    ActionClaims {
        iss: ISS.into(),
        aud: AUD.into(),
        sub: tenant.into(),
        jti: ulid::Ulid::new().to_string(),
        iat,
        exp: iat + TTL_SECONDS,
        act: act.into(),
        sug: suggestion.into(),
        max_qty,
        unit: unit.into(),
    }
}

/// Record of spent `jti`s.
///
/// In memory for now. Once the `passtok` schema exists this becomes the
/// `action_tokens` table, with `used_at` written in the **same transaction**
/// as the worksh.io mutation — so single use survives a crash mid-request.
/// The property to preserve is in the tests: two simultaneous taps must
/// produce exactly one write.
#[derive(Default)]
pub struct TokenGuard {
    used: Mutex<HashMap<String, i64>>,
}

impl TokenGuard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Claim a token for one use.
    ///
    /// Returns `Ok` for the first caller only; anyone else presenting the same
    /// `jti` gets [`AppError::TokenUsed`].
    pub fn claim(
        &self,
        claims: &ActionClaims,
        tenant: &str,
        quantity: i64,
    ) -> Result<(), AppError> {
        if claims.sub != tenant {
            // A token belonging to another shop: treat it as forged.
            return Err(AppError::TokenInvalid);
        }
        if quantity < 1 {
            return Err(AppError::BadRequest("quantity must be at least 1".into()));
        }
        if quantity > claims.max_qty {
            return Err(AppError::QtyAboveCap {
                requested: quantity,
                cap: claims.max_qty,
            });
        }

        let mut used = self
            .used
            .lock()
            .map_err(|_| AppError::Config("token lock poisoned".into()))?;

        // Drop records whose token has expired; there is no point keeping a
        // jti that can never be presented again.
        let cutoff = now();
        used.retain(|_, exp| *exp > cutoff);

        if used.insert(claims.jti.clone(), claims.exp).is_some() {
            return Err(AppError::TokenUsed);
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn used_count(&self) -> usize {
        self.used.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TENANT: &str = "hangat-bakery";

    fn keys() -> ActionKeys {
        ActionKeys::ephemeral().unwrap()
    }

    #[test]
    fn round_trips_intact() {
        let k = keys();
        let original = suggestion_claims(TENANT, "sug-1", 12, "kg");
        let token = k.sign(&original).unwrap();
        assert_eq!(k.verify(&token).unwrap(), original);
    }

    #[test]
    fn rejects_another_keys_signature() {
        let token = keys()
            .sign(&suggestion_claims(TENANT, "sug-1", 12, "kg"))
            .unwrap();
        assert!(matches!(keys().verify(&token), Err(AppError::TokenInvalid)));
    }

    #[test]
    fn rejects_a_tampered_token() {
        let k = keys();
        let token = k
            .sign(&suggestion_claims(TENANT, "sug-1", 12, "kg"))
            .unwrap();
        // Change one payload character: the signature no longer matches.
        let mut parts: Vec<&str> = token.split('.').collect();
        let payload = parts[1].to_string();
        let altered = format!("{}A", &payload[..payload.len() - 1]);
        parts[1] = &altered;
        assert!(matches!(
            k.verify(&parts.join(".")),
            Err(AppError::TokenInvalid)
        ));
    }

    #[test]
    fn rejects_an_expired_token() {
        let k = keys();
        let mut c = suggestion_claims(TENANT, "sug-1", 12, "kg");
        c.iat = now() - 7200;
        c.exp = now() - 3600;
        let token = k.sign(&c).unwrap();
        assert!(matches!(k.verify(&token), Err(AppError::TokenExpired)));
    }

    #[test]
    fn a_token_is_single_use() {
        let g = TokenGuard::new();
        let c = suggestion_claims(TENANT, "sug-1", 12, "kg");
        assert!(g.claim(&c, TENANT, 9).is_ok());
        assert!(matches!(g.claim(&c, TENANT, 9), Err(AppError::TokenUsed)));
    }

    #[test]
    fn rejects_another_tenant() {
        let g = TokenGuard::new();
        let c = suggestion_claims(TENANT, "sug-1", 12, "kg");
        assert!(matches!(
            g.claim(&c, "another-shop", 9),
            Err(AppError::TokenInvalid)
        ));
        // Rejected before the jti was recorded, so the real owner can still
        // use their token.
        assert_eq!(g.used_count(), 0);
        assert!(g.claim(&c, TENANT, 9).is_ok());
    }

    #[test]
    fn rejects_a_quantity_above_the_cap() {
        let g = TokenGuard::new();
        let c = suggestion_claims(TENANT, "sug-1", 12, "kg");
        assert!(matches!(
            g.claim(&c, TENANT, 13),
            Err(AppError::QtyAboveCap {
                requested: 13,
                cap: 12
            })
        ));
        // And the token has not been spent.
        assert!(g.claim(&c, TENANT, 12).is_ok());
    }

    #[test]
    fn allows_revising_the_quantity_down() {
        for quantity in [1, 5, 12] {
            let g = TokenGuard::new();
            let c = suggestion_claims(TENANT, "sug-1", 12, "kg");
            assert!(g.claim(&c, TENANT, quantity).is_ok(), "quantity={quantity}");
        }
    }

    #[test]
    fn rejects_zero_or_negative_quantities() {
        let g = TokenGuard::new();
        let c = suggestion_claims(TENANT, "sug-1", 12, "kg");
        for quantity in [0, -1] {
            assert!(
                matches!(g.claim(&c, TENANT, quantity), Err(AppError::BadRequest(_))),
                "quantity={quantity}"
            );
        }
    }

    /// Two simultaneous taps on the same token: exactly one may win.
    #[test]
    fn concurrent_taps_produce_exactly_one_write() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        for _ in 0..50 {
            let g = Arc::new(TokenGuard::new());
            let c = Arc::new(suggestion_claims(TENANT, "sug-1", 12, "kg"));
            let winners = Arc::new(AtomicUsize::new(0));

            let handles: Vec<_> = (0..8)
                .map(|_| {
                    let (g, c, winners) = (g.clone(), c.clone(), winners.clone());
                    std::thread::spawn(move || {
                        if g.claim(&c, TENANT, 9).is_ok() {
                            winners.fetch_add(1, Ordering::SeqCst);
                        }
                    })
                })
                .collect();
            for h in handles {
                h.join().unwrap();
            }
            assert_eq!(winners.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn sweeps_expired_jtis() {
        let g = TokenGuard::new();
        let mut old = suggestion_claims(TENANT, "sug-old", 12, "kg");
        old.exp = now() - 1;
        // An old token is on record...
        g.used.lock().unwrap().insert(old.jti.clone(), old.exp);
        // ...and the next claim sweeps it away.
        let fresh = suggestion_claims(TENANT, "sug-new", 12, "kg");
        g.claim(&fresh, TENANT, 1).unwrap();
        assert_eq!(g.used_count(), 1);
    }
}
