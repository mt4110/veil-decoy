use clap::{Parser, Subcommand, ValueEnum};
use rand::RngExt;
use reqwest::{StatusCode, Url, blocking::Client, redirect::Policy};
use serde::{Deserialize, Serialize};
use std::{env, error::Error, process::ExitCode, time::Duration};
use uuid::Uuid;

type CliResult<T> = Result<T, Box<dyn Error>>;

#[derive(Parser)]
#[command(
    version,
    about = "Generate decoy credentials and manage HTTP honeytokens"
)]
struct Cli {
    /// Base URL of the Veil Worker
    #[arg(short, long, global = true, env = "VEIL_EDGE_URL")]
    edge_url: Option<Url>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Generate a new decoy or register an HTTP honeytoken
    Generate {
        #[arg(value_enum)]
        kind: TokenKind,
        /// Label used to identify the placement
        #[arg(short, long)]
        name: String,
    },
    /// List registered tokens
    List,
    /// Show recent detection and delivery states for a token
    Events { id: Uuid },
    /// Revoke a registered token by ID
    Revoke { id: Uuid },
}

#[derive(Clone, Copy, ValueEnum)]
enum TokenKind {
    /// Monitored HTTP link
    Http,
    /// AWS-shaped access key pair (offline)
    Aws,
    /// Redis connection string (offline)
    Redis,
    /// Generic API key (offline)
    Api,
    /// PostgreSQL connection string (offline)
    Postgres,
    /// Slack-shaped bot token (offline)
    Slack,
}

#[derive(Serialize)]
struct Registration<'a> {
    id: Uuid,
    name: &'a str,
}

#[derive(Deserialize)]
struct TokenRecord {
    id: Uuid,
    name: String,
    kind: String,
    created_at: String,
}

#[derive(Deserialize)]
struct TokenPage {
    tokens: Vec<TokenRecord>,
    cursor: Option<String>,
}

#[derive(Deserialize)]
struct EventRecord {
    id: Uuid,
    observed_at: i64,
    state: String,
    attempts: u32,
    last_error: Option<String>,
}

struct EdgeClient {
    base_url: Url,
    client: Client,
    admin_token: String,
}

impl EdgeClient {
    fn new(base_url: Option<Url>) -> CliResult<Self> {
        let base_url = base_url.ok_or("Set VEIL_EDGE_URL or pass --edge-url.")?;
        let local = matches!(
            base_url.host_str(),
            Some("localhost" | "127.0.0.1" | "[::1]")
        );
        let safe_scheme = base_url.scheme() == "https" || (local && base_url.scheme() == "http");
        if !safe_scheme
            || !base_url.username().is_empty()
            || base_url.password().is_some()
            || base_url.query().is_some()
            || base_url.fragment().is_some()
        {
            return Err("Use an HTTPS edge URL without credentials, a query, or a fragment (HTTP is allowed for local development).".into());
        }

        let admin_token = env::var("VEIL_ADMIN_TOKEN").map_err(|_| "Set VEIL_ADMIN_TOKEN.")?;
        if admin_token.len() != 64 || !admin_token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("VEIL_ADMIN_TOKEN must contain 64 hexadecimal characters.".into());
        }

        let mut builder = Client::builder()
            .redirect(Policy::none())
            .timeout(Duration::from_secs(10));
        if local {
            builder = builder.no_proxy();
        }

