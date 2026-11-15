use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use chrono::{Local, NaiveDateTime};
use reqwest::{Client, header::*};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;
use url::Url;

use crate::{
    error::SFError,
    misc::sha1_hash,
    session::{
        ConnectionOptions, PWHash, Session, read_response_text, reqwest_client,
    },
};

#[derive(Debug)]
#[allow(dead_code)]
enum SSOAuthData {
    SF { pw_hash: PWHash },
    Google,
    Steam,
}

#[derive(Debug)]
pub struct SFAccount {
    pub(super) username: String,
    auth: SSOAuthData,
    pub(super) session: AccountSession,
    pub(super) client: Client,
    pub(super) options: ConnectionOptions,
}

#[derive(Clone, PartialEq, Eq)]
pub struct AccountSession {
    pub(super) uuid: String,
    pub(super) bearer_token: String,
}

/// The bearer token is a usable SSO credential and the uuid identifies the
/// account it belongs to; both are redacted, because `SFAccount`/`Session`
/// derive `Debug` and must not leak them into logs or panic messages. The
/// values themselves stay reachable through the fields.
impl std::fmt::Debug for AccountSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountSession")
            .field("uuid", &"[redacted]")
            .field("bearer_token", &"[redacted]")
            .finish()
    }
}

// This could just be a Login/Characters thing, but who knows what else will be
// integrated into this API. Maybe Register?
#[derive(Debug)]
enum APIRequest {
    Get,
    Post {
        parameters: Vec<&'static str>,
        form_data: HashMap<String, String>,
    },
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SSOCharacter {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) server_id: i32,
}
impl SFAccount {
    /// Returns the username of this S&F account
    #[must_use]
    pub fn username(&self) -> &str {
        &self.username
    }

    /// Initializes a `SFAccount` by logging the user in using the supplied
    /// clear text credentials
    ///
    /// # Errors
    /// May return basically every possible `SFError` variant, because we are
    /// both sending a command and parsing the result
    // TODO: Document what wring pw returns
    pub async fn login(
        username: String,
        password: String,
    ) -> Result<SFAccount, SFError> {
        Self::login_with_options(
            username,
            password,
            ConnectionOptions::default(),
        )
        .await
    }

    /// Initializes a `SFAccount` by logging the user in using the supplied
    /// clear text credentials and the provided options to use for the
    /// communication with the server
    ///
    /// # Errors
    /// May return basically every possible `SFError` variant, because we are
    /// both sending a command and parsing the result
    pub async fn login_with_options(
        username: String,
        password: String,
        options: ConnectionOptions,
    ) -> Result<SFAccount, SFError> {
        let pw_hash = PWHash::new(&password);
        Self::login_hashed_with_options(username, pw_hash, options).await
    }

    /// Initializes a `SFAccount` by logging the user in using the hashed
    /// password
    ///
    /// # Errors
    /// May return basically every possible `SFError` variant, because we are
    /// both sending a command and parsing the result
    pub async fn login_hashed(
        username: String,
        pw_hash: PWHash,
    ) -> Result<SFAccount, SFError> {
        Self::login_hashed_with_options(
            username,
            pw_hash,
            ConnectionOptions::default(),
        )
        .await
    }

    /// Initializes a `SFAccount` by logging the user in using the hashed
    /// password and the provided options to use for communication
    ///
    /// # Errors
    /// May return basically every possible `SFError` variant, because we are
    /// both sending a command and parsing the result
    pub async fn login_hashed_with_options(
        username: String,
        pw_hash: PWHash,
        options: ConnectionOptions,
    ) -> Result<SFAccount, SFError> {
        let mut tmp_self = Self {
            username,
            auth: SSOAuthData::SF { pw_hash },
            session: AccountSession {
                uuid: String::new(),
                bearer_token: String::new(),
            },
            client: reqwest_client(&options).ok_or(SFError::ConnectionError)?,
            options,
        };

        tmp_self.refresh_login().await?;
        Ok(tmp_self)
    }

