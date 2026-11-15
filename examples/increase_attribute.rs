//! # Attribute increase probe (one attempt, diagnostic)
//!
//! Prints the attribute model (basis, times bought) before and after a single
//! `PlayerAttributIncrease{attr}/{times_bought+1}` attempt, so the allocation
//! semantics (free points vs silver purchase, refusal text) are measured on a
//! live server instead of guessed.
//!
//! ```bash
//! SF_USER=… SF_PW=… SF_SERVER=… [SF_ATTR=strength|dexterity|intelligence|constitution|luck] \
//!   cargo run --example increase_attribute
//! ```

use sf_api::command::{AttributeType, Command};
use sf_api::misc::EnumMapGet;
use sf_api::session::SimpleSession;

fn parse_attr(raw: &str) -> AttributeType {
    match raw.to_ascii_lowercase().as_str() {
        "strength" => AttributeType::Strength,
        "dexterity" => AttributeType::Dexterity,
        "intelligence" => AttributeType::Intelligence,
        "constitution" => AttributeType::Constitution,
        "luck" => AttributeType::Luck,
        other => panic!("unknown SF_ATTR {other:?}"),
    }
}

#[tokio::main]
async fn main() {
    let user = std::env::var("SF_USER").expect("SF_USER env missing");
    let pw = std::env::var("SF_PW").expect("SF_PW env missing");
    let server = std::env::var("SF_SERVER").expect("SF_SERVER env missing");

    let mut session = SimpleSession::login(&user, &pw, &server)
        .await
        .expect("login");
    let gs = session.game_state().expect("gs");
    let attr = std::env::var("SF_ATTR").ok().map_or_else(
        || gs.character.class.main_attribute(),
        |a| parse_attr(&a),
    );
    let before = *gs.character.attribute_times_bought.get(attr);
    let basis_before = *gs.character.attribute_basis.get(attr);
    eprintln!(
        "class={:?} attr={attr:?} times_bought={before} basis={basis_before} \
         silver={}",
        gs.character.class, gs.character.silver
    );

    let target = before + 1;
    match session
        .send_command(Command::IncreaseAttribute {
            attribute: attr,
            increase_to: target,
        })
        .await
    {
        Ok(gs) => {
            let after = *gs.character.attribute_times_bought.get(attr);
            let basis_after = *gs.character.attribute_basis.get(attr);
            println!(
                "OK increase_to={target}: times {before}→{after}, basis \
                 {basis_before}→{basis_after}, silver={}",
                gs.character.silver
            );
        }
        Err(e) => println!("ERR increase_to={target}: {e}"),
    }
}
