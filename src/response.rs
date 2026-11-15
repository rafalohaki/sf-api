// TODO: Remove this once ouroboros is updated:
// https://github.com/someguynamedjosh/ouroboros/issues/140
#![allow(clippy::extra_unused_lifetimes)]

use std::{collections::HashMap, fmt::Debug, str::FromStr};

use chrono::NaiveDateTime;
use log::{debug, error, trace, warn};

use crate::error::SFError;

/// A bunch of new information about the state of the server and/or the
/// player
///
/// NOTE: This has a weird syntax to access, because we do not want to create
/// 10000 strings on each request and instead just store the raw response body
/// and references into it. This is faster & uses less memory, but because of
/// rusts borrow checker requires some weird syntax here.
// Technically we could do this safely with an iterator, that parses on demand,
// but send_command() needs to access specific response keys to keep the session
// running, which means a HashMap needs to be constructed no matter what
#[ouroboros::self_referencing]
pub struct Response {
    body: String,
    #[borrows(body)]
    #[covariant]
    resp: HashMap<&'this str, ResponseVal<'this>>,
    /// We store this to make sure the time calculations are still correct, if
    /// this response is held any amount of time before being used to update
    /// character state
    received_at: NaiveDateTime,
}

impl Clone for Response {
    // This is not a good clone..
    #[allow(clippy::expect_used)]
    fn clone(&self) -> Self {
        Self::parse(self.raw_response().to_string(), self.received_at())
            .expect("Invalid response cloned")
    }
}

impl Debug for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Debug output is routinely used for logging, so credential values
        // that arrive in every response body must be redacted here. The raw
        // values stay available through `values()` and `raw_response()`.
        f.debug_map()
            .entries(self.values().iter().map(|(key, val)| {
                let value = if is_credential_key(key) {
                    "[REDACTED]"
                } else {
                    val.as_str()
                };
                (*key, value)
            }))
            .finish()
    }
}

/// Returns `true` if the response key carries a credential issued during
/// login, whose value must never be written to logs or debug output.
fn is_credential_key(key: &str) -> bool {
    key.eq_ignore_ascii_case("sessionid")
        || key.eq_ignore_ascii_case("cryptoid")
        || key.eq_ignore_ascii_case("cryptokey")
}

