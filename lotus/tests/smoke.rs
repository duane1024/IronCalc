//! End-to-end smoke test: load each bundled WK3 file and verify it produces
//! a non-empty workbook without errors. Cell-value comparisons against the
//! cached values stored inside FORMULACELL records are deferred — those
//! require a fully-fledged formula evaluation pass and are not in scope for
//! the v1 importer.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

use std::fs;
use std::path::PathBuf;

use ironcalc_lotus::base::cell::CellValue;
use ironcalc_lotus::load_from_wk3_bytes;

fn samples_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("wk3");
    p
}

#[test]
fn load_sales_wk3_into_workbook() {
    let path = samples_dir().join("SALES.WK3");
    let bytes = fs::read(&path).expect("read SALES.WK3");
    let model = load_from_wk3_bytes(&bytes, "SALES", "en", "UTC", "en").expect("load SALES.WK3");

    // The file has labels in column 0 starting at row 0 ("Sales report --").
    let label = match model.get_cell_value_by_index(0, 1, 1).expect("read A1") {
        CellValue::String(s) => s,
        other => panic!("expected string at SALES!A1, got {other:?}"),
    };
    assert!(
        label.starts_with("Sales report"),
        "unexpected A1: {label:?}"
    );

    // `Total hats sold` formula at WK3 row=5,col=2 → display C6. The formula
    // is B3+B4, with B3=5000 (women's hats) and B4=4000 (men's hats), so the
    // expected result is 9000.
    let mut model = model;
    model.evaluate();
    let computed = match model.get_cell_value_by_index(0, 6, 3).expect("read C6") {
        CellValue::Number(n) => n,
        other => panic!("expected number at SALES!C6, got {other:?}"),
    };
    assert!((computed - 9000.0).abs() < 1e-6, "C6 = {computed}");
}

#[test]
fn load_a_readme_wk3_into_workbook() {
    let path = samples_dir().join("A_README.WK3");
    let bytes = fs::read(&path).expect("read A_README.WK3");
    let model =
        load_from_wk3_bytes(&bytes, "README", "en", "UTC", "en").expect("load A_README.WK3");

    // A_README.WK3 puts its first label at B2 (row 2, col 2 in 1-based view).
    let b2 = match model.get_cell_value_by_index(0, 2, 2).expect("read B2") {
        CellValue::String(s) => s,
        other => panic!("expected string at B2, got {other:?}"),
    };
    assert!(b2.starts_with("Thank you"), "unexpected B2: {b2:?}");
}

#[test]
fn load_multish_wk3_three_sheets() {
    let path = samples_dir().join("MULTISH.WK3");
    let bytes = fs::read(&path).expect("read MULTISH.WK3");
    let model =
        load_from_wk3_bytes(&bytes, "MULTISH", "en", "UTC", "en").expect("load MULTISH.WK3");

    let sheet_names: Vec<&str> = model
        .workbook
        .worksheets
        .iter()
        .map(|w| w.name.as_str())
        .collect();
    assert_eq!(
        sheet_names,
        vec!["A", "B", "C"],
        "expected Lotus-style names A/B/C, got {sheet_names:?}"
    );

    // First label on each sheet ("Sheet A/B/C") at WK3 row=0,col=0 → display A1.
    let titles: Vec<String> = (0..3)
        .map(|s| match model.get_cell_value_by_index(s, 1, 1).unwrap() {
            CellValue::String(s) => s,
            other => panic!("sheet {s}!A1 expected string, got {other:?}"),
        })
        .collect();
    assert_eq!(titles, vec!["Sheet A", "Sheet B", "Sheet C"]);

    // The cell letters per sheet: tab=0 has A/B/C, tab=1 has D/E/F, tab=2 has G/H/I.
    let cells: Vec<String> = (0..3)
        .map(|s| match model.get_cell_value_by_index(s, 3, 1).unwrap() {
            CellValue::String(s) => s,
            other => panic!("sheet {s} A3 expected string, got {other:?}"),
        })
        .collect();
    assert_eq!(cells, vec!["A", "D", "G"]);
}

#[test]
fn load_sum1991s_wk3_into_workbook() {
    let path = samples_dir().join("SUM1991S.WK3");
    let bytes = fs::read(&path).expect("read SUM1991S.WK3");
    let model =
        load_from_wk3_bytes(&bytes, "SUM1991S", "en", "UTC", "en").expect("load SUM1991S.WK3");

    let a1 = match model.get_cell_value_by_index(0, 1, 1).expect("read A1") {
        CellValue::String(s) => s,
        other => panic!("expected string at A1, got {other:?}"),
    };
    assert!(a1.starts_with("INCOME SUMMARY"), "unexpected A1: {a1:?}");
}
