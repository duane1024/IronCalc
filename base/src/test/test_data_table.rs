#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::{cell::CellValue, types::DataTable, Model};

fn number_at(model: &Model, reference: &str) -> f64 {
    match model.get_cell_value_by_ref(reference).unwrap() {
        CellValue::Number(value) => value,
        other => panic!("{reference} is not a number: {other:?}"),
    }
}

#[test]
fn evaluates_one_variable_column_data_table() {
    let mut model = Model::new_empty("model", "en", "UTC", "en").unwrap();
    model.set_user_input(0, 1, 2, "0".to_string()).unwrap();
    model.set_user_input(0, 2, 3, "=B1*2".to_string()).unwrap();
    model.set_user_input(0, 2, 4, "=B1*3".to_string()).unwrap();
    for (row, value) in [(3, "1"), (4, "2"), (5, "3")] {
        model.set_user_input(0, row, 2, value.to_string()).unwrap();
        model.set_user_input(0, row, 3, "999".to_string()).unwrap();
        model.set_user_input(0, row, 4, "999".to_string()).unwrap();
    }
    model.set_user_input(0, 3, 5, "=C3+D3".to_string()).unwrap();
    model.workbook.worksheets[0].data_tables.push(DataTable {
        range: "C3:D5".to_string(),
        two_dimensional: false,
        row_oriented: false,
        r1: "B1".to_string(),
        r2: None,
        calculate_always: true,
    });

    model.evaluate();

    assert_eq!(number_at(&model, "Sheet1!C3"), 2.0);
    assert_eq!(number_at(&model, "Sheet1!D3"), 3.0);
    assert_eq!(number_at(&model, "Sheet1!C5"), 6.0);
    assert_eq!(number_at(&model, "Sheet1!D5"), 9.0);
    assert_eq!(number_at(&model, "Sheet1!E3"), 5.0);
    assert_eq!(number_at(&model, "Sheet1!B1"), 0.0);
}

#[test]
fn evaluates_one_variable_row_data_table() {
    let mut model = Model::new_empty("model", "en", "UTC", "en").unwrap();
    model.set_user_input(0, 1, 2, "0".to_string()).unwrap();
    model.set_user_input(0, 3, 2, "=B1*2".to_string()).unwrap();
    model.set_user_input(0, 4, 2, "=B1*3".to_string()).unwrap();
    for (column, value) in [(3, "1"), (4, "2"), (5, "3")] {
        model
            .set_user_input(0, 2, column, value.to_string())
            .unwrap();
        model
            .set_user_input(0, 3, column, "999".to_string())
            .unwrap();
        model
            .set_user_input(0, 4, column, "999".to_string())
            .unwrap();
    }
    model.workbook.worksheets[0].data_tables.push(DataTable {
        range: "C3:E4".to_string(),
        two_dimensional: false,
        row_oriented: true,
        r1: "B1".to_string(),
        r2: None,
        calculate_always: true,
    });

    model.evaluate();

    assert_eq!(number_at(&model, "Sheet1!C3"), 2.0);
    assert_eq!(number_at(&model, "Sheet1!E3"), 6.0);
    assert_eq!(number_at(&model, "Sheet1!C4"), 3.0);
    assert_eq!(number_at(&model, "Sheet1!E4"), 9.0);
    assert_eq!(number_at(&model, "Sheet1!B1"), 0.0);
}

#[test]
fn evaluates_two_variable_data_table() {
    let mut model = Model::new_empty("model", "en", "UTC", "en").unwrap();
    model.set_user_input(0, 1, 1, "0".to_string()).unwrap();
    model.set_user_input(0, 1, 2, "0".to_string()).unwrap();
    model.set_user_input(0, 2, 3, "=A1+B1".to_string()).unwrap();
    model.set_user_input(0, 2, 4, "10".to_string()).unwrap();
    model.set_user_input(0, 2, 5, "20".to_string()).unwrap();
    model.set_user_input(0, 3, 3, "1".to_string()).unwrap();
    model.set_user_input(0, 4, 3, "2".to_string()).unwrap();
    for row in 3..=4 {
        for column in 4..=5 {
            model
                .set_user_input(0, row, column, "999".to_string())
                .unwrap();
        }
    }
    model.workbook.worksheets[0].data_tables.push(DataTable {
        range: "D3:E4".to_string(),
        two_dimensional: true,
        row_oriented: true,
        r1: "A1".to_string(),
        r2: Some("B1".to_string()),
        calculate_always: true,
    });

    model.evaluate();

    assert_eq!(number_at(&model, "Sheet1!D3"), 11.0);
    assert_eq!(number_at(&model, "Sheet1!E3"), 21.0);
    assert_eq!(number_at(&model, "Sheet1!D4"), 12.0);
    assert_eq!(number_at(&model, "Sheet1!E4"), 22.0);
    assert_eq!(number_at(&model, "Sheet1!A1"), 0.0);
    assert_eq!(number_at(&model, "Sheet1!B1"), 0.0);
}
