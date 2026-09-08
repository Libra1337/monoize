use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{Mutex, Notify, oneshot};

use crate::replica::admission_http::{
    ADMISSION_CONFIRM_PATH, ADMISSION_ISSUE_PATH, ADMISSION_KEYSET_PATH, public_admission_message,
};
use crate::replica::internal_http::{InternalResponseError, read_internal_response};
use crate::store_billing::admission_token::{
    AdmissionClaimStore, AdmissionError, AdmissionVerificationBinding, AdmissionVerifierKey,
    AdmissionVerifierRing, TerminalSpoolInput, VerifiedAdmission,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaIssueInput {
    pub user_id: String,
    pub request_id: String,
    pub effective_groups: Vec<String>,
    pub maximum_nano_usd: i128,
    pub pricing_revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplicaFundingDecision {
    Balance,
    Plan(VerifiedAdmission),
}

#[derive(Clone)]
pub struct AdmissionHandlerScope {
    inner: Arc<AdmissionHandlerScopeInner>,
}

struct AdmissionHandlerScopeInner {
    client: AdmissionClient,
    request_id: String,
    completed: AtomicBool,
}

impl AdmissionHandlerScope {
    pub async fn mark_routed(
        &self,
        routed_at: DateTime<Utc>,
    ) -> Result<bool, AdmissionClientError> {
        self.inner
            .client
            .mark_routed(&self.inner.request_id, routed_at)
            .await
    }

    pub async fn release(&self, created_at: DateTime<Utc>) -> Result<bool, AdmissionClientError> {
        let released = self
            .inner
            .client
            .release(&self.inner.request_id, created_at)
            .await?;
        self.inner.completed.store(true, Ordering::Release);
        Ok(released)
    }

    pub fn complete_if_inactive(&self) -> bool {
        if self.inner.client.has_active(&self.inner.request_id) {
            return false;
        }
        self.inner.completed.store(true, Ordering::Release);
        true
    }
}

impl Drop for AdmissionHandlerScopeInner {
    fn drop(&mut self) {
        if self.completed.swap(true, Ordering::AcqRel) {
            return;
        }
        let client = self.client.clone();
        let request_id = self.request_id.clone();
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::error!(%request_id, "unfinished admission scope dropped outside a Tokio runtime");
            return;
        };
        runtime.spawn(async move {
            if let Err(error) = client.release(&request_id, Utc::now()).await {
                tracing::error!(
                    code = error.code(),
                    detail = error.internal_message().unwrap_or(error.message()),
                    %request_id,
                    "unfinished admission scope could not publish release"
                );
            }
        });
    }
}

#[derive(Debug, Clone, Error)]
#[error("{code}: {message}")]
pub struct AdmissionClientError {
    code: String,
    message: String,
    internal_message: Option<String>,
}

impl AdmissionClientError {
    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn internal_message(&self) -> Option<&str> {
        self.internal_message.as_deref()
    }

    pub(crate) fn new(code: impl Into<String>, internal_message: impl Into<String>) -> Self {
        let code = code.into();
        Self {
            message: public_admission_message(&code).to_string(),
            code,
            internal_message: Some(internal_message.into()),
        }
    }
}

#[derive(Clone)]
pub struct AdmissionClient {
    http: reqwest::Client,
    primary_url: String,
    token: String,
    replica_id: String,
    refresh_interval: Duration,
    verifier: AdmissionVerifierRing,
    claims: Arc<AdmissionClaimStore>,
    refresh_gate: Arc<Mutex<()>>,
    active: Arc<DashMap<String, Arc<ActiveAdmission>>>,
    ship_notify: Arc<Notify>,
}

struct ActiveAdmission {
    admission: VerifiedAdmission,
    routed_at: Mutex<Option<DateTime<Utc>>>,
}

#[derive(Serialize)]
struct IssueRequest<'a> {
    audience: &'a str,
    user_id: &'a str,
    request_id: &'a str,
    effective_groups: &'a [String],
    maximum_nano_usd: String,
    pricing_revision: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "funding", rename_all = "lowercase", deny_unknown_fields)]
enum IssueResponse {
    Balance,
    Plan {
        token_id: String,
        reservation_id: String,
        compact_jws: String,
        issued_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
        duplicate: bool,
    },
}