    /// Refreshes the SSO session by logging in again with the stored
    /// credentials. This can be used when the server removed our session
    /// either for being connected too long, or the server was
    /// restarted/cache cleared and forgot us
    ///
    /// # Errors
    /// `InvalidRequest` if you call this with anything else then a SSO/SF
    /// account character. Even Google/Steam accounts will not work.
    pub async fn refresh_login(&mut self) -> Result<(), SFError> {
        let SSOAuthData::SF { pw_hash } = &self.auth else {
            // I do not think there is a way to reauth without going through
            // the SSO process again for these
            return Err(SFError::InvalidRequest(
                "Refreshing the SSO-login is only supported for SSO-Accounts",
            ));
        };

        let mut form_data = HashMap::new();
        form_data.insert("username".to_string(), self.username.clone());
        form_data.insert(
            "password".to_string(),
            sha1_hash(&(pw_hash.get().to_string() + "0")),
        );

        let res = self
            .send_api_request(
                "json/login",
                APIRequest::Post {
                    parameters: vec![
                        "client_id=i43nwwnmfc5tced4jtuk4auuygqghud2yopx",
                        "auth_type=access_token",
                    ],
                    form_data,
                },
            )
            .await?;

        self.session = parse_account_session(&res)?;

        Ok(())
    }

    /// Queries the SSO for all characters associated with this account. This
    /// consumes the Account, as the character sessions may need to refresh
    /// the accounts session, which they are only allowed to do, if they own it
    /// (in an Arc<Mutex<_>>) and there should be no need to keep the account
    /// around anyways
    ///
    /// # Errors
    /// May return `ParsingError` if the server changed its API, or
    /// `ConnectionError`, if the server could not be reached. The characters
    /// in the Vec may be `InvalidRequest`, iff the server the
    /// character would be on could not be determined
    pub async fn characters(
        self,
    ) -> Result<Vec<Result<Session, SFError>>, SFError> {
        // This could be passed in as an argument in case of multiple SSO
        // accounts to safe on requests, but I dont think people have multiple
        // and this is way easier
        let server_lookup =
            ServerLookup::fetch_with_client(&self.client).await?;
        let mut res = self
            .send_api_request("json/client/characters", APIRequest::Get)
            .await?;

        #[allow(clippy::indexing_slicing)]
        let characters: Vec<SSOCharacter> =
            serde_json::from_value(res["characters"].take()).map_err(|_| {
                SFError::ParsingError("missing json value ", String::new())
            })?;

        let account = Arc::new(Mutex::new(self));

        let mut chars = vec![];
        for char in characters {
            chars.push(
                Session::from_sso_char(char, account.clone(), &server_lookup)
                    .await,
            );
        }

        Ok(chars)
    }

    async fn send_api_request(
        &self,
        endpoint: &str,
        method: APIRequest,
    ) -> Result<Value, SFError> {
        send_api_request(
            &self.client,
            &self.session.bearer_token,
            endpoint,
            method,
        )
        .await
    }
}

/// Send a request to the SSO server. The endpoint will be "json/*". We try
/// to check if the response is bad in any way, but S&F responses never obey
/// to HTML status codes, or their own system, so good luck
#[allow(clippy::items_after_statements)]
async fn send_api_request(
    client: &Client,
    bearer_token: &str,
    endpoint: &str,
    method: APIRequest,
) -> Result<Value, SFError> {
    let mut url = url::Url::parse("https://sso.playa-games.com")
        .map_err(|_| SFError::ConnectionError)?;
    url.set_path(endpoint);

    let mut request = match method {
        APIRequest::Get => client.get(url.as_str()),
        APIRequest::Post {
            parameters,
            form_data,
        } => {
            url.set_query(Some(&parameters.join("&")));
            client.post(url.as_str()).form(&form_data)
        }
    };

    // Set all necessary header values to make our request succeed
    if !bearer_token.is_empty() {
        request = request.bearer_auth(bearer_token);
    }
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(
        REFERER,
        HeaderValue::from_str(url.authority())
            .map_err(|_| SFError::ConnectionError)?,
    );

    let res = request
        .headers(headers)
        .send()
        .await
        .map_err(|_| SFError::ConnectionError)?;
    if !res.status().is_success() {
        return Err(SFError::ConnectionError);
    }
    let text = read_response_text(res).await?;
    parse_api_response(&text)
}

