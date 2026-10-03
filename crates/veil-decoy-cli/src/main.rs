use clap::{Parser, Subcommand, ValueEnum};
use rand::Rng;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

/// veil-decoy — Honeytoken generator & tracker
///
/// Generate realistic-looking fake credentials and register them with the
/// Veil edge server so you get an instant Slack alert the moment an attacker
/// tries to use them.
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate a new honeytoken
    Generate {
        /// Type of credential to spoof
        #[arg(value_enum)]
        kind: TokenKind,

        /// Human-readable label for this trap (used in alert messages)
        #[arg(short, long)]
        name: String,

        /// Veil edge endpoint to register the token with (optional)
        #[arg(short, long, env = "VEIL_EDGE_URL")]
        edge_url: Option<String>,
    },

    /// List all tokens registered with the edge (requires VEIL_EDGE_URL)
    List {
        #[arg(short, long, env = "VEIL_EDGE_URL")]
        edge_url: String,
    },
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum TokenKind {
    /// AWS access key pair (AKIA…)
    Aws,
    /// Redis connection string
    Redis,
    /// Generic API key / Bearer token
    Api,
    /// PostgreSQL DSN
    Postgres,
    /// Slack bot token
    Slack,
}

// ---------------------------------------------------------------------------
// Token generation
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
struct Honeytoken {
    id: String,
    name: String,
    kind: String,
    value: String,
    created_at: String,
}

fn generate_token(kind: TokenKind, name: &str) -> Honeytoken {
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let value = match kind {
        TokenKind::Aws => generate_aws_keypair(),
        TokenKind::Redis => generate_redis_url(),
        TokenKind::Api => generate_api_key(),
        TokenKind::Postgres => generate_postgres_dsn(),
        TokenKind::Slack => generate_slack_token(),
    };
    Honeytoken {
        id,
        name: name.to_string(),
        kind: format!("{:?}", kind).to_lowercase(),
        value,
        created_at: now,
    }
}

fn random_alphanum(len: usize) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char)
        .collect()
}

fn generate_aws_keypair() -> String {
    // Real AWS keys: AKIA[A-Z0-9]{16}  (20 chars total)
    let key_id = format!("AKIA{}", random_alphanum(16).to_uppercase());
    let secret = random_alphanum(40);
    format!(
        "AWS_ACCESS_KEY_ID={key_id}\nAWS_SECRET_ACCESS_KEY={secret}"
    )
}

fn generate_redis_url() -> String {
    let pass = random_alphanum(24);
    // Points to the Veil edge; placeholder replaced at registration time
    format!("redis://admin:{pass}@trap.veil-edge.workers.dev:6379/0")
}

fn generate_api_key() -> String {
    format!("veil_{}", random_alphanum(40).to_lowercase())
}

fn generate_postgres_dsn() -> String {
    let pass = random_alphanum(24);
    format!("postgresql://postgres:{pass}@trap.veil-edge.workers.dev:5432/production")
}

fn generate_slack_token() -> String {
    format!("xoxb-{}-{}-{}", random_alphanum(10), random_alphanum(10), random_alphanum(24))
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Generate { kind, name, edge_url } => {
            let token = generate_token(kind, &name);

            println!("\n🎯  Honeytoken generated\n");
            println!("  ID      : {}", token.id);
            println!("  Name    : {}", token.name);
            println!("  Kind    : {}", token.kind);
            println!("  Created : {}", token.created_at);
            println!("\n━━━  Embed the following into your target file  ━━━\n");
            println!("{}", token.value);
            println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

            if let Some(url) = edge_url {
                match register_with_edge(&url, &token) {
                    Ok(_) => println!("✅  Registered with edge server — monitoring active."),
                    Err(e) => eprintln!("⚠️   Edge registration failed: {e}"),
                }
            } else {
                println!("💡  Tip: set VEIL_EDGE_URL to auto-register with the edge tracker.");
            }
        }

        Commands::List { edge_url } => {
            println!("📋  Fetching registered tokens from {edge_url} …");
            // TODO: implement GET /tokens once edge API is live
            println!("(not yet implemented — coming in v0.2)");
        }
    }
}

fn register_with_edge(base_url: &str, token: &Honeytoken) -> Result<(), Box<dyn std::error::Error>> {
    let url = format!("{}/register", base_url.trim_end_matches('/'));
    let client = reqwest::blocking::Client::new();
    let res = client.post(&url).json(token).send()?;
    if res.status().is_success() {
        Ok(())
    } else {
        Err(format!("HTTP {}", res.status()).into())
    }
}
