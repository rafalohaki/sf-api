//! # Live SSO login diagnostic (offline-safe: login + character list only)
//!
//! Narzędzie diagnostyczne dla sf-next: loguje się do konta Playa (SSO),
//! pobiera listę postaci i drukuje je. **Nie wysyła żadnych komend gry** —
//! tylko endpointy SSO (json/login, json/client/characters). Zero zapisu
//! poświadczeń na dysk.
//!
//! Użycie (poświadczenia z env, nie z argumentów — nie lądują w shell
//! history Narzędzi):
//!
//! ```bash
//! SF_USER="..." SF_PW="..." cargo run --example sso_login
//! ```
//!
//! Ten sam łańcuch, który GUI robi jako „Login SSO” (login konta → lista
//! postaci → wybór świata per postać). Błędy (złe hasło, scalony serwer,
//! brak świata) wypisywane nazwanie — dokładnie to, co needujemy do
//! naprawy ścieżek logowania.

use sf_api::sso::SFAccount;

#[tokio::main]
async fn main() {
    let user = std::env::var("SF_USER").expect("SF_USER env missing");
    let pw = std::env::var("SF_PW").expect("SF_PW env missing");

    eprintln!("SSO login: {} …", user);
    let account = match SFAccount::login(user, pw).await {
        Ok(acc) => {
            eprintln!("✅ SSO login OK (konto: {})", acc.username());
            acc
        }
        Err(e) => {
            eprintln!("❌ SSO login FAILED: {e}");
            std::process::exit(1);
        }
    };

    eprintln!("Pobieram listę postaci …");
    let sessions = match account.characters().await {
        Ok(sessions) => sessions,
        Err(e) => {
            eprintln!("❌ character list FAILED: {e}");
            std::process::exit(1);
        }
    };

    let mut ok = 0usize;
    let mut failed = 0usize;
    for session in sessions {
        match session {
            Ok(s) => {
                ok += 1;
                let name = s.username();
                let url = s.server_url();
                println!("POSTAĆ: {name} @ {url}");
            }
            Err(e) => {
                failed += 1;
                println!("POSTAĆ: ❌ login nieudany — {e}");
            }
        }
    }
    println!("---");
    println!("zalogowane: {ok}, nieudane: {failed}");
}
