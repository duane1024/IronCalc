#![allow(clippy::unwrap_used, clippy::panic)]

use crate::test::user_model::util::new_empty_user_model;

// A one-variable column data table: input cell A1 (referenced by the governing
// formula B2 = A1*10), input values 1/2/3 down column A, results in B3:B5.
fn model_with_column_table_inputs() -> crate::UserModel<'static> {
    let mut model = new_empty_user_model();
    model.set_user_input(0, 1, 1, "0").unwrap(); // A1 (column input cell)
    model.set_user_input(0, 2, 2, "=A1*10").unwrap(); // B2 (governing formula)
    model.set_user_input(0, 3, 1, "1").unwrap(); // A3 input value
    model.set_user_input(0, 4, 1, "2").unwrap(); // A4
    model.set_user_input(0, 5, 1, "3").unwrap(); // A5
    model
}

#[test]
fn create_and_compute_column_data_table() {
    let mut model = model_with_column_table_inputs();
    model.set_data_table(0, "B3:B5", None, Some("A1")).unwrap();

    assert_eq!(model.get_formatted_cell_value(0, 3, 2).unwrap(), "10");
    assert_eq!(model.get_formatted_cell_value(0, 4, 2).unwrap(), "20");
    assert_eq!(model.get_formatted_cell_value(0, 5, 2).unwrap(), "30");

    // The table is discoverable from any cell in its body, but not elsewhere.
    assert!(model.get_data_table(0, 4, 2).unwrap().is_some());
    assert!(model.get_data_table(0, 1, 1).unwrap().is_none());
}

#[test]
fn undo_redo_create_data_table() {
    let mut model = model_with_column_table_inputs();
    model.set_data_table(0, "B3:B5", None, Some("A1")).unwrap();
    assert_eq!(model.get_formatted_cell_value(0, 3, 2).unwrap(), "10");

    // Undo removes the table and clears its (now orphaned) body.
    model.undo().unwrap();
    assert!(model.get_data_table(0, 3, 2).unwrap().is_none());
    assert_eq!(model.get_formatted_cell_value(0, 3, 2).unwrap(), "");

    // Redo re-creates and recomputes it.
    model.redo().unwrap();
    assert!(model.get_data_table(0, 3, 2).unwrap().is_some());
    assert_eq!(model.get_formatted_cell_value(0, 3, 2).unwrap(), "10");
}

#[test]
fn delete_data_table_and_undo() {
    let mut model = model_with_column_table_inputs();
    model.set_data_table(0, "B3:B5", None, Some("A1")).unwrap();

    // Delete via any cell inside the table; the body is cleared.
    model.delete_data_table(0, 4, 2).unwrap();
    assert!(model.get_data_table(0, 4, 2).unwrap().is_none());
    assert_eq!(model.get_formatted_cell_value(0, 4, 2).unwrap(), "");

    // Undo restores the table and recomputes the body.
    model.undo().unwrap();
    assert!(model.get_data_table(0, 4, 2).unwrap().is_some());
    assert_eq!(model.get_formatted_cell_value(0, 4, 2).unwrap(), "20");
}

#[test]
fn two_variable_inference_and_missing_inputs() {
    let mut model = new_empty_user_model();
    model.set_user_input(0, 1, 1, "0").unwrap(); // A1 (row input)
    model.set_user_input(0, 1, 2, "0").unwrap(); // B1 (column input)
    model
        .set_data_table(0, "C3:D4", Some("A1"), Some("B1"))
        .unwrap();
    let data_table = model.get_data_table(0, 3, 3).unwrap().unwrap();
    assert!(data_table.two_dimensional);
    assert_eq!(data_table.r1, "A1");
    assert_eq!(data_table.r2.as_deref(), Some("B1"));

    // Neither input cell provided is an error.
    assert!(model.set_data_table(0, "F3:F5", None, None).is_err());
}

#[test]
fn iterative_calculation_setting_undo_redo() {
    let mut model = new_empty_user_model();
    assert!(!model.get_iterative_calculation().iterate);

    model
        .set_iterative_calculation(true, Some(50), Some(0.01))
        .unwrap();
    assert!(model.get_iterative_calculation().iterate);
    assert_eq!(model.get_iterative_calculation().iterate_count, 50);

    model.undo().unwrap();
    assert!(!model.get_iterative_calculation().iterate);

    model.redo().unwrap();
    assert!(model.get_iterative_calculation().iterate);
    assert_eq!(model.get_iterative_calculation().iterate_count, 50);
}