#[derive(Serialize)]
struct ConfirmRequest<'a> {
    audience: &'a str,
    token_id: &'a str,
    reservation_id: &'a str,
    request_id: &'a str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfirmResponse {
    confirmed: bool,
    duplicate: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeysetResponse {
    keys: Vec<AdmissionVerifierKeyWire>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdmissionVerifierKeyWire {
    key_id: String,
    public_key_base64: String,
    state: String,
    activated_at: DateTime<Utc>,
    verify_until: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Deserialize)]
struct ErrorBody {
    code: String,
    message: String,
}

enum ConfirmAttemptError {
    Ambiguous(String),
    Final(AdmissionClientError),
}

impl AdmissionClient {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        http: reqwest::Client,
        primary_url: String,
        token: String,
        replica_id: String,
        refresh_interval: Duration,
        verifier: AdmissionVerifierRing,
        claims: Arc<AdmissionClaimStore>,
        ship_notify: Arc<Notify>,
    ) -> Result<Self, AdmissionClientError> {
        let parsed = reqwest::Url::parse(&primary_url).map_err(|error| {
            AdmissionClientError::new(
                "plan_admission_config_invalid",
                format!("Primary URL is invalid: {error}"),
            )
        })?;
        if !matches!(parsed.scheme(), "http" | "https")
            || token.is_empty()
            || replica_id.is_empty()
            || refresh_interval.is_zero()
        {
            return Err(AdmissionClientError::new(
                "plan_admission_config_invalid",
                "Replica admission configuration is invalid",
            ));
        }
        Ok(Self {
            http,
            primary_url: primary_url.trim_end_matches('/').to_string(),
            token,
            replica_id,
            refresh_interval,
            verifier,
            claims,
            refresh_gate: Arc::new(Mutex::new(())),
            active: Arc::new(DashMap::new()),
            ship_notify,
        })
    }

    pub fn with_refresh_interval(mut self, refresh_interval: Duration) -> Self {
        if !refresh_interval.is_zero() {
            self.refresh_interval = refresh_interval;
        }
        self
    }

    pub fn spawn_keyset_refresh_loop(
        self,
        shutdown: Arc<AtomicBool>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(self.refresh_interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                if shutdown.load(Ordering::Acquire) {
                    break;
                }
                let result = {
                    let _guard = self.refresh_gate.lock().await;
                    self.fetch_and_replace_keyset(Utc::now()).await
                };
                if let Err(error) = result {
                    tracing::warn!(
                        code = error.code(),
                        detail = error.internal_message().unwrap_or(error.message()),
                        "scheduled admission keyset refresh failed; retaining prior snapshot"
                    );
                }
            }
        })
    }

    pub fn verifier(&self) -> &AdmissionVerifierRing {
        &self.verifier
    }

    pub fn replica_id(&self) -> &str {
        &self.replica_id
    }

    pub fn claims(&self) -> &AdmissionClaimStore {
        &self.claims
    }

    pub fn has_active(&self, request_id: &str) -> bool {
        self.active.contains_key(request_id)
    }

    pub fn handler_scope(&self, request_id: &str) -> Option<AdmissionHandlerScope> {
        self.has_active(request_id).then(|| AdmissionHandlerScope {
            inner: Arc::new(AdmissionHandlerScopeInner {
                client: self.clone(),
                request_id: request_id.to_string(),
                completed: AtomicBool::new(false),
            }),
        })
    }

    pub async fn refresh_if_due(&self, now: DateTime<Utc>) -> Result<(), AdmissionClientError> {
        if !self.snapshot_due(now) {
            return Ok(());
        }
        let _guard = self.refresh_gate.lock().await;
        if !self.snapshot_due(now) {
            return Ok(());
        }
        self.fetch_and_replace_keyset(now).await
    }

    pub async fn issue(
        &self,
        input: ReplicaIssueInput,
    ) -> Result<ReplicaFundingDecision, AdmissionClientError> {
        validate_issue_input(&input)?;
        let now = Utc::now();
        self.refresh_if_due(now).await?;
        let response: IssueResponse = self
            .post_json(
                ADMISSION_ISSUE_PATH,
                &IssueRequest {
                    audience: &self.replica_id,
                    user_id: &input.user_id,
                    request_id: &input.request_id,
                    effective_groups: &input.effective_groups,
                    maximum_nano_usd: input.maximum_nano_usd.to_string(),
                    pricing_revision: &input.pricing_revision,
                },
                "plan_admission_issue_unavailable",
            )
            .await?;
        let IssueResponse::Plan {
            token_id,
            reservation_id,
            compact_jws,
            issued_at,
            expires_at,
            duplicate: _duplicate,
        } = response
        else {
            return Ok(ReplicaFundingDecision::Balance);
        };
        let binding = AdmissionVerificationBinding {
            audience: self.replica_id.clone(),
            token_id,
            reservation_id,
            request_id: input.request_id.clone(),
            maximum_nano_usd: input.maximum_nano_usd,
            pricing_revision: input.pricing_revision,
            issued_at,
            expires_at,
        };
        let observed_refresh = self.verifier.refreshed_at();
        let verified = match self.verifier.verify(&compact_jws, &binding, Utc::now()) {
            Ok(verified) => verified,
            Err(AdmissionError::UnknownKey) => {
                self.refresh_after_unknown_key(observed_refresh, Utc::now())
                    .await?;
                self.verifier
                    .verify(&compact_jws, &binding, Utc::now())
                    .map_err(verification_error)?
            }
            Err(error) => return Err(verification_error(error)),
        };
        let (sender, receiver) = oneshot::channel();
        let owner = self.clone();
        tokio::spawn(async move {
            let result = owner.claim_confirm_activate(verified).await;
            if let Err(result) = sender.send(result)
                && let Ok(ReplicaFundingDecision::Plan(admission)) = result
                && let Err(error) = owner.release(&admission.request_id, Utc::now()).await
            {
                tracing::error!(
                    code = error.code(),
                    detail = error.internal_message().unwrap_or(error.message()),
                    request_id = admission.request_id,
                    "cancelled admission handoff could not publish release"
                );
            }
        });
        receiver.await.map_err(|_| {
            AdmissionClientError::new(
                "plan_admission_issue_unavailable",
                "admission lifecycle owner stopped before handoff",
            )
        })?
    }

    async fn claim_confirm_activate(
        &self,
        verified: VerifiedAdmission,
    ) -> Result<ReplicaFundingDecision, AdmissionClientError> {
        self.claims.claim(&verified).await.map_err(claim_error)?;
        if let Err(error) = self.confirm(&verified).await {
            let _ = self.begin_release(&verified, Utc::now()).await;
            return Err(error);
        }
        if let Err(error) = self.claims.mark_confirmed(&verified.token_id).await {
            let _ = self.begin_release(&verified, Utc::now()).await;
            return Err(AdmissionClientError::new(
                "plan_admission_confirmation_failed",
                format!("confirmed claim could not be persisted: {error}"),
            ));
        }
        self.active.insert(
            verified.request_id.clone(),
            Arc::new(ActiveAdmission {
                admission: verified.clone(),
                routed_at: Mutex::new(None),
            }),
        );
        Ok(ReplicaFundingDecision::Plan(verified))
    }

    pub async fn mark_routed(
        &self,
        request_id: &str,
        routed_at: DateTime<Utc>,
    ) -> Result<bool, AdmissionClientError> {
        let Some(active) = self.active.get(request_id).map(|entry| entry.clone()) else {
            return Ok(false);
        };
        let mut guard = active.routed_at.lock().await;
        if guard.is_some() {
            return Ok(true);
        }
        if let Err(error) = self
            .claims
            .mark_routed(&active.admission.token_id, routed_at)
            .await
        {
            drop(guard);
            let _ = self.release(request_id, Utc::now()).await;
            return Err(AdmissionClientError::new(
                "plan_admission_dispatch_unavailable",
                format!("routed claim could not be persisted: {error}"),
            ));
        }
        *guard = Some(routed_at);
        Ok(true)
    }

    pub async fn settle(
        &self,
        request_id: &str,
        actual_nano_usd: i128,
        created_at: DateTime<Utc>,
    ) -> Result<bool, AdmissionClientError> {
        if actual_nano_usd < 0 {
            return Err(AdmissionClientError::new(
                "plan_admission_terminal_invalid",
                "actual_nano_usd must be nonnegative",
            ));
        }
        let Some(active) = self.active.get(request_id).map(|entry| entry.clone()) else {
            return Ok(false);
        };
        if active.routed_at.lock().await.is_none() {
            return Err(AdmissionClientError::new(
                "plan_admission_terminal_invalid",
                "a Plan request cannot settle before routing",
            ));
        }
        self.claims
            .spool_terminal(TerminalSpoolInput::settlement(
                &active.admission,
                actual_nano_usd,
                created_at,
            ))
            .await
            .map_err(terminal_error)?;
        self.ship_notify.notify_one();
        self.active.remove(request_id);
        Ok(true)
    }

    pub async fn release(
        &self,
        request_id: &str,
        created_at: DateTime<Utc>,
    ) -> Result<bool, AdmissionClientError> {
        let Some(active) = self.active.get(request_id).map(|entry| entry.clone()) else {
            return Ok(false);
        };
        self.begin_release(&active.admission, created_at).await?;
        self.active.remove(request_id);
        Ok(true)
    }

    fn snapshot_due(&self, now: DateTime<Utc>) -> bool {
        self.verifier.refreshed_at().is_none_or(|refreshed_at| {
            now.signed_duration_since(refreshed_at)
                .to_std()
                .map_or(true, |age| age >= self.refresh_interval)
        })
    }

    async fn refresh_after_unknown_key(
        &self,
        observed_refresh: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Result<(), AdmissionClientError> {
        let _guard = self.refresh_gate.lock().await;
        if self.verifier.refreshed_at() != observed_refresh {
            return Ok(());
        }
        self.fetch_and_replace_keyset(now).await
    }

    async fn fetch_and_replace_keyset(
        &self,
        now: DateTime<Utc>,
    ) -> Result<(), AdmissionClientError> {
        let response = self
            .request(reqwest::Method::GET, ADMISSION_KEYSET_PATH)
            .send()
            .await
            .map_err(|error| verification_transport(error.to_string()))?;
        let bytes = response_bytes(response, "plan_admission_verification_unavailable").await?;
        let keyset: KeysetResponse = serde_json::from_slice(&bytes).map_err(|error| {
            AdmissionClientError::new(
                "plan_admission_verification_unavailable",
                format!("Primary keyset response is invalid: {error}"),
            )
        })?;
        let keys = keyset
            .keys
            .into_iter()
            .map(|key| AdmissionVerifierKey {
                key_id: key.key_id,
                public_key_base64: key.public_key_base64,
                state: key.state,
                activated_at: key.activated_at,
                verify_until: key.verify_until,
            })
            .collect();
        self.verifier
            .replace_snapshot(keys, now)
            .map_err(verification_error)
    }

    async fn confirm(&self, admission: &VerifiedAdmission) -> Result<(), AdmissionClientError> {
        let bytes = serde_json::to_vec(&ConfirmRequest {
            audience: &admission.audience,
            token_id: &admission.token_id,
            reservation_id: &admission.reservation_id,
            request_id: &admission.request_id,
        })
        .map_err(|error| {
            AdmissionClientError::new(
                "plan_admission_confirmation_failed",
                format!("confirmation request could not be encoded: {error}"),
            )
        })?;
        for attempt in 0..2 {
            match self.confirm_once(bytes.clone()).await {
                Ok(()) => return Ok(()),
                Err(ConfirmAttemptError::Ambiguous(message)) if attempt == 0 => {
                    tracing::warn!(%message, "admission confirmation response was ambiguous; retrying once");
                }
                Err(ConfirmAttemptError::Ambiguous(message)) => {
                    return Err(AdmissionClientError::new(
                        "plan_admission_confirmation_failed",
                        message,
                    ));
                }
                Err(ConfirmAttemptError::Final(error)) => return Err(error),
            }
        }
        unreachable!("confirmation loop returns on every second attempt")
    }

    async fn confirm_once(&self, body: Vec<u8>) -> Result<(), ConfirmAttemptError> {
        let response = self
            .request(reqwest::Method::POST, ADMISSION_CONFIRM_PATH)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(|error| ConfirmAttemptError::Ambiguous(error.to_string()))?;
        let (status, bytes) = match read_internal_response(response).await {
            Ok(response) => response,
            Err(InternalResponseError::Transport(detail)) => {
                return Err(ConfirmAttemptError::Ambiguous(detail));
            }
            Err(InternalResponseError::TooLarge) => {
                return Err(ConfirmAttemptError::Final(AdmissionClientError::new(
                    "plan_admission_confirmation_failed",
                    "Primary admission response exceeds 65536 bytes",
                )));
            }
        };
        if !status.is_success() {
            return Err(ConfirmAttemptError::Final(remote_error(
                status,
                &bytes,
                "plan_admission_confirmation_failed",
            )));
        }
        let response: ConfirmResponse = serde_json::from_slice(&bytes).map_err(|error| {
            ConfirmAttemptError::Final(AdmissionClientError::new(
                "plan_admission_confirmation_failed",
                format!("confirmation response is invalid: {error}"),
            ))
        })?;
        let _duplicate = response.duplicate;
        if !response.confirmed {
            return Err(ConfirmAttemptError::Final(AdmissionClientError::new(
                "plan_admission_confirmation_failed",
                "Primary did not confirm the admission token",
            )));
        }
        Ok(())
    }

    async fn begin_release(
        &self,
        admission: &VerifiedAdmission,
        created_at: DateTime<Utc>,
    ) -> Result<(), AdmissionClientError> {
        self.claims
            .mark_release_pending(&admission.token_id)
            .await
            .map_err(terminal_error)?;
        self.claims
            .spool_terminal(TerminalSpoolInput::release(admission, created_at))
            .await
            .map_err(terminal_error)?;
        self.ship_notify.notify_one();
        Ok(())
    }

    async fn post_json<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: &impl Serialize,
        fallback_code: &str,
    ) -> Result<T, AdmissionClientError> {
        let response = self
            .request(reqwest::Method::POST, path)
            .json(body)
            .send()
            .await
            .map_err(|error| AdmissionClientError::new(fallback_code, error.to_string()))?;
        let bytes = response_bytes(response, fallback_code).await?;
        serde_json::from_slice(&bytes).map_err(|error| {
            AdmissionClientError::new(
                fallback_code,
                format!("Primary admission response is invalid: {error}"),
            )
        })
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{path}", self.primary_url))
            .bearer_auth(&self.token)
            .header("X-Monoize-Replica-ID", &self.replica_id)
    }
}