        Ok(Self {
            base_url,
            client: builder.build()?,
            admin_token,
        })
    }

    fn endpoint(&self, parts: &[&str]) -> Url {
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .expect("validated HTTP(S) URL")
            .pop_if_empty()
            .extend(parts);
        url
    }

    fn register(&self, name: &str) -> CliResult<Url> {
        let id = Uuid::new_v4();
        let response = self
            .client
            .post(self.endpoint(&["register"]))
            .bearer_auth(&self.admin_token)
            .json(&Registration { id, name })
            .send()?;
        require_status(response.status(), StatusCode::CREATED)?;
        Ok(self.endpoint(&["ping", &id.to_string()]))
    }

    fn list(&self) -> CliResult<()> {
        let mut url = self.endpoint(&["tokens"]);
        loop {
            let response = self
                .client
                .get(url.clone())
                .bearer_auth(&self.admin_token)
                .send()?;
            require_status(response.status(), StatusCode::OK)?;
            let page: TokenPage = response.json()?;

            for token in page.tokens {
                println!(
                    "{}\t{}\t{}\t{}",
                    token.id,
                    single_line(&token.kind),
                    single_line(&token.created_at),
                    single_line(&token.name),
                );
            }

            let Some(cursor) = page.cursor else {
                return Ok(());
            };
            url.set_query(None);
            url.query_pairs_mut().append_pair("cursor", &cursor);
        }
    }

    fn revoke(&self, id: Uuid) -> CliResult<()> {
        let response = self
            .client
            .delete(self.endpoint(&["tokens", &id.to_string()]))
            .bearer_auth(&self.admin_token)
            .send()?;
        require_status(response.status(), StatusCode::NO_CONTENT)
    }

    fn events(&self, id: Uuid) -> CliResult<()> {
        let mut url = self.endpoint(&["events"]);
        url.query_pairs_mut()
            .append_pair("token_id", &id.to_string());
        let response = self.client.get(url).bearer_auth(&self.admin_token).send()?;
        require_status(response.status(), StatusCode::OK)?;
        for event in response.json::<Vec<EventRecord>>()? {
            println!(
                "{}\t{}\t{}\t{}\t{}",
                event.id,
                event.observed_at,
                single_line(&event.state),
                event.attempts,
                single_line(event.last_error.as_deref().unwrap_or("-")),
            );
        }
        Ok(())
    }
}

fn require_status(actual: StatusCode, expected: StatusCode) -> CliResult<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(format!("Edge API returned HTTP {actual}.").into())
    }
}

fn single_line(value: &str) -> String {
    value.replace(char::is_control, " ")
}

fn random_chars(len: usize, alphabet: &[u8]) -> String {
    let mut rng = rand::rng();
    (0..len)
        .map(|_| alphabet[rng.random_range(0..alphabet.len())] as char)
        .collect()
}

fn offline_decoy(kind: TokenKind) -> String {
    const ALPHANUMERIC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    const UPPERCASE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    const LOWERCASE: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";

    match kind {
        TokenKind::Aws => format!(
            "AWS_ACCESS_KEY_ID=AKIA{}\nAWS_SECRET_ACCESS_KEY={}",
            random_chars(16, UPPERCASE),
            random_chars(40, ALPHANUMERIC)
        ),
        TokenKind::Redis => format!(
            "redis://admin:{}@cache.example.invalid:6379/0",
            random_chars(24, ALPHANUMERIC)
        ),
        TokenKind::Api => format!("veil_{}", random_chars(40, LOWERCASE)),
        TokenKind::Postgres => format!(
            "postgresql://postgres:{}@db.example.invalid:5432/production",
            random_chars(24, ALPHANUMERIC)
        ),
        TokenKind::Slack => format!(
            "xoxb-{}-{}-{}",
            random_chars(10, ALPHANUMERIC),
            random_chars(10, ALPHANUMERIC),
            random_chars(24, ALPHANUMERIC)
        ),
        TokenKind::Http => unreachable!("HTTP honeytokens are registered with the Worker"),
    }
}

fn run(cli: Cli) -> CliResult<()> {
    match cli.command {
        Command::Generate {
            kind: TokenKind::Http,
            name,
        } => {
            println!("{}", EdgeClient::new(cli.edge_url)?.register(&name)?);
        }
        Command::Generate { kind, name } => {
            eprintln!(
                "Offline decoy: {}. This credential does not send alerts.",
                single_line(&name)
            );
            println!("{}", offline_decoy(kind));
        }
        Command::List => EdgeClient::new(cli.edge_url)?.list()?,
        Command::Events { id } => EdgeClient::new(cli.edge_url)?.events(id)?,
        Command::Revoke { id } => {
            EdgeClient::new(cli.edge_url)?.revoke(id)?;
            println!("Revoked {id}.");
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    if let Err(error) = run(Cli::parse()) {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
