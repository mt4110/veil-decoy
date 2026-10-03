use serde::{Deserialize, Serialize};
use worker::*;

// ---------------------------------------------------------------------------
// Data models
// ---------------------------------------------------------------------------

/// Payload sent by the CLI when registering a new honeytoken.
#[derive(Debug, Deserialize, Serialize)]
struct RegisterPayload {
    id: String,
    name: String,
    kind: String,
    value: String,
    created_at: String,
}

/// The alert payload forwarded to Slack.
#[derive(Serialize)]
struct SlackAlert {
    text: String,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

#[event(fetch)]
pub async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let router = Router::new();

    router
        // POST /register — CLI registers a new honeytoken
        .post_async("/register", |mut req, ctx| async move {
            let payload: RegisterPayload = req.json().await?;

            // Persist to KV (token_id -> JSON metadata)
            let kv = ctx.kv("VEIL_TOKENS")?;
            kv.put(&payload.id, serde_json::to_string(&payload)?)?
                .execute()
                .await?;

            Response::ok(format!("registered:{}", payload.id))
        })
        // GET /ping/<token_id> — attacker "uses" a honeytoken URL
        // (embed this path in Redis URLs, API base URLs, etc.)
        .get_async("/ping/:token_id", |req, ctx| async move {
            let token_id = ctx.param("token_id").unwrap_or("unknown");
            let ip = req
                .headers()
                .get("CF-Connecting-IP")?
                .unwrap_or_else(|| "unknown".into());
            let ua = req
                .headers()
                .get("User-Agent")?
                .unwrap_or_else(|| "unknown".into());
            let country = req
                .headers()
                .get("CF-IPCountry")?
                .unwrap_or_else(|| "??".into());
            let url = req.url()?.to_string();

            // Look up token metadata from KV
            let kv = ctx.kv("VEIL_TOKENS")?;
            let name = if let Some(raw) = kv.get(token_id).text().await? {
                let meta: RegisterPayload = serde_json::from_str(&raw).unwrap_or_else(|_| {
                    RegisterPayload {
                        id: token_id.to_string(),
                        name: "(unknown trap)".into(),
                        kind: "?".into(),
                        value: String::new(),
                        created_at: String::new(),
                    }
                });
                meta.name
            } else {
                "(unregistered token)".into()
            };

            // Fire Slack alert
            if let Ok(slack_url) = ctx.secret("SLACK_WEBHOOK_URL") {
                let alert = SlackAlert {
                    text: format!(
                        "🚨 *[Veil Decoy Alert]* Honeytoken triggered!\n\
                        • Trap   : *{name}*\n\
                        • Token  : `{token_id}`\n\
                        • IP     : `{ip}` ({country})\n\
                        • UA     : `{ua}`\n\
                        • URL    : {url}\n\
                        ⚡ Initiate incident response immediately."
                    ),
                };
                let mut headers = Headers::new();
                headers.set("Content-Type", "application/json")?;
                let mut init = RequestInit::new();
                init.with_method(Method::Post)
                    .with_headers(headers)
                    .with_body(Some(serde_json::to_string(&alert)?.into()));
                let slack_req = Request::new_with_init(slack_url.to_string().as_str(), &init)?;
                // best-effort — don't fail the response if Slack is down
                let _ = Fetch::Request(slack_req).send().await;
            }

            // Return a convincing-looking error to the attacker so they
            // think the service is just temporarily down.
            let mut res = Response::error("Internal Server Error", 500)?;
            res.headers_mut().set("Retry-After", "30")?;
            Ok(res)
        })
        // GET /health — sanity check
        .get("/health", |_, _| Response::ok("ok"))
        .run(req, env)
        .await
}
