//! Models for the S&F 32.0 shop / payment request layer.
//!
//! The names below are taken from the official client metadata:
//! `GetShopCatalogRequest` (type 5766), `ShopCheckoutRequest` (5767),
//! `ShopInitRequest` (5768) and the `AppStore` enum (5682), see
//! `docs/re/sf-client-protocol.md` §4.
//!
//! **Limitation:** the metadata dump provides field *names* and declaration
//! order, but not the `ToString` bodies. Therefore the wire order, separators
//! and the encoding of `AppStore` used by [`crate::command::Command`] are
//! best-effort and must be verified against a live server (or IL) before
//! relying on them.

/// Platform identifier used by the client's shop/payment requests
/// (`AppStore`, client metadata type 5682).
///
/// The numeric values follow the declaration order observed in the metadata.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, num_derive::FromPrimitive,
)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[allow(missing_docs)]
pub enum AppStore {
    /// No store / not specified
    #[default]
    None = 0,
    /// Google Play / Android
    Android,
    /// Apple App Store. The client spells this `iOS`.
    #[allow(non_camel_case_types)]
    iOS,
    /// Steam
    Steam,
}

/// Fields of the client's `GetShopCatalogRequest` (type 5766).
///
/// The `onRequestCompleted` / `onRequestFailed` callbacks of the client class
/// are not part of the wire request and are intentionally not modelled.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GetShopCatalogRequest {
    /// The store platform the client reports
    pub platform: AppStore,
    /// Whether the blackmarket offers should be included
    pub blackmarket_offers: bool,
}

/// Fields of the client's `ShopInitRequest` (type 5768).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ShopInitRequest {
    /// The store platform the client reports
    pub platform: AppStore,
    /// The product id of the item to buy
    pub product_id: String,
    /// The currency of the purchase
    pub currency: String,
    /// The human readable product description
    pub description: String,
    /// The id of the buying user
    pub user_id: String,
}

/// Fields of the client's `ShopCheckoutRequest` (type 5767).
#[derive(Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ShopCheckoutRequest {
    /// The store platform the client reports
    pub platform: AppStore,
    /// The product id of the item to buy
    pub product_id: String,
    /// The locale of the purchase
    pub locale: String,
    /// The human readable product description
    pub description: String,
    /// Whether the purchase was free or paid with in-game currency
    pub free_or_payed_with_in_game_currency: bool,
    /// The store signature of the purchase
    ///
    /// **Secret**: a payment credential. Deliberately omitted from
    /// [`std::fmt::Debug`] so `trace!("Sending a {command:?}")` and any
    /// `{:?}` of a session/command cannot leak it into logs.
    pub signature: String,
    /// The store transaction uuid
    pub transaction_uuid: String,
    /// The currency of the purchase
    pub currency: String,
    /// The iOS storefront location (only used by the iOS store)
    pub ios_storefront_location: String,
}

impl std::fmt::Debug for ShopCheckoutRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Payment-credential redaction: the signature is a usable secret; the
        // transaction uuid identifies the purchase and is redacted with it.
        f.debug_struct("ShopCheckoutRequest")
            .field("platform", &self.platform)
            .field("product_id", &self.product_id)
            .field("locale", &self.locale)
            .field("description", &self.description)
            .field(
                "free_or_payed_with_in_game_currency",
                &self.free_or_payed_with_in_game_currency,
            )
            .field("signature", &"[redacted]")
            .field("transaction_uuid", &"[redacted]")
            .field("currency", &self.currency)
            .field("ios_storefront_location", &self.ios_storefront_location)
            .finish()
    }
}
