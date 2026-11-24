use super::{JsResult, SlackAlert, property, to_json, webhook_url};
use futures_util::{StreamExt, stream};
use js_sys::{Array, Date, JSON};
use serde::Deserialize;
use wasm_bindgen::{JsCast, JsValue, prelude::wasm_bindgen};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AbortSignal, Headers, Request, RequestInit, RequestRedirect, Response, WorkerGlobalScope,
};
use worker_sys::{D1Database, D1PreparedStatement, D1Result, console_error};

const MAX_ATTEMPTS: u32 = 6;
const LEASE_MS: f64 = 60_000.0;
const RETENTION_MS: f64 = 7.0 * 24.0 * 60.0 * 60_000.0;

const EVENT_COLUMNS: &str = "id, token_id, observed_at, state, attempts, last_error";

#[derive(Deserialize)]
struct Claim {
    payload: String,
    attempts: u32,
}

struct DeliveryFailure {
    reason: String,
    retryable: bool,
    retry_after_ms: f64,
}

pub fn new_event_id() -> JsResult<String> {
    global().crypto().map(|crypto| crypto.random_uuid())
}

fn global() -> WorkerGlobalScope {
    js_sys::global().unchecked_into()
}

fn database(env: &JsValue) -> JsResult<D1Database> {
    Ok(property(env, "VEIL_EVENTS")?.unchecked_into())
}

pub async fn record(
    env: &JsValue,
    id: &str,
    token_id: &str,
    observed_at: f64,
    alert: Option<SlackAlert>,
) -> JsResult<()> {
    let state = if alert.is_some() {
        "pending"
    } else {
        "rate_limited"
    };
    let payload = alert.map(|alert| to_json(&alert)).transpose()?;
    run(
        &database(env)?,
        "INSERT INTO decoy_events (id, token_id, observed_at, state, next_attempt_at, payload)
         VALUES (?1, ?2, ?3, ?4, ?3, ?5)",
        &[
            id.into(),
            token_id.into(),
            observed_at.into(),
            state.into(),
            payload.map_or(JsValue::NULL, JsValue::from),
        ],
    )
    .await
}

pub async fn recent(env: &JsValue, token_id: &str) -> JsResult<String> {
    let rows = all(
        &database(env)?,
        &format!(
            "SELECT {EVENT_COLUMNS} FROM decoy_events WHERE token_id = ?1
                  ORDER BY observed_at DESC, id DESC LIMIT 50"
        ),
        &[token_id.into()],
    )
    .await?;
    Ok(JSON::stringify(rows.as_ref())?.into())
}

pub async fn deliver(env: &JsValue, id: &str) -> JsResult<()> {
    let db = database(env)?;
    let lease_id = new_event_id()?;
    let now = Date::now();
    let statement = bind(
        &db,
        "UPDATE decoy_events
         SET attempts = attempts + 1, lease_id = ?2, lease_until = ?3
         WHERE id = ?1 AND state = 'pending' AND attempts < ?4
           AND next_attempt_at <= ?5 AND (lease_until IS NULL OR lease_until <= ?5)
         RETURNING payload, attempts",
        &[
            id.into(),
            lease_id.as_str().into(),
            (now + LEASE_MS).into(),
            MAX_ATTEMPTS.into(),
            now.into(),
        ],
    )?;
    let result = JsFuture::from(statement.first(None)?).await?;
    if result.is_null() {
        return Ok(());
    }
    let claim: Claim = serde_json::from_str(&String::from(JSON::stringify(&result)?))
        .map_err(|_| JsValue::from_str("Invalid delivery record"))?;

    let outcome = match webhook_url(env)? {
        Some(url) => send(&url, &claim.payload).await,
        None => Err(transient("webhook_unavailable")),
    };
    let now = Date::now();
    let (state, error, next) = match outcome {
        Ok(()) => ("delivered", JsValue::NULL, now),
        Err(failure) => {
            let reason = failure.reason.into();
            if !failure.retryable || claim.attempts >= MAX_ATTEMPTS {
                ("failed", reason, now)
            } else {
                let backoff = 60_000.0 * 2_f64.powi(claim.attempts.saturating_sub(1) as i32);
                ("pending", reason, now + backoff.max(failure.retry_after_ms))
            }
        }
    };
    run(
        &db,
        "UPDATE decoy_events SET state = ?3, last_error = ?4, next_attempt_at = ?5,
           lease_id = NULL, lease_until = NULL, payload = CASE WHEN ?3 = 'pending' THEN payload END
         WHERE id = ?1 AND lease_id = ?2",
        &[id.into(), lease_id.into(), state.into(), error, next.into()],
    )
    .await
}

