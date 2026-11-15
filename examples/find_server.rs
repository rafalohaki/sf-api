//! # World-server finder for classic (non-SSO) accounts
//!
//! Diagnostic tool: classic accounts live on exactly one world server and
//! there is no global lookup, so like the official client (and MFBot) we try
//! one login per server. To keep the request count minimal this reads the
//! active server list from `https://sfgame.net/config.json` (respects
//! merges: a server merged into another one is skipped — the merge target
//! is in the list itself) and filters it by country.
//!
//! Credentials come from env, never from arguments:
//!
//! ```bash
//! SF_USER="name" SF_PW="pw" SF_COUNTRY="pl" cargo run --example find_server
//! ```
//!
//! Guidelines honored (crate README): no tight loop without delay — 1–2 s
//! random sleep between attempts; stops at the first server that accepts
//! the login.

use serde::Deserialize;
use sf_api::session::SimpleSession;

#[derive(Deserialize)]
struct Config {
    servers: Vec<ServerInfo>,
}

#[derive(Deserialize)]
struct ServerInfo {
    #[serde(rename = "i")]
    _id: i32,
    #[serde(rename = "d")]
    url: String,
    #[serde(rename = "c")]
    country: String,
    #[serde(rename = "md")]
    merged_into: Option<String>,
}

#[tokio::main]
async fn main() {
    let user = std::env::var("SF_USER").expect("SF_USER env missing");
    let pw = std::env::var("SF_PW").expect("SF_PW env missing");
    let country = std::env::var("SF_COUNTRY").unwrap_or_else(|_| "pl".into());
    // Ograniczenie celów (po przecinku) — np. recheck konkretnych światów
    // po fuzji, bez pełnego skanu kraju.
    let only = std::env::var("SF_ONLY").ok();

    let config: Config = serde_json::from_str(
        &reqwest::get("https://sfgame.net/config.json")
            .await
            .expect("config.json fetch")
            .text()
            .await
            .expect("config.json body"),
    )
    .expect("config.json parse");
    let targets: Vec<String> = match only {
        Some(list) => {
            list.split(',').map(str::trim).map(String::from).collect()
        }
        None => {
            // Servers get merged into international ones (all classic .pl
            // worlds now live on f*.sfgame.net): resolve each country
            // server through its merge target, then dedupe.
            let mut t: Vec<String> = config
                .servers
                .iter()
                .filter(|s| s.country == country)
                .map(|s| s.merged_into.as_deref().unwrap_or(&s.url).to_string())
                .collect();
            t.sort();
            t.dedup();
            t
        }
    };

    eprintln!("Kandydaci ({country}): {} serwerów", targets.len());

    for (n, server) in targets.iter().enumerate() {
        eprintln!("[{}/{}] próbuję {server} …", n + 1, targets.len());
        match SimpleSession::login(&user, &pw, server).await {
            Ok(session) => {
                let gs = session.game_state().expect("game state after login");
                println!("SERWER: {server}");
                println!(
                    "POSTAĆ: {} (lvl {}, {:?})",
                    gs.character.name, gs.character.level, gs.character.class
                );
                return;
            }
            Err(e) => {
                // Distinct errors matter: a wrong password on the right
                // server says so — a wrong server says "player not found".
                eprintln!("  ✗ {e}");
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(
            1000 + fastrand::u64(0..1000),
        ))
        .await;
    }
    eprintln!("❌ żadne państwo {country} nie przyjęło danych");
    std::process::exit(1);
}
