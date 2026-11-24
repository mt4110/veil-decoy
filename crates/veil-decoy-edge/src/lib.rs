use futures_util::StreamExt;
use js_sys::{Array, Date, Object, Promise, Reflect};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use subtle::ConstantTimeEq;
use uuid::Uuid;
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_futures::{JsFuture, future_to_promise};
use wasm_streams::ReadableStream;
use web_sys::{Request, Response, ResponseInit, Url};
use worker_sys::{Context, RateLimiter, console_error};

type JsResult<T> = Result<T, JsValue>;
const MAX_REGISTRATION_BYTES: usize = 4096;

#[wasm_bindgen]
extern "C" {
    type TokenStore;

    #[wasm_bindgen(method, catch, js_name = get)]
    fn get_token(this: &TokenStore, id: &str) -> JsResult<Promise>;

    #[wasm_bindgen(method, catch, js_name = put)]
    fn put_token(this: &TokenStore, id: &str, json: &str) -> JsResult<Promise>;

    #[wasm_bindgen(method, catch, js_name = list)]
    fn list_keys(this: &TokenStore, options: &Object) -> JsResult<Promise>;

    #[wasm_bindgen(method, catch, js_name = delete)]
    fn delete_token(this: &TokenStore, id: &str) -> JsResult<Promise>;
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    id: String,
    name: String,
}

#[derive(Deserialize, Serialize)]
struct Token {
    id: String,
    name: String,
    kind: String,
    created_at: String,
}

#[derive(Serialize)]
struct TokenPage {
    tokens: Vec<Token>,
    cursor: Option<String>,
}

#[derive(Serialize)]
struct SlackAlert {
    text: String,
    mrkdwn: bool,
    parse: &'static str,
    unfurl_links: bool,
    unfurl_media: bool,
}

#[wasm_bindgen]
pub async fn fetch(req: Request, env: JsValue, ctx: Context) -> JsResult<Response> {
    match handle(req, &env, &ctx).await {
        Ok(response) => Ok(response),
        Err(_) => {
            console_error!("Veil Decoy request failed");
            response(500, Some("Internal Server Error"))
        }
    }
}

async fn handle(req: Request, env: &JsValue, ctx: &Context) -> JsResult<Response> {
    let url = Url::new(&req.url())?;
    let path = url.pathname();
    let method = req.method();

    if method == "GET" && path == "/health" {
        return response(200, Some("ok"));
    }

    if matches!(method.as_str(), "GET" | "HEAD")
        && let Some(id) = path.strip_prefix("/ping/")
    {
        if valid_id(id) && observe_ping(&req, env, ctx, id).await.is_err() {
            console_error!("Veil Decoy could not process a honeytoken request");
        }
        return response(404, (method == "GET").then_some("Not Found"));
    }

    let revoke_id = (method == "DELETE")
        .then(|| path.strip_prefix("/tokens/"))
        .flatten();
    if !((method == "POST" && path == "/register")
        || (method == "GET" && matches!(path.as_str(), "/tokens" | "/events"))
        || revoke_id.is_some())
    {
        return response(404, Some("Not Found"));
    }

    let configured_token = property(env, "VEIL_ADMIN_TOKEN")?.as_string();
    let auth_header = req.headers().get("Authorization")?;
    if !authorized(configured_token.as_deref(), auth_header.as_deref()) {
        let result = response(401, Some("Unauthorized"))?;
        result.headers().set("WWW-Authenticate", "Bearer")?;
        return Ok(result);
    }

    if path == "/events" {
        let Some(id) = url
            .search_params()
            .get("token_id")
            .filter(|id| valid_id(id))
        else {
            return response(400, Some("Invalid token ID"));
        };
        return json_response(&outbox::recent(env, &id).await?);
    }

    let kv: TokenStore = property(env, "VEIL_TOKENS")?.unchecked_into();
    if method == "POST" {
        return register(&req, env, &kv).await;
    }
    if let Some(id) = revoke_id {
        if !valid_id(id) {
            return response(400, Some("Invalid token ID"));
        }
        JsFuture::from(kv.delete_token(id)?).await?;
        return response(204, None);
    }
    list_tokens(&kv, url.search_params().get("cursor")).await
}

