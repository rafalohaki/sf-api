//! # Expedition state probe (diagnostic)
//!
//! Logs in, sends a raw `Update`, prints every raw response key containing
//! `exped` and the parsed expedition state — so a "stale expedition gate" is
//! diagnosed from the actual wire (key present? parsed empty? action still
//! Expedition?).

use sf_api::command::Command;
use sf_api::gamestate::GameState;
use sf_api::session::{ServerConnection, Session};

#[tokio::main]
async fn main() {
    let user = std::env::var("SF_USER").expect("SF_USER env missing");
    let pw = std::env::var("SF_PW").expect("SF_PW env missing");
    let server = std::env::var("SF_SERVER").expect("SF_SERVER env missing");

    let conn = ServerConnection::new(&server).expect("server url");
    let mut session = Session::new(&user, &pw, conn);
    let login = session.login().await.expect("login");
    println!("--- login response keys with 'exped':");
    for (k, v) in login.values() {
        if k.to_ascii_lowercase().contains("exped") {
            println!(
                "{k} = {}",
                v.as_str().chars().take(160).collect::<String>()
            );
        }
    }
    let mut gs = GameState::new(login).expect("game state");

    let resp = session
        .send_command_raw(&Command::Update)
        .await
        .expect("update");
    println!("--- raw keys with 'exped':");
    for (k, v) in resp.values() {
        if k.to_ascii_lowercase().contains("exped") {
            println!(
                "{k} = {}",
                v.as_str().chars().take(240).collect::<String>()
            );
        }
    }
    gs.update(resp).expect("apply update");
    println!("action={:?}", gs.tavern.current_action);
    println!("expeditions={:?}", gs.tavern.expeditions);
}