#[wasm_bindgen]
pub async fn retry(env: JsValue) -> JsResult<()> {
    scheduled(&env)
        .await
        .map_err(|_| JsValue::from_str("Veil Decoy retry sweep failed"))
}

async fn scheduled(env: &JsValue) -> JsResult<()> {
    let db = database(env)?;
    let now = Date::now();
    run(
        &db,
        "DELETE FROM decoy_events WHERE observed_at < ?1",
        &[(now - RETENTION_MS).into()],
    )
    .await?;
    run(
        &db,
        "UPDATE decoy_events
              SET state = 'failed', last_error = 'attempt_interrupted', payload = NULL,
                  lease_id = NULL, lease_until = NULL
              WHERE state = 'pending' AND attempts >= ?2 AND lease_until <= ?1",
        &[now.into(), MAX_ATTEMPTS.into()],
    )
    .await?;
    let rows = all(
        &db,
        "SELECT id FROM decoy_events
         WHERE state = 'pending' AND next_attempt_at <= ?1
           AND (lease_until IS NULL OR lease_until <= ?1) AND attempts < ?2
         ORDER BY next_attempt_at, id LIMIT 12",
        &[now.into(), MAX_ATTEMPTS.into()],
    )
    .await?;
    let mut ids = Vec::with_capacity(rows.length() as usize);
    for row in rows.iter() {
        if let Some(id) = property(&row, "id")?.as_string() {
            ids.push(id);
        }
    }

    let mut deliveries = stream::iter(ids)
        .map(|id| async move { deliver(env, &id).await })
        .buffer_unordered(4);
    let mut failed = false;
    while let Some(result) = deliveries.next().await {
        if result.is_err() {
            console_error!("Veil Decoy could not complete a delivery attempt");
            failed = true;
        }
    }
    if failed {
        return Err(JsValue::from_str("Delivery database operation failed"));
    }
    Ok(())
}

fn bind(db: &D1Database, query: &str, values: &[JsValue]) -> JsResult<D1PreparedStatement> {
    db.prepare(query)?.bind(values.iter().collect())
}

async fn all(db: &D1Database, query: &str, values: &[JsValue]) -> JsResult<Array> {
    let result: D1Result = JsFuture::from(bind(db, query, values)?.all()?)
        .await?
        .unchecked_into();
    result
        .results()?
        .ok_or_else(|| JsValue::from_str("Missing event rows"))
}

async fn run(db: &D1Database, query: &str, values: &[JsValue]) -> JsResult<()> {
    let result: D1Result = JsFuture::from(bind(db, query, values)?.run()?)
        .await?
        .unchecked_into();
    if !result.success()? {
        return Err(JsValue::from_str("Event database operation failed"));
    }
    Ok(())
}

async fn send(url: &str, payload: &str) -> Result<(), DeliveryFailure> {
    let headers = Headers::new().map_err(|_| transient("request_headers"))?;
    headers
        .set("Content-Type", "application/json")
        .map_err(|_| transient("request_headers"))?;
    let init = RequestInit::new();
    init.set_method("POST");
    init.set_headers_headers(&headers);
    init.set_body(&payload.into());
    init.set_redirect(RequestRedirect::Manual);
    init.set_signal(Some(&AbortSignal::timeout_with_u32(5000)));
    let request =
        Request::new_with_str_and_init(url, &init).map_err(|_| transient("webhook_request"))?;
    let result: Response = JsFuture::from(global().fetch_with_request(&request))
        .await
        .map_err(|_| transient("webhook_transport"))?
        .dyn_into()
        .map_err(|_| transient("webhook_response"))?;

    if result.ok() {
        return Ok(());
    }
    let status = result.status();
    let retry_after_ms = result
        .headers()
        .get("Retry-After")
        .ok()
        .flatten()
        .map_or(0.0, |value| {
            let delay = match value.parse::<u64>() {
                Ok(seconds) => seconds as f64 * 1000.0,
                Err(_) => Date::parse(&value) - Date::now(),
            };
            // Cap malformed or extreme delays to keep the retry window bounded.
            if delay.is_finite() {
                delay.clamp(0.0, 3_600_000.0)
            } else {
                0.0
            }
        });
    Err(DeliveryFailure {
        reason: format!("http_{status}"),
        retryable: matches!(status, 408 | 429 | 500..=599),
        retry_after_ms,
    })
}

fn transient(reason: &str) -> DeliveryFailure {
    DeliveryFailure {
        reason: reason.to_owned(),
        retryable: true,
        retry_after_ms: 0.0,
    }
}