async fn register(req: &Request, env: &JsValue, kv: &TokenStore) -> JsResult<Response> {
    if webhook_url(env)?.is_none() {
        return response(503, Some("Slack webhook is not configured"));
    }

    let Some(body) = req.body() else {
        return response(400, Some("Missing body"));
    };
    let mut stream = ReadableStream::from_raw(body).into_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?.dyn_into::<js_sys::Uint8Array>()?;
        if chunk.length() as usize > MAX_REGISTRATION_BYTES - bytes.len() {
            return response(413, Some("Registration body too large"));
        }
        bytes.extend_from_slice(&chunk.to_vec());
    }

    let Ok(payload) = serde_json::from_slice::<Registration>(&bytes) else {
        return response(400, Some("Invalid registration"));
    };
    let name = payload.name.trim();
    if !valid_id(&payload.id)
        || name.is_empty()
        || name.len() > 120
        || name.chars().any(char::is_control)
    {
        return response(400, Some("Invalid token ID or name"));
    }

    if !JsFuture::from(kv.get_token(&payload.id)?).await?.is_null() {
        return response(409, Some("Token already exists"));
    }

    let token = Token {
        id: payload.id,
        name: name.to_owned(),
        kind: "http".to_owned(),
        created_at: Date::new_0().to_iso_string().into(),
    };
    let json = to_json(&token)?;
    JsFuture::from(kv.put_token(&token.id, &json)?).await?;
    response(201, None)
}

async fn list_tokens(kv: &TokenStore, cursor: Option<String>) -> JsResult<Response> {
    // Leave headroom for the KV list and per-key reads on the Workers Free plan.
    let options = Object::new();
    Reflect::set(&options, &"limit".into(), &20.into())?;
    if let Some(cursor) = cursor {
        Reflect::set(&options, &"cursor".into(), &cursor.into())?;
    }
    let page = JsFuture::from(kv.list_keys(&options)?).await?;
    let keys = property(&page, "keys")?.dyn_into::<Array>()?;
    let mut tokens = Vec::new();
    for key in keys.iter() {
        if let Some(id) = property(&key, "name")?
            .as_string()
            .filter(|id| valid_id(id))
            && let Some(token) = read_token(kv, &id).await?
        {
            tokens.push(token);
        }
    }

    let cursor = if property(&page, "list_complete")?.as_bool() == Some(true) {
        None
    } else {
        property(&page, "cursor")?.as_string()
    };
    json_response(&to_json(&TokenPage { tokens, cursor })?)
}

async fn read_token(kv: &TokenStore, id: &str) -> JsResult<Option<Token>> {
    let value = JsFuture::from(kv.get_token(id)?).await?;
    let Some(raw) = value.as_string() else {
        return Ok(None);
    };
    // Older records may contain a credential value. Deserialize only metadata.
    match serde_json::from_str::<Token>(&raw) {
        Ok(token) if token.id == id => Ok(Some(token)),
        _ => {
            console_error!("Veil Decoy encountered invalid stored token metadata");
            Ok(None)
        }
    }
}

async fn observe_ping(req: &Request, env: &JsValue, ctx: &Context, id: &str) -> JsResult<()> {
    let headers = req.headers();
    let ip = headers
        .get("CF-Connecting-IP")?
        .and_then(|value| value.parse::<IpAddr>().ok())
        .map_or_else(|| "unknown".to_owned(), |ip| ip.to_string());
    if !rate_allowed(env, "VEIL_REQUEST_LIMIT", &ip).await? {
        return Ok(());
    }
    let kv: TokenStore = property(env, "VEIL_TOKENS")?.unchecked_into();
    let Some(token) = read_token(&kv, id).await? else {
        return Ok(());
    };
    let event_id = outbox::new_event_id()?;
    let observed_at = Date::now();
    if !rate_allowed(env, "VEIL_ALERT_LIMIT", id).await? {
        return outbox::record(env, &event_id, id, observed_at, None).await;
    }

    let cf = property(req.as_ref(), "cf")?;
    let country = if cf.is_object() {
        property(&cf, "country")?.as_string()
    } else {
        None
    };
    let country = header_text(country.or(headers.get("CF-IPCountry")?), 8);
    let agent = header_text(headers.get("User-Agent")?, 256);
    let name = header_text(Some(token.name), 120);
    let alert = SlackAlert {
        text: format!(
            "Veil Decoy honeytoken accessed\nEvent: {event_id}\nTrap: {name}\nToken: {id}\nIP: {ip} ({country})\nUser-Agent: {agent}"
        ),
        mrkdwn: false,
        parse: "none",
        unfurl_links: false,
        unfurl_media: false,
    };

    outbox::record(env, &event_id, id, observed_at, Some(alert)).await?;
    let env = env.clone();
    ctx.wait_until(&future_to_promise(async move {
        if outbox::deliver(&env, &event_id).await.is_err() {
            console_error!("Veil Decoy could not complete a delivery attempt");
        }
        Ok(JsValue::UNDEFINED)
    }))
}