#[cfg(feature = "serde")]
impl serde::Serialize for Response {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("Response", 2)?;
        s.serialize_field("body", self.borrow_body())?;
        s.serialize_field("received_at", &self.received_at())?;
        s.end()
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Response {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct AVisitor;

        impl<'de> serde::de::Visitor<'de> for AVisitor {
            type Value = Response;

            fn expecting(
                &self,
                formatter: &mut std::fmt::Formatter,
            ) -> std::fmt::Result {
                formatter.write_str(
                    "struct Response with fields body and received_at",
                )
            }

            fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
            where
                V: serde::de::MapAccess<'de>,
            {
                let mut body = None;
                let mut received_at = None;

                while let Some(key) = map.next_key()? {
                    match key {
                        "body" => {
                            body = Some(map.next_value()?);
                        }
                        "received_at" => {
                            received_at = Some(map.next_value()?);
                        }
                        _ => {
                            // Ignore unknown fields
                            map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }

                let body: String = body
                    .ok_or_else(|| serde::de::Error::missing_field("body"))?;
                let received_at: NaiveDateTime =
                    received_at.ok_or_else(|| {
                        serde::de::Error::missing_field("received_at")
                    })?;

                Response::parse(body, received_at).map_err(|_| {
                    serde::de::Error::custom("invalid response body")
                })
            }
        }

        deserializer.deserialize_struct(
            "Response",
            &["body", "received_at"],
            AVisitor,
        )
    }
}

impl Response {
    /// Returns a reference to the hashmap, that contains mappings of response
    /// keys to values
    #[must_use]
    pub fn values(&self) -> &HashMap<&str, ResponseVal<'_>> {
        self.borrow_resp()
    }

    /// Returns the raw response from the server. This should only ever be
    /// necessary for debugging, caching, or in case there is ever a new
    /// response format in a response, that is not yet supported. You can of
    /// course also use this to look at how horrible the S&F encoding is..
    #[must_use]
    pub fn raw_response(&self) -> &str {
        self.borrow_body()
    }

    /// Returns the raw value of the client's `messages` response layer, if the
    /// server sent one.
    ///
    /// The 32.0 client parses this into typed messages (`Response.Messages` /
    /// `RawMessages`, `TryGetMessage`, `GetMessageType`, ...). The wire
    /// semantics of that layer were not established statically, so this
    /// accessor deliberately exposes the raw value only instead of guessing a
    /// message format.
    #[must_use]
    pub fn messages_raw(&self) -> Option<&str> {
        self.values().get("messages").map(ResponseVal::as_str)
    }

    /// Returns the raw value of the client's `rawmessages` response layer, if
    /// the server sent one.
    ///
    /// See [`Self::messages_raw`] for why this is raw.
    #[must_use]
    pub fn raw_messages_raw(&self) -> Option<&str> {
        self.values().get("rawmessages").map(ResponseVal::as_str)
    }

    /// Returns `true` if the response contains either of the client's message
    /// layers (`messages` or `rawmessages`).
    #[must_use]
    pub fn has_message_layer(&self) -> bool {
        self.messages_raw().is_some() || self.raw_messages_raw().is_some()
    }

    /// Returns the time, at which the response was received
    #[must_use]
    pub fn received_at(&self) -> NaiveDateTime {
        self.with_received_at(|a| *a)
    }

    /// Parses a response body from the server into a usable format
    /// You might want to use this, if you are analyzing responses from the
    /// browsers network tab. If you are trying to store/read responses to/from
    /// disk to cache them, or otherwise, you should use the sso feature to
    /// serialize/deserialize them instead
    ///
    /// # Errors
    /// - `ServerError`: If the server responsed with an error
    /// - `ParsingError`: If the response does not follow the standard S&F
    ///   server response schema
    pub fn parse(
        og_body: String,
        received_at: NaiveDateTime,
    ) -> Result<Response, SFError> {
        // We can not return from the closure below, so we have to do this work
        // twice (sadly)

        // NOTE: I think the trims might actually be completely unnecessary.
        // Pretty sure I mixed them up with command encoding, which is actually
        // '|' padded

        let body = og_body
            .trim_end_matches('|')
            .trim_start_matches(|a: char| !a.is_alphabetic());
        trace!("Received response: {} bytes", body.len());

        if !body.contains(':')
            && !body.starts_with("success")
            && !body.starts_with("Success")
        {
            // The raw body can contain credentials or other sensitive
            // server data; only report safe context here.
            return Err(SFError::ParsingError(
                "unexpected server response",
                format!("nonstandard response, {} bytes", body.len()),
            ));
        }

        if body.starts_with("error") || body.starts_with("Error") {
            let raw_error = body.split_once(':').unwrap_or_default().1;

            let error_msg = match raw_error {
                "adventure index must be 1-3" => "quest index must be 0-2",
                x => x,
            };

            return Err(SFError::ServerError(error_msg.to_string()));
        }

        let resp = ResponseBuilder {
            body: og_body,
            resp_builder: |body: &String| {
                let mut res = HashMap::new();
                for part in body
                    .trim_start_matches(|a: char| !a.is_alphabetic())
                    .trim_end_matches('|')
                    .split('&')
                    .filter(|a| !a.is_empty())
                {
                    let Some((full_key, value)) = part.split_once(':') else {
                        warn!(
                            "skipping malformed response part ({} bytes, no \
                             ':' separator)",
                            part.len()
                        );
                        continue;
                    };

                    let (key, sub_key) = match full_key.split_once('.') {
                        Some(x) => {
                            // full_key == key.subkey
                            x
                        }
                        None => {
                            if let Some((k, sk)) = full_key.split_once('(') {
                                // full_key == key(4)
                                (k, sk.trim_matches(')'))
                            } else {
                                // full_key == key
                                (full_key, "")
                            }
                        }
                    };
                    if key.is_empty() {
                        continue;
                    }

                    let old_val =
                        res.insert(key, ResponseVal { value, sub_key });
                    if let Some(old_val) = old_val {
                        debug!(
                            "Overwrote duplicate response key [{key}]: {} => \
                             {} bytes",
                            old_val.as_str().len(),
                            value.len()
                        );
                    }
                }
                res
            },
            received_at,
        }
        .build();

        Ok(resp)
    }
}

/// This is the raw &str, that the server sent as a value to some key. This
/// often requires extra conversions/parsing to use practically, so we associate
/// the most common parsing functions as methods to this data.
#[derive(Debug, Clone, Copy)]
#[allow(clippy::module_name_repetitions)]
pub struct ResponseVal<'a> {
    value: &'a str,
    sub_key: &'a str,
}

