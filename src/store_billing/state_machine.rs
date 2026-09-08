use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentState {
    Unpaid,
    Paid,
    RefundPending,
    Refunded,
    Closed,
}

impl PaymentState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unpaid => "unpaid",
            Self::Paid => "paid",
            Self::RefundPending => "refund_pending",
            Self::Refunded => "refunded",
            Self::Closed => "closed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FulfillmentState {
    Pending,
    Fulfilled,
    Failed,
}

impl FulfillmentState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Fulfilled => "fulfilled",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaymentEvent {
    VerifiedPayment,
    ConfirmedUnpaid,
    RefundReserved,
    VerifiedRefund,
    DefiniteRefundRejection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FulfillmentEvent {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionDecision {
    Apply {
        next: PaymentState,
        late_payment: bool,
    },
    Noop,
    Reject,
}

pub const fn transition_payment(
    current: PaymentState,
    contract_version: i32,
    event: PaymentEvent,
) -> TransitionDecision {
    match (current, event) {
        (PaymentState::Unpaid, PaymentEvent::VerifiedPayment) => TransitionDecision::Apply {
            next: PaymentState::Paid,
            late_payment: false,
        },
        (PaymentState::Closed, PaymentEvent::VerifiedPayment) if contract_version == 2 => {
            TransitionDecision::Apply {
                next: PaymentState::Paid,
                late_payment: true,
            }
        }
        (PaymentState::Closed, PaymentEvent::VerifiedPayment) => TransitionDecision::Reject,
        (
            PaymentState::Paid | PaymentState::RefundPending | PaymentState::Refunded,
            PaymentEvent::VerifiedPayment,
        ) => TransitionDecision::Noop,
        (PaymentState::Unpaid, PaymentEvent::ConfirmedUnpaid) => TransitionDecision::Apply {
            next: PaymentState::Closed,
            late_payment: false,
        },
        (_, PaymentEvent::ConfirmedUnpaid) => TransitionDecision::Noop,
        (PaymentState::Paid, PaymentEvent::RefundReserved) => TransitionDecision::Apply {
            next: PaymentState::RefundPending,
            late_payment: false,
        },
        (PaymentState::RefundPending, PaymentEvent::RefundReserved) => TransitionDecision::Noop,
        (PaymentState::RefundPending, PaymentEvent::VerifiedRefund) => TransitionDecision::Apply {
            next: PaymentState::Refunded,
            late_payment: false,
        },
        (PaymentState::Refunded, PaymentEvent::VerifiedRefund) => TransitionDecision::Noop,
        (PaymentState::RefundPending, PaymentEvent::DefiniteRefundRejection) => {
            TransitionDecision::Apply {
                next: PaymentState::Paid,
                late_payment: false,
            }
        }
        (PaymentState::Paid, PaymentEvent::DefiniteRefundRejection) => TransitionDecision::Noop,
        _ => TransitionDecision::Reject,
    }
}

pub const fn transition_fulfillment(
    current: FulfillmentState,
    event: FulfillmentEvent,
) -> Option<FulfillmentState> {
    match (current, event) {
        (FulfillmentState::Pending, FulfillmentEvent::Succeeded)
        | (FulfillmentState::Failed, FulfillmentEvent::Succeeded) => {
            Some(FulfillmentState::Fulfilled)
        }
        (FulfillmentState::Pending, FulfillmentEvent::Failed) => Some(FulfillmentState::Failed),
        (FulfillmentState::Fulfilled, FulfillmentEvent::Succeeded) => {
            Some(FulfillmentState::Fulfilled)
        }
        (FulfillmentState::Failed, FulfillmentEvent::Failed) => Some(FulfillmentState::Failed),
        (FulfillmentState::Fulfilled, FulfillmentEvent::Failed) => None,
    }
}
