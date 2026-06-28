#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

use std::io::Cursor;

use ironcalc::export::save_xlsx_to_writer;
use ironcalc::import::load_from_xlsx_bytes;
use ironcalc_base::Model;

#[test]
fn calc_properties_round_trip_through_xlsx() {
    // Build a model with iterative calculation enabled, export to xlsx, then
    // re-import: the <calcPr> settings must survive the round trip.
    let mut model = Model::new_empty("iter", "en", "UTC", "en").unwrap();
    model.set_iterative_calculation(true, Some(250), Some(0.0001));

    let writer = save_xlsx_to_writer(&model, Cursor::new(Vec::new())).unwrap();
    let bytes = writer.into_inner();

    let workbook = load_from_xlsx_bytes(&bytes, "iter", "en", "UTC").unwrap();
    let model2 = Model::from_workbook(workbook, "en").unwrap();

    let properties = model2.get_iterative_calculation();
    assert!(properties.iterate);
    assert_eq!(properties.iterate_count, 250);
    assert_eq!(properties.iterate_delta, 0.0001);
}

#[test]
fn calc_pr_absent_defaults_to_no_iteration() {
    // A model with iteration disabled exports a bare <calcPr/> and re-imports
    // with iteration off (defaults).
    let model = Model::new_empty("plain", "en", "UTC", "en").unwrap();
    let writer = save_xlsx_to_writer(&model, Cursor::new(Vec::new())).unwrap();
    let bytes = writer.into_inner();

    let workbook = load_from_xlsx_bytes(&bytes, "plain", "en", "UTC").unwrap();
    let model2 = Model::from_workbook(workbook, "en").unwrap();

    assert!(!model2.get_iterative_calculation().iterate);
}