async fn response_bytes(
    response: reqwest::Response,
    fallback_code: &str,
) -> Result<bytes::Bytes, AdmissionClientError> {
    let (status, bytes) = read_internal_response(response)
        .await
        .map_err(|error| AdmissionClientError::new(fallback_code, error.to_string()))?;
    if !status.is_success() {
        return Err(remote_error(status, &bytes, fallback_code));
    }
    Ok(bytes)
}

fn remote_error(status: StatusCode, bytes: &[u8], fallback_code: &str) -> AdmissionClientError {
    serde_json::from_slice::<ErrorEnvelope>(bytes).map_or_else(
        |_| {
            AdmissionClientError::new(
                fallback_code,
                format!("Primary admission request failed with HTTP {status}"),
            )
        },
        |envelope| {
            let remote_code = envelope.error.code;
            let code = if known_remote_code(&remote_code) {
                remote_code.as_str()
            } else {
                fallback_code
            };
            AdmissionClientError::new(
                code,
                format!(
                    "Primary admission request failed with HTTP {status}: remote code {remote_code}: {}",
                    envelope.error.message
                ),
            )
        },
    )
}

fn known_remote_code(code: &str) -> bool {
    matches!(
        code,
        "plan_quota_exhausted"
            | "plan_request_unbounded"
            | "plan_payment_hold"
            | "plan_quota_violation_blocked"
            | "admission_input_invalid"
            | "admission_active_key_missing"
            | "admission_wrap_key_missing"
            | "admission_key_invalid"
            | "admission_issuer_invalid"
            | "admission_issue_conflict"
            | "admission_token_not_found"
            | "admission_binding_mismatch"
            | "admission_terminal_digest_invalid"
            | "admission_terminal_conflict"
            | "admission_confirmation_expired"
            | "admission_storage_error"
            | "quota_gate_unavailable"
            | "quota_storage_error"
    )
}

fn validate_issue_input(input: &ReplicaIssueInput) -> Result<(), AdmissionClientError> {
    if input.user_id.is_empty()
        || input.request_id.is_empty()
        || input.pricing_revision.is_empty()
        || input.maximum_nano_usd <= 0
    {
        return Err(AdmissionClientError::new(
            "plan_admission_input_invalid",
            "Replica admission input is invalid",
        ));
    }
    Ok(())
}

fn verification_transport(message: String) -> AdmissionClientError {
    AdmissionClientError::new("plan_admission_verification_unavailable", message)
}

fn verification_error(error: AdmissionError) -> AdmissionClientError {
    AdmissionClientError::new("plan_admission_verification_unavailable", error.to_string())
}

fn claim_error(error: AdmissionError) -> AdmissionClientError {
    AdmissionClientError::new("plan_admission_claim_failed", error.to_string())
}

fn terminal_error(error: AdmissionError) -> AdmissionClientError {
    AdmissionClientError::new("plan_admission_terminal_failed", error.to_string())
}