async fn rate_allowed(env: &JsValue, binding: &str, key: &str) -> JsResult<bool> {
    let limiter: RateLimiter = property(env, binding)?.unchecked_into();
    let options = Object::new();
    Reflect::set(&options, &"key".into(), &key.into())?;
    let result = JsFuture::from(limiter.limit(options)?).await?;
    Ok(property(&result, "success")?.as_bool() == Some(true))
}

fn webhook_url(env: &JsValue) -> JsResult<Option<String>> {
    let value = property(env, "SLACK_WEBHOOK_URL")?.as_string();
    Ok(value.filter(|value| {
        Url::new(value).is_ok_and(|url| {
            url.protocol() == "https:" && url.username().is_empty() && url.password().is_empty()
        })
    }))
}

fn json_response(json: &str) -> JsResult<Response> {
    let result = response(200, Some(json))?;
    result.headers().set("Content-Type", "application/json")?;
    Ok(result)
}

fn response(status: u16, body: Option<&str>) -> JsResult<Response> {
    let init = ResponseInit::new();
    init.set_status(status);
    let result = Response::new_with_opt_str_and_init(body, &init)?;
    result.headers().set("Cache-Control", "no-store")?;
    Ok(result)
}

fn property(object: &JsValue, name: &str) -> JsResult<JsValue> {
    Reflect::get(object, &name.into())
}

fn to_json(value: &impl Serialize) -> JsResult<String> {
    serde_json::to_string(value).map_err(|_| JsValue::from_str("Serialization failed"))
}

fn valid_id(id: &str) -> bool {
    Uuid::parse_str(id).is_ok_and(|uuid| uuid.get_version_num() == 4 && uuid.to_string() == id)
}

fn authorized(configured: Option<&str>, authorization: Option<&str>) -> bool {
    let Some(secret) =
        configured.filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
    else {
        return false;
    };
    let Some((scheme, value)) = authorization.and_then(|s| s.split_once(' ')) else {
        return false;
    };
    scheme.eq_ignore_ascii_case("Bearer") && bool::from(value.as_bytes().ct_eq(secret.as_bytes()))
}

fn header_text(value: Option<String>, limit: usize) -> String {
    value
        .unwrap_or_else(|| "unknown".to_owned())
        .chars()
        .take(limit)
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorization_requires_the_configured_bearer() {
        let secret = "a".repeat(64);
        let bearer = format!("Bearer {secret}");
        assert!(authorized(Some(&secret), Some(&bearer)));
        assert!(!authorized(Some(&secret), None));
        assert!(!authorized(None, Some(&bearer)));
        assert!(!authorized(Some(&secret), Some(&format!("Basic {secret}"))));
        assert!(!authorized(
            Some(&secret),
            Some(&format!("Bearer {secret}a"))
        ));
    }

    #[test]
    fn legacy_credentials_are_excluded_from_inventory() {
        let legacy = r#"{
            "id":"a880f40b-5d0e-4b32-9b3a-c1398960d67a",
            "name":"Legacy trap",
            "kind":"aws",
            "created_at":"2024-04-05T00:00:00Z",
            "value":"old-decoy-credential"
        }"#;
        let token: Token = serde_json::from_str(legacy).unwrap();
        let stored_metadata = serde_json::to_value(token).unwrap();
        assert_eq!(stored_metadata["name"], "Legacy trap");
        assert!(stored_metadata.get("value").is_none());
    }
}
mod outbox;