impl ResponseVal<'_> {
    /// Converts the response value into the required type
    ///
    /// # Errors
    /// If the response value can not be parsed into the output
    /// value, a `ParsingError` will be returned
    pub fn into<T: FromStr>(self, name: &'static str) -> Result<T, SFError> {
        self.value.trim().parse().map_err(|_| {
            error!("Could not convert {name} into target type: {self}");
            SFError::ParsingError(name, self.value.to_string())
        })
    }

    /// Converts the repsponse into a list, by splitting the raw value by '/'
    /// and converting each value into the required type. If any conversion
    /// fails, an error is returned
    ///
    /// # Errors
    /// If any of the values in the string can not be parsed into the output
    /// value, the `ParsingError` for that value will be returned
    pub fn into_list<T: FromStr>(
        self,
        name: &'static str,
    ) -> Result<Vec<T>, SFError> {
        let x = &self.value;
        if x.is_empty() {
            return Ok(Vec::new());
        }
        // Trimming ` ` & `\n` is not required. Might remove this later
        x.trim_matches(|a| ['/', ' ', '\n'].contains(&a))
            .split('/')
            .map(|c| {
                c.trim().parse::<T>().map_err(|_| {
                    error!(
                        "Could not convert {name} into list because of {c}: \
                         {self}"
                    );
                    SFError::ParsingError(name, format!("{c:?}"))
                })
            })
            .collect()
    }

    /// The way keys are parsed will trim some info from the string. The key for
    /// the player save `ownplayersave` is actually `ownplayersave.playerSave`.
    /// As this `.playerSave` is not relevant here and not in most cases, I
    /// decided to trim that off. More common, this is also just `s`, `r`, or a
    /// size hint like `(10)`. In some cases though, this information can be
    /// helpful for parsing. Thus, you can access it here
    #[must_use]
    pub fn sub_key(&self) -> &str {
        self.sub_key
    }

    /// Returns the raw reference to the internal &str, that the server sent
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.value
    }
}

impl std::fmt::Display for ResponseVal<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.value)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::{Response, ResponseVal};

    /// FIX-11: the client's message layers are exposed raw, because their wire
    /// semantics were not established statically.
    #[test]
    fn test_message_layer_accessors() {
        let body = "messages:hello&rawmessages:raw".to_string();
        let resp =
            Response::parse(body, chrono::Local::now().naive_local()).unwrap();
        assert_eq!(resp.messages_raw(), Some("hello"));
        assert_eq!(resp.raw_messages_raw(), Some("raw"));
        assert!(resp.has_message_layer());
    }

    #[test]
    fn test_missing_message_layer() {
        let body = "success:1".to_string();
        let resp =
            Response::parse(body, chrono::Local::now().naive_local()).unwrap();
        assert_eq!(resp.messages_raw(), None);
        assert!(!resp.has_message_layer());
    }

    /// Debug output must not leak the credential values that arrive in
    /// every response body, while the raw accessors still return them for
    /// real consumers.
    #[test]
    fn test_debug_redacts_credentials_but_raw_access_works() {
        let body = "success:1&sessionid:sekret-session-id&cryptoid:\
                    sekret-crypto-id&cryptokey:sekret-crypto-key&login count:5"
            .to_string();
        let resp =
            Response::parse(body, chrono::Local::now().naive_local()).unwrap();

        let debug_output = format!("{resp:?}");
        assert!(!debug_output.contains("sekret-session-id"));
        assert!(!debug_output.contains("sekret-crypto-id"));
        assert!(!debug_output.contains("sekret-crypto-key"));
        assert!(debug_output.contains("[REDACTED]"));
        // Non-credential fields stay useful for debugging.
        assert!(debug_output.contains("login count"));

        assert_eq!(
            resp.values().get("sessionid").map(ResponseVal::as_str),
            Some("sekret-session-id")
        );
        assert_eq!(
            resp.values().get("cryptoid").map(ResponseVal::as_str),
            Some("sekret-crypto-id")
        );
        assert_eq!(
            resp.values().get("cryptokey").map(ResponseVal::as_str),
            Some("sekret-crypto-key")
        );
    }

    /// The parsing error for nonstandard responses must not include the
    /// raw body, which can carry credentials or other sensitive text.
    #[test]
    fn test_malformed_response_error_hides_body() {
        let secret = "TOP-SECRET-BODY-CONTENT";
        let body = format!("nonsense without any separators {secret}");
        let error =
            Response::parse(body.clone(), chrono::Local::now().naive_local())
                .unwrap_err();

        let message = error.to_string();
        assert!(!message.contains(secret));
        assert!(!message.contains(&body));
        assert!(
            message.contains(&format!("{} bytes", body.len())),
            "error should carry safe context: {message}"
        );
    }
}
