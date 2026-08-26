use super::money::Currency;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

macro_rules! string_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        pub enum $name {
            $(#[serde(rename = $value)] $variant),+
        }

        impl $name {
            pub(crate) const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value),+
                }
            }

            pub(crate) fn from_str(value: &str) -> Option<Self> {
                match value {
                    $($value => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

string_enum!(ProductKind {
    Balance => "balance",
    Plan => "plan",
});
string_enum!(PaymentChannelKind {
    Alipay => "alipay",
    Wechat => "wechat",
    Custom => "custom",
});
string_enum!(PaymentChannelMode {
    Redirect => "redirect",
    Qr => "qr",
    Manual => "manual",
});
string_enum!(IconKind {
    Builtin => "builtin",
    Url => "url",
    Upload => "upload",
});
string_enum!(WindowKind {
    FiveHours => "5h",
    TwelveHours => "12h",
    Day => "day",
    Week => "week",
    Month => "month",
    Custom => "custom",
});
string_enum!(OrderStatus {
    Pending => "pending",
    Completed => "completed",
    Cancelled => "cancelled",
});
string_enum!(RedemptionCodeStatus {
    Unused => "unused",
    Used => "used",
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalanceProductInput {
    pub recharge_minor: String,
    pub bonus_minor: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanQuotaInput {
    pub window_kind: WindowKind,
    pub window_seconds: i64,
    pub quota_fen_cny: String,
    pub sort_order: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateProductInput {
    pub kind: ProductKind,
    pub name: String,
    pub description: String,
    pub price_currency: Currency,
    pub price_minor: String,
    pub duration_seconds: Option<i64>,
    pub group_ids: Vec<String>,
    pub sort_order: i32,
    pub enabled: bool,
    pub balance: Option<BalanceProductInput>,
    pub quotas: Vec<PlanQuotaInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalanceProduct {
    pub recharge_minor: String,
    pub bonus_minor: String,
    pub actual_received_minor: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanQuota {
    pub id: String,
    pub window_kind: WindowKind,
    pub window_seconds: i64,
    pub quota_fen_cny: String,
    pub sort_order: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreProduct {
    pub id: String,
    pub kind: ProductKind,
    pub name: String,
    pub description: String,
    pub price_currency: Currency,
    pub price_minor: String,
    pub duration_seconds: Option<i64>,
    pub group_ids: Vec<String>,
    pub sort_order: i32,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub balance: Option<BalanceProduct>,
    pub quotas: Vec<PlanQuota>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatePaymentChannelInput {
    pub kind: PaymentChannelKind,
    pub name: String,
    pub mode: PaymentChannelMode,
    pub endpoint: Option<String>,
    pub icon_kind: IconKind,
    pub icon_value: Option<String>,
    pub config_secret: Option<String>,
    pub sort_order: i32,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdatePaymentChannelInput {
    pub kind: Option<PaymentChannelKind>,
    pub name: Option<String>,
    pub mode: Option<PaymentChannelMode>,
    pub endpoint: Option<String>,
    pub icon_kind: Option<IconKind>,
    pub icon_value: Option<String>,
    pub config_secret: Option<String>,
    pub sort_order: Option<i32>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentChannel {
    pub id: String,
    pub kind: PaymentChannelKind,
    pub name: String,
    pub mode: PaymentChannelMode,
    pub endpoint: Option<String>,
    pub icon_kind: IconKind,
    pub icon_value: Option<String>,
    pub sort_order: i32,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreCatalog {
    pub products: Vec<StoreProduct>,
    pub payment_channels: Vec<PaymentChannel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreSettings {
    pub custom_recharge_cny_min_minor: String,
    pub custom_recharge_cny_max_minor: String,
    pub custom_recharge_usd_min_minor: String,
    pub custom_recharge_usd_max_minor: String,
}

impl Default for StoreSettings {
    fn default() -> Self {
        Self {
            custom_recharge_cny_min_minor: "1000".to_string(),
            custom_recharge_cny_max_minor: "100000000".to_string(),
            custom_recharge_usd_min_minor: "1000".to_string(),
            custom_recharge_usd_max_minor: "100000000".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateOrderInput {
    pub product_id: String,
    pub payment_channel_id: String,
    pub payment_currency: Currency,
    pub custom_recharge_minor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductSnapshot {
    pub id: String,
    pub kind: ProductKind,
    pub name: String,
    pub description: String,
    pub price_currency: Currency,
    pub price_minor: String,
    pub duration_seconds: Option<i64>,
    pub group_ids: Vec<String>,
    pub balance: Option<BalanceProduct>,
    pub quotas: Vec<PlanQuota>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentChannelSnapshot {
    pub id: String,
    pub kind: PaymentChannelKind,
    pub name: String,
    pub mode: PaymentChannelMode,
    pub endpoint: Option<String>,
    pub icon_kind: IconKind,
    pub icon_value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderQuote {
    pub version: u32,
    pub product: ProductSnapshot,
    pub balance: Option<BalanceProduct>,
    pub payment_channel: PaymentChannelSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreOrder {
    pub id: String,
    pub order_number: String,
    pub user_id: String,
    pub product_id: String,
    pub product_kind: ProductKind,
    pub status: OrderStatus,
    pub payment_channel_id: String,
    pub payment_currency: Currency,
    pub payment_minor: String,
    pub cny_per_usd: String,
    pub rate_source_updated_at: DateTime<Utc>,
    pub quote: OrderQuote,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanEntitlement {
    pub id: String,
    pub user_id: String,
    pub product_id: String,
    pub product_name: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub cny_per_usd: String,
    pub group_ids: Vec<String>,
    pub quotas: Vec<PlanQuota>,
    pub source_kind: String,
    pub source_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum RedemptionRewardInput {
    Balance {
        currency: Currency,
        amount_minor: String,
    },
    Plan {
        product_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerateRedemptionCodesInput {
    pub reward: RedemptionRewardInput,
    pub count: u32,
    pub validity_days: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedemptionCodeRecord {
    pub id: String,
    pub code_hint: String,
    pub reward_kind: ProductKind,
    pub reward: serde_json::Value,
    pub status: RedemptionCodeStatus,
    pub expires_at: DateTime<Utc>,
    pub redeemed_by_user_id: Option<String>,
    pub redeemed_at: Option<DateTime<Utc>>,
    pub created_by_user_id: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedRedemptionCode {
    pub code: String,
    pub record: RedemptionCodeRecord,
}
