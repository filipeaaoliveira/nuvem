use nuvem_domain::gcp::money::{Money, MoneyError, MoneyTotal};

/// google.type.Money carries the sign on BOTH fields: -$1.75 is
/// units=-1, nanos=-750000000. Reconstructing through f64 loses cents at
/// scale, so the whole type is i128 nanos.
#[test]
fn a_negative_amount_carries_the_sign_on_both_fields() {
    let money = Money::parse("-97", -500_000_000, "USD").expect("parses");
    assert_eq!(money.nanos, -97_500_000_000);
    assert_eq!(money.currency, "USD");
}

#[test]
fn a_whole_amount_has_zero_nanos() {
    assert_eq!(
        Money::parse("40", 0, "EUR").expect("parses").nanos,
        40_000_000_000
    );
}

/// The API documents negative cost as savings. A savings card negates once,
/// here, so no render site has to remember the convention.
#[test]
fn negating_a_saving_makes_it_positive() {
    let saving = Money::parse("-97", -500_000_000, "USD")
        .expect("parses")
        .negate();
    assert_eq!(saving.nanos, 97_500_000_000);
}

/// units is an int64 encoded as a JSON string. A non-numeric one is a schema
/// change, not a zero.
#[test]
fn a_non_numeric_units_field_is_an_error_not_a_zero() {
    assert!(matches!(
        Money::parse("", 0, "USD"),
        Err(MoneyError::Units(_))
    ));
    assert!(matches!(
        Money::parse("1.5", 0, "USD"),
        Err(MoneyError::Units(_))
    ));
}

/// costProjection.duration is per-recommendation. Monthly-ness is a
/// derivation, and a zero denominator is refused rather than divided by.
#[test]
fn scaling_to_a_month_divides_by_the_projections_own_duration() {
    let fortnight = Money::parse("-50", 0, "USD").expect("parses");
    let monthly = fortnight
        .clone()
        .scaled(2_592_000, 1_296_000)
        .expect("scales");
    assert_eq!(monthly.nanos, -100_000_000_000);
    assert_eq!(fortnight.scaled(2_592_000, 0), None);
}

/// Totals are per-currency. Converting needs a rate the app does not have, so
/// two currencies produce two subtotals rather than one wrong number.
#[test]
fn a_total_never_adds_two_currencies_together() {
    let mut total = MoneyTotal::default();
    total.add(Money::parse("10", 0, "USD").expect("parses"));
    total.add(Money::parse("5", 500_000_000, "USD").expect("parses"));
    total.add(Money::parse("7", 0, "EUR").expect("parses"));

    let mut parts = total.into_parts();
    parts.sort_by(|a, b| a.currency.cmp(&b.currency));
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].currency, "EUR");
    assert_eq!(parts[0].nanos, 7_000_000_000);
    assert_eq!(parts[1].currency, "USD");
    assert_eq!(parts[1].nanos, 15_500_000_000);
}

/// Round once, at the end. Summing rounded values is how a card's total
/// disagrees with the sum of its own rows.
#[test]
fn rendering_rounds_once_at_the_end() {
    let mut total = MoneyTotal::default();
    for _ in 0..3 {
        total.add(Money::parse("0", 400_000_000, "USD").expect("parses"));
    }
    let parts = total.into_parts();
    assert_eq!(parts[0].render(), "USD 1.20");
}

/// The sub-cent tail rounds rather than truncating, in both directions and on
/// both signs. Truncation would be systematically kind to savings figures:
/// every card would understate by up to a cent per row, always in the same
/// direction, and the doc comment on `render` would be describing something
/// the function does not do.
#[test]
fn rendering_rounds_the_sub_cent_tail_rather_than_dropping_it() {
    let round_up = Money::parse("1", 995_000_000, "USD").expect("parses");
    assert_eq!(round_up.render(), "USD 2.00");

    let round_down = Money::parse("1", 994_000_000, "USD").expect("parses");
    assert_eq!(round_down.render(), "USD 1.99");

    let negative_rounds_away_from_zero = Money::parse("-1", -995_000_000, "USD").expect("parses");
    assert_eq!(negative_rounds_away_from_zero.render(), "USD -2.00");
}

/// A total with no parts renders no subtotal rows at all, so the card needs to
/// distinguish "nothing added" from "added up to zero" — the second is a real
/// figure and the first is not.
#[test]
fn an_untouched_total_is_empty_and_one_with_a_zero_part_is_not() {
    let mut total = MoneyTotal::default();
    assert!(total.is_empty());

    total.add(Money::parse("0", 0, "USD").expect("parses"));
    assert!(!total.is_empty());
}

/// Zero is not negative. A sign taken from `<= 0` renders "USD -0.00" for a
/// card whose recommendations all netted out, which reads as a loss.
#[test]
fn a_zero_amount_renders_without_a_minus_sign() {
    assert_eq!(
        Money::parse("0", 0, "USD").expect("parses").render(),
        "USD 0.00"
    );

    let netted_out = Money::parse("-1", 0, "EUR").expect("parses").negate();
    let mut total = MoneyTotal::default();
    total.add(netted_out);
    total.add(Money::parse("-1", 0, "EUR").expect("parses"));
    assert_eq!(total.into_parts()[0].render(), "EUR 0.00");
}
