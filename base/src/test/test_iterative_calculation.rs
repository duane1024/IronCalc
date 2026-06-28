#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::cell::CellValue;
use crate::test::util::new_empty_model;
use crate::Model;

fn number(model: &Model, row: i32, column: i32) -> f64 {
    match model.get_cell_value_by_index(0, row, column).unwrap() {
        CellValue::Number(value) => value,
        other => panic!("expected a number at (row {row}, col {column}), got {other:?}"),
    }
}

#[test]
fn circular_reference_errors_without_iteration() {
    let mut model = new_empty_model();
    // A1 = B1/2 + 5, B1 = A1: a circular pair. Without iteration enabled this is
    // the existing behaviour -- both cells resolve to #CIRC!.
    model._set("A1", "=B1/2 + 5");
    model._set("B1", "=A1");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "#CIRC!");
    assert_eq!(model._get_text("B1"), "#CIRC!");
}

#[test]
fn convergent_pair_resolves_with_iteration() {
    let mut model = new_empty_model();
    model._set("A1", "=B1/2 + 5");
    model._set("B1", "=A1");
    model.set_iterative_calculation(true, Some(100), Some(0.001));
    model.evaluate();
    // Fixed point: A1 = A1/2 + 5  =>  A1 = 10, B1 = 10.
    assert!(
        (number(&model, 1, 1) - 10.0).abs() < 0.05,
        "A1 = {}",
        number(&model, 1, 1)
    );
    assert!(
        (number(&model, 1, 2) - 10.0).abs() < 0.05,
        "B1 = {}",
        number(&model, 1, 2)
    );
}

#[test]
fn circular_interest_model_converges() {
    // The canonical circular-finance pattern: a balance that includes interest
    // computed on itself. B1 = A1 + C1 (principal + interest), C1 = B1 * 0.1:
    //   B1 = 100 + 0.1*B1  =>  B1 = 1000/9 = 111.11..., C1 = 100/9 = 11.11...
    let mut model = new_empty_model();
    model._set("A1", "100");
    model._set("B1", "=A1 + C1");
    model._set("C1", "=B1 * 0.1");
    model.set_iterative_calculation(true, None, None);
    model.evaluate();
    assert!(
        (number(&model, 1, 2) - 1000.0 / 9.0).abs() < 0.01,
        "B1 = {}",
        number(&model, 1, 2)
    );
    assert!(
        (number(&model, 1, 3) - 100.0 / 9.0).abs() < 0.01,
        "C1 = {}",
        number(&model, 1, 3)
    );
}

#[test]
fn non_circular_model_is_unaffected_by_iteration() {
    // Enabling iteration must not change ordinary (acyclic) results.
    let mut model = new_empty_model();
    model._set("A1", "10");
    model._set("B1", "=A1 * 2");
    model._set("C1", "=B1 + 1");
    model.set_iterative_calculation(true, None, None);
    model.evaluate();
    assert_eq!(number(&model, 1, 2), 20.0);
    assert_eq!(number(&model, 1, 3), 21.0);
}

#[test]
fn iteration_settings_are_readable() {
    let mut model = new_empty_model();
    assert!(!model.get_iterative_calculation().iterate);
    model.set_iterative_calculation(true, Some(42), Some(0.5));
    let properties = model.get_iterative_calculation();
    assert!(properties.iterate);
    assert_eq!(properties.iterate_count, 42);
    assert_eq!(properties.iterate_delta, 0.5);
}
