//! # ViewPlayer diagnostic (login + one lookup only)
//!
//! Prints the fortress attack target of the logged-in character, then tries
//! `ViewPlayer` by name (control) and by that player id, printing the raw
//! server error keys/values so lookup failures are diagnosable.
//!
//! ```bash
//! SF_USER=… SF_PW=… SF_SERVER=… [SF_IDENT=…] cargo run --example view_player
//! ```

use sf_api::{command::Command, session::SimpleSession};

#[tokio::main]
async fn main() {
    let user = std::env::var("SF_USER").expect("SF_USER env missing");
    let pw = std::env::var("SF_PW").expect("SF_PW env missing");
    let server = std::env::var("SF_SERVER").expect("SF_SERVER env missing");

    let mut session = SimpleSession::login(&user, &pw, &server)
        .await
        .expect("login");
    let gs = session.game_state().expect("gs");
    let target = gs.fortress.as_ref().and_then(|f| f.attack_target);
    eprintln!("attack_target: {target:?}");

    for ident in [
        std::env::var("SF_IDENT").unwrap_or_else(|_| gs.character.name.clone()),
        target.map(|p| p.to_string()).unwrap_or_default(),
    ] {
        if ident.is_empty() {
            continue;
        }
        eprintln!("ViewPlayer ident={ident:?} …");
        match session
            .send_command(Command::ViewPlayer {
                ident: ident.clone(),
            })
            .await
        {
            Ok(_) => println!("OK ident={ident:?}"),
            Err(e) => println!("ERR ident={ident:?}: {e}"),
        }
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    }
}
