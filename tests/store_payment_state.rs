use monoize::store_billing::state_machine::{
    FulfillmentEvent, FulfillmentState, PaymentEvent, PaymentState, TransitionDecision,
    transition_fulfillment, transition_payment,
};

#[test]
fn verified_payment_applies_once_and_marks_version_two_late_payment() {
    assert_eq!(
        transition_payment(PaymentState::Unpaid, 2, PaymentEvent::VerifiedPayment),
        TransitionDecision::Apply {
            next: PaymentState::Paid,
            late_payment: false,
        }
    );
    assert_eq!(
        transition_payment(PaymentState::Closed, 2, PaymentEvent::VerifiedPayment),
        TransitionDecision::Apply {
            next: PaymentState::Paid,
            late_payment: true,
        }
    );
    assert_eq!(
        transition_payment(PaymentState::Paid, 2, PaymentEvent::VerifiedPayment),
        TransitionDecision::Noop
    );
}

#[test]
fn legacy_closed_order_rejects_new_payment_projection() {
    assert_eq!(
        transition_payment(PaymentState::Closed, 1, PaymentEvent::VerifiedPayment),
        TransitionDecision::Reject
    );
}

#[test]
fn payment_failure_never_downgrades_a_terminal_money_state() {
    assert_eq!(
        transition_payment(PaymentState::Unpaid, 2, PaymentEvent::ConfirmedUnpaid),
        TransitionDecision::Apply {
            next: PaymentState::Closed,
            late_payment: false,
        }
    );
    for state in [
        PaymentState::Paid,
        PaymentState::RefundPending,
        PaymentState::Refunded,
        PaymentState::Closed,
    ] {
        assert_eq!(
            transition_payment(state, 2, PaymentEvent::ConfirmedUnpaid),
            TransitionDecision::Noop
        );
    }
}

#[test]
fn refund_transitions_require_verified_results() {
    assert_eq!(
        transition_payment(PaymentState::Paid, 2, PaymentEvent::RefundReserved),
        TransitionDecision::Apply {
            next: PaymentState::RefundPending,
            late_payment: false,
        }
    );
    assert_eq!(
        transition_payment(PaymentState::RefundPending, 2, PaymentEvent::VerifiedRefund),
        TransitionDecision::Apply {
            next: PaymentState::Refunded,
            late_payment: false,
        }
    );
    assert_eq!(
        transition_payment(
            PaymentState::RefundPending,
            2,
            PaymentEvent::DefiniteRefundRejection
        ),
        TransitionDecision::Apply {
            next: PaymentState::Paid,
            late_payment: false,
        }
    );
    assert_eq!(
        transition_payment(PaymentState::Unpaid, 2, PaymentEvent::RefundReserved),
        TransitionDecision::Reject
    );
}

#[test]
fn fulfillment_can_retry_failed_work_but_never_reverse_success() {
    assert_eq!(
        transition_fulfillment(FulfillmentState::Pending, FulfillmentEvent::Succeeded),
        Some(FulfillmentState::Fulfilled)
    );
    assert_eq!(
        transition_fulfillment(FulfillmentState::Pending, FulfillmentEvent::Failed),
        Some(FulfillmentState::Failed)
    );
    assert_eq!(
        transition_fulfillment(FulfillmentState::Failed, FulfillmentEvent::Succeeded),
        Some(FulfillmentState::Fulfilled)
    );
    assert_eq!(
        transition_fulfillment(FulfillmentState::Fulfilled, FulfillmentEvent::Failed),
        None
    );
}