fn parse_api_response(text: &str) -> Result<Value, SFError> {
    #[derive(Debug, Serialize, Deserialize)]
    struct APIResponse {
        success: bool,
        status: u8,
        data: Option<Value>,
        message: Option<Value>,
    }

    let resp: APIResponse = serde_json::from_str(text).map_err(|_| {
        SFError::ParsingError(
            "API response",
            format!("invalid envelope ({} bytes)", text.len()),
        )
    })?;

    if !resp.success {
        return Err(SFError::ConnectionError);
    }
    let data = match resp.data {
        Some(data) => data,
        None => match resp.message {
            Some(message) => message,
            None => return Err(SFError::ConnectionError),
        },
    };

    Ok(data)
}

#[derive(Debug, Clone)]
pub struct ServerLookup(HashMap<i32, Url>);

impl ServerLookup {
    /// Fetches the current mapping of numeric server ids to their URLs.
    ///
    /// # Errors
    /// Returns `ConnectionError`, if the server could not be reached, ro
    /// `ParsingError`, if the list could not be parsed
    pub async fn fetch() -> Result<ServerLookup, SFError> {
        let client = reqwest_client(&ConnectionOptions::default())
            .ok_or(SFError::ConnectionError)?;
        Self::fetch_with_client(&client).await
    }

    /// Fetches the current mapping of server ids to server URLs.
    #[allow(clippy::items_after_statements)]
    async fn fetch_with_client(
        client: &Client,
    ) -> Result<ServerLookup, SFError> {
        let response = client
            .get("https://sfgame.net/config.json")
            .send()
            .await
            .map_err(|_| SFError::ConnectionError)?;
        if !response.status().is_success() {
            return Err(SFError::ConnectionError);
        }
        let res = read_response_text(response).await?;

        #[derive(Debug, Deserialize, Serialize)]
        struct ServerResp {
            servers: Vec<ServerInfo>,
        }

        #[derive(Debug, Deserialize, Serialize)]
        struct ServerInfo {
            #[serde(rename = "i")]
            id: i32,
            #[serde(rename = "d")]
            url: String,
            #[serde(rename = "c")]
            country_code: String,
            #[serde(rename = "md")]
            merged_into: Option<String>,
            #[serde(rename = "m")]
            merge_date_time: Option<String>,
        }

        let resp: ServerResp = serde_json::from_str(&res).map_err(|_| {
            SFError::ParsingError("server response", res.clone())
        })?;

        let servers: HashMap<i32, Url> = resp
            .servers
            .into_iter()
            .filter_map(|s| {
                let mut server_url = s.url;
                if let Some(merged_url) = s.merged_into {
                    if let Some(mdt) = s.merge_date_time.and_then(|a| {
                        NaiveDateTime::parse_from_str(&a, "%Y-%m-%d %H:%M:%S")
                            .ok()
                    }) {
                        if Local::now().naive_utc() > mdt {
                            server_url = merged_url;
                        }
                    } else {
                        server_url = merged_url;
                    }
                }

                Some((s.id, format!("https://{server_url}").parse().ok()?))
            })
            .collect();
        if servers.is_empty() {
            return Err(SFError::ParsingError("empty server list", res));
        }

        Ok(ServerLookup(servers))
    }

