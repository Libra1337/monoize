use monoize::store_billing::money::{
    Currency, ExchangeRateRational, cny_fen_to_nano_usd, convert_minor_rational,
    nano_usd_to_cny_fen,
};

#[test]
fn decimal_rate_is_reduced_to_one_canonical_rational() {
    let rate = ExchangeRateRational::parse("6.7370").unwrap();
    assert_eq!(rate.numerator(), 6_737);
    assert_eq!(rate.denominator(), 1_000);
    assert_eq!(rate.decimal(), "6.7370");
}

#[test]
fn rational_conversion_rounds_once_at_the_target_minor_unit() {
    let rate = ExchangeRateRational::parse("6.7370").unwrap();
    assert_eq!(
        convert_minor_rational(100, Currency::USD, Currency::CNY, &rate).unwrap(),
        674
    );
    assert_eq!(
        convert_minor_rational(6_737, Currency::CNY, Currency::USD, &rate).unwrap(),
        1_000
    );
}

#[test]
fn cny_and_nano_usd_use_the_same_saved_generation() {
    let rate = ExchangeRateRational::parse("6.7370").unwrap();
    assert_eq!(cny_fen_to_nano_usd(6_737, &rate).unwrap(), 10_000_000_000);
    assert_eq!(nano_usd_to_cny_fen(10_000_000_000, &rate).unwrap(), 6_737);
}

#[test]
fn rational_parser_rejects_noncanonical_or_excessive_rates() {
    for invalid in [
        "",
        "0",
        "00.1",
        ".5",
        "1.",
        "+1",
        "1e2",
        "1.1234567890123456789",
    ] {
        assert!(
            ExchangeRateRational::parse(invalid).is_err(),
            "accepted {invalid}"
        );
    }
}

#[test]
fn rational_arithmetic_rejects_intermediate_overflow() {
    let rate = ExchangeRateRational::parse("20").unwrap();
    assert!(convert_minor_rational(i128::MAX, Currency::USD, Currency::CNY, &rate).is_err());
}
