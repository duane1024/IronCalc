#![allow(clippy::unwrap_used)]

use crate::{formatter::format::format_number, locale::get_locale};

fn fmt(value: f64, format: &str) -> String {
    format_number(value, format, get_locale("en").unwrap()).text
}

// A value below 1 that ROUNDS UP into the integer digit must carry, as Excel does:
// 0.98 shown to one decimal is 1.0, never 0.0 (Metro Bank's 2027 MOIC, 0.98, read "0.0x").
#[test]
fn a_value_below_one_carries_into_the_integer_digit() {
    assert_eq!(fmt(0.98, "0.0"), "1.0");
    assert_eq!(fmt(0.98, "0.0\\x"), "1.0x");
    assert_eq!(fmt(0.98, "0.0\"x\""), "1.0x");
    assert_eq!(fmt(0.98, "#,##0.0"), "1.0");
    assert_eq!(fmt(0.98, "$#,##0.0"), "$1.0");
    assert_eq!(fmt(0.996, "0.00"), "1.00");
    assert_eq!(fmt(0.9999, "0.0"), "1.0");
}

#[test]
fn a_value_below_one_that_does_not_carry_is_unchanged() {
    assert_eq!(fmt(0.94, "0.0"), "0.9");
    assert_eq!(fmt(0.04, "0.0"), "0.0");
    assert_eq!(fmt(0.096, "0.0"), "0.1");
    assert_eq!(fmt(0.0996, "0.00"), "0.10");
    assert_eq!(fmt(0.98, "0.00"), "0.98");
    assert_eq!(fmt(0.5, "0.0\\x"), "0.5x");
    assert_eq!(fmt(0.98, "0.0%"), "98.0%");
}

#[test]
fn values_of_one_and_above_are_unchanged() {
    assert_eq!(fmt(1.18, "0.0\\x"), "1.2x");
    assert_eq!(fmt(1.98, "0.0"), "2.0");
    assert_eq!(fmt(9.96, "0.0"), "10.0");
    assert_eq!(fmt(99.97, "0.0"), "100.0");
    assert_eq!(fmt(12.3, "0.0\\x"), "12.3x");
    assert_eq!(fmt(1234.5, "#,##0"), "1,235");
}

#[test]
fn a_negative_value_keeps_its_sign_when_its_digits_show() {
    assert_eq!(fmt(-0.98, "0.0"), "-1.0");
    assert_eq!(fmt(-0.98, "0"), "-1");
    assert_eq!(fmt(-0.98, "0.00"), "-0.98");
    assert_eq!(fmt(-1.5, "0.0"), "-1.5");
    assert_eq!(fmt(-0.98, "0.0;(0.0)"), "(1.0)");
}