    /// Gets the mapping of a server id to a URL
    ///
    /// # Errors
    /// Reutns `InvalidRequest` if there was no server with this id
    pub fn get(&self, server_id: i32) -> Result<Url, SFError> {
        self.0
            .get(&server_id)
            .cloned()
            .ok_or(SFError::InvalidRequest("There is no server with this id"))
    }

    /// Returns a set of all the servers, that are currently active, so no
    /// merged, or not yet available servers
    #[must_use]
    pub fn all(&self) -> HashSet<Url> {
        self.0.iter().map(|a| a.1.clone()).collect()
    }
}

#[derive(Debug)]
pub enum AuthResponse {
    Success(SFAccount),
    NoAuth(SSOAuth),
}

fn val_to_string(val: &Value) -> Option<String> {
    val.as_str().map(std::string::ToString::to_string)
}

fn parse_account_session(value: &Value) -> Result<AccountSession, SFError> {
    let bearer_token = value
        .get("token")
        .and_then(|token| token.get("access_token"))
        .and_then(val_to_string);
    let uuid = value
        .get("account")
        .and_then(|account| account.get("uuid"))
        .and_then(val_to_string);
    match (bearer_token, uuid) {
        (Some(bearer_token), Some(uuid)) => {
            Ok(AccountSession { uuid, bearer_token })
        }
        _ => Err(SFError::ParsingError(
            "missing auth value in api response",
            "required token/account fields missing".to_string(),
        )),
    }
}

pub struct SSOAuth {
    client: Client,
    options: ConnectionOptions,
    auth_url: Url,
    auth_id: String,
    provider: SSOProvider,
}

impl std::fmt::Debug for SSOAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SSOAuth")
            .field("options", &self.options)
            .field("auth_url", &"[redacted]")
            .field("auth_id", &"[redacted]")
            .field("provider", &self.provider)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum SSOProvider {
    Google,
    Steam,
}

impl SSOProvider {
    fn endpoint_ident(self) -> &'static str {
        match self {
            SSOProvider::Google => "googleauth",
            SSOProvider::Steam => "steamauth",
        }
    }
}

impl SSOAuth {
    async fn send_api_request(
        &self,
        endpoint: &str,
        method: APIRequest,
    ) -> Result<Value, SFError> {
        send_api_request(&self.client, "", endpoint, method).await
    }

    /// Returns the SSO auth URL, that the user has to login through
    #[must_use]
    pub fn auth_url(&self) -> &Url {
        &self.auth_url
    }

    /// Tries to login. If the user has successfully authenticated via the
    /// `auth_url`, this will return the normal `SFAccount`. Otherwise, this
    /// will return the existing Auth for you to reattempt the login after a
    /// few seconds
    ///
    /// # Errors
    /// May return `ConnectionError`, or `ParsingError` depending on what part
    /// of the communication failed
    #[allow(clippy::indexing_slicing)]
    pub async fn try_login(self) -> Result<AuthResponse, SFError> {
        let endpoint = format!(
            "/json/sso/{}/check/{}",
            self.provider.endpoint_ident(),
            self.auth_id
        );
        let resp = self.send_api_request(&endpoint, APIRequest::Get).await?;

        if let Some(message) = val_to_string(&resp) {
            return match message.as_str() {
                "SSO_POPUP_STATE_PROCESSING" => Ok(AuthResponse::NoAuth(self)),
                _ => Err(SFError::ConnectionError),
            };
        }

        let id_token =
            val_to_string(&resp["id_token"]).ok_or(SFError::ConnectionError)?;

        let mut form_data = HashMap::new();
        form_data.insert("token".to_string(), id_token.clone());
        form_data.insert("language".to_string(), "en".to_string());

        let res = self
            .send_api_request(
                &format!("json/login/sso/{}", self.provider.endpoint_ident()),
                APIRequest::Post {
                    parameters: vec![
                        "client_id=i43nwwnmfc5tced4jtuk4auuygqghud2yopx",
                        "auth_type=access_token",
                    ],
                    form_data,
                },
            )
            .await?;

        let session = parse_account_session(&res)?;
        let username = val_to_string(&res["account"]["username"])
            .ok_or(SFError::ConnectionError)?;

        Ok(AuthResponse::Success(SFAccount {
            username,
            client: self.client,
            session,
            options: self.options,
            auth: match self.provider {
                SSOProvider::Google => SSOAuthData::Google,
                SSOProvider::Steam => SSOAuthData::Steam,
            },
        }))
    }

