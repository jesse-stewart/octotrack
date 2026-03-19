//! JWT auth: config, token issuing, and `BearerAuth` extractor.

use actix_web::{web, FromRequest, HttpRequest};
use futures_util::future::{ready, Ready};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use std::time::{SystemTime, UNIX_EPOCH};

/// JWT claims payload.
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    /// Subject (always "octotrack").
    pub sub: String,
    /// Expiry timestamp (unix seconds).
    pub exp: u64,
}

/// Holds the JWT signing secret (lives in memory only, re-randomised on restart).
#[derive(Clone)]
pub struct JwtConfig {
    pub secret: Vec<u8>,
    pub session_timeout_hours: u32,
}

impl JwtConfig {
    pub fn new(session_timeout_hours: u32) -> Self {
        let secret: [u8; 32] = rand::random();
        Self {
            secret: secret.to_vec(),
            session_timeout_hours,
        }
    }

    pub fn issue_token(&self) -> Result<String, jsonwebtoken::errors::Error> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let claims = Claims {
            sub: "octotrack".to_string(),
            exp: now + self.session_timeout_hours as u64 * 3600,
        };
        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(&self.secret),
        )
    }

    pub fn verify_token(&self, token: &str) -> bool {
        let mut validation = Validation::default();
        validation.validate_exp = true;
        decode::<Claims>(token, &DecodingKey::from_secret(&self.secret), &validation).is_ok()
    }
}

/// Extractor that validates a Bearer token from `Authorization` header or `session` cookie.
/// Handlers that require auth add `_auth: BearerAuth` as a parameter.
pub struct BearerAuth;

impl FromRequest for BearerAuth {
    type Error = actix_web::Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut actix_web::dev::Payload) -> Self::Future {
        let jwt_cfg = req
            .app_data::<web::Data<JwtConfig>>()
            .expect("JwtConfig must be registered as app_data");

        // Try Authorization: Bearer <token>
        let token = req
            .headers()
            .get("Authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::to_owned);

        // Fall back to session cookie
        let token = token.or_else(|| req.cookie("session").map(|c| c.value().to_owned()));

        match token {
            Some(t) if jwt_cfg.verify_token(&t) => ready(Ok(BearerAuth)),
            _ => ready(Err(actix_web::error::ErrorUnauthorized(""))),
        }
    }
}

/// Async version — not needed since Ready implements Future, but kept for clarity.
impl Future for BearerAuth {
    type Output = Result<BearerAuth, actix_web::Error>;
    fn poll(
        self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::task::Poll::Ready(Ok(BearerAuth))
    }
}