    /// Instantiates a new attempt to login through a SSO provider. A user then
    /// has to interact with the `auth_url` this returns to validate the
    /// login. Afterwards you can login and transform this into a normal
    /// `SFAccount`
    ///
    /// # Errors
    /// May return `ConnectionError`, or `ParsingError` depending on what part
    /// of the communication failed
    pub async fn new(provider: SSOProvider) -> Result<Self, SFError> {
        Self::new_with_options(provider, ConnectionOptions::default()).await
    }

    /// The same as `new()`, but with optional connection options
    ///
    /// # Errors
    /// May return `ConnectionError`, or `ParsingError` depending on what part
    /// of the communication failed
    #[allow(clippy::indexing_slicing)]
    pub async fn new_with_options(
        provider: SSOProvider,
        options: ConnectionOptions,
    ) -> Result<Self, SFError> {
        let client =
            reqwest_client(&options).ok_or(SFError::ConnectionError)?;

        let resp = send_api_request(
            &client,
            "",
            &format!("json/sso/{}", provider.endpoint_ident()),
            APIRequest::Get,
        )
        .await?;

        let auth_url = val_to_string(&resp["redirect"])
            .and_then(|a| Url::parse(&a).ok())
            .ok_or(SFError::ConnectionError)?;
        let auth_id =
            val_to_string(&resp["id"]).ok_or(SFError::ConnectionError)?;
        Ok(Self {
            client,
            options,
            auth_url,
            auth_id,
            provider,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::json;

    use super::*;

    #[test]
    fn malformed_auth_responses_do_not_expose_partial_credentials() {
        let value = json!({
            "token": {
                "access_token": "SECRET-ACCESS-TOKEN",
                "refresh_token": "SECRET-REFRESH-TOKEN",
            },
            "account": {},
        });
        let error = parse_account_session(&value).unwrap_err();
        let printed = format!("{error} {error:?}");
        assert!(!printed.contains("SECRET-ACCESS-TOKEN"));
        assert!(!printed.contains("SECRET-REFRESH-TOKEN"));

        let envelope = r#"{"token":{"access_token":"SECRET-ACCESS-TOKEN"}}"#;
        let error = parse_api_response(envelope).unwrap_err();
        assert!(!format!("{error} {error:?}").contains("SECRET-ACCESS-TOKEN"));
    }

    #[test]
    fn valid_auth_envelope_preserves_credentials() {
        let value = parse_api_response(
            r#"{
            "success":true,"status":200,
            "data":{"token":{"access_token":"token"},"account":{"uuid":"id"}}
        }"#,
        )
        .unwrap();
        let session = parse_account_session(&value).unwrap();
        assert_eq!(session.bearer_token, "token");
        assert_eq!(session.uuid, "id");
    }

    #[test]
    fn pending_sso_debug_redacts_handshake_credentials() {
        let auth = SSOAuth {
            client: Client::new(),
            options: ConnectionOptions::default(),
            auth_url: Url::parse("https://example.com/?token=SECRET-URL")
                .unwrap(),
            auth_id: "SECRET-HANDSHAKE-ID".to_string(),
            provider: SSOProvider::Steam,
        };
        let printed = format!("{auth:?}");
        assert!(!printed.contains("SECRET-HANDSHAKE-ID"));
        assert!(!printed.contains("SECRET-URL"));
        assert!(printed.contains("Steam"));
    }
}
