use crate::types::{CalcProperties, DataTable};

use super::{common::UserModel, history::Diff};

/// Builds a [`DataTable`] descriptor from the authoring inputs. The orientation
/// is inferred from which input cells are supplied (see
/// [`UserModel::set_data_table`]).
fn build_data_table(
    range: &str,
    row_input_cell: Option<&str>,
    column_input_cell: Option<&str>,
) -> Result<DataTable, String> {
    match (row_input_cell, column_input_cell) {
        (Some(row_input), Some(column_input)) => Ok(DataTable {
            range: range.to_string(),
            two_dimensional: true,
            row_oriented: false,
            r1: row_input.to_string(),
            r2: Some(column_input.to_string()),
            calculate_always: false,
        }),
        (Some(row_input), None) => Ok(DataTable {
            range: range.to_string(),
            two_dimensional: false,
            row_oriented: true,
            r1: row_input.to_string(),
            r2: None,
            calculate_always: false,
        }),
        (None, Some(column_input)) => Ok(DataTable {
            range: range.to_string(),
            two_dimensional: false,
            row_oriented: false,
            r1: column_input.to_string(),
            r2: None,
            calculate_always: false,
        }),
        (None, None) => {
            Err("A data table needs a row input cell, a column input cell, or both".to_string())
        }
    }
}

impl UserModel<'_> {
    /// Returns the data table whose output range contains `(row, column)` on
    /// `sheet`, or `None`. Useful for edit-mode detection (Excel locks the body
    /// of a data table against partial edits).
    pub fn get_data_table(
        &self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Result<Option<DataTable>, String> {
        self.model.get_data_table(sheet, row, column)
    }

    /// Creates (or replaces) a What-If data table over `range` on `sheet`,
    /// recording the change for undo/redo and recomputing. The orientation is
    /// inferred from which input cells are supplied:
    /// - both → two-variable table (`row_input_cell` = row input, `column_input_cell` = column input);
    /// - `row_input_cell` only → one-variable row-oriented table;
    /// - `column_input_cell` only → one-variable column-oriented table.
    pub fn set_data_table(
        &mut self,
        sheet: u32,
        range: &str,
        row_input_cell: Option<&str>,
        column_input_cell: Option<&str>,
    ) -> Result<(), String> {
        let data_table = build_data_table(range, row_input_cell, column_input_cell)?;
        let old_value = self.model.set_data_table(sheet, data_table.clone())?;
        self.push_diff_list(vec![Diff::SetDataTable {
            sheet,
            old_value: Box::new(old_value),
            new_value: Box::new(data_table),
        }]);
        self.evaluate_if_not_paused();
        Ok(())
    }

    /// Removes the data table whose output range contains `(row, column)` on
    /// `sheet`, recording the change for undo/redo.
    pub fn delete_data_table(&mut self, sheet: u32, row: i32, column: i32) -> Result<(), String> {
        let old_value = self.model.delete_data_table(sheet, row, column)?;
        self.push_diff_list(vec![Diff::DeleteDataTable {
            sheet,
            old_value: Box::new(old_value),
        }]);
        self.evaluate_if_not_paused();
        Ok(())
    }

    /// Returns the workbook's iterative-calculation settings.
    pub fn get_iterative_calculation(&self) -> &CalcProperties {
        self.model.get_iterative_calculation()
    }

    /// Enables or disables iterative calculation (and optionally updates the
    /// maximum iteration count and convergence delta), recording the change for
    /// undo/redo and recomputing.
    pub fn set_iterative_calculation(
        &mut self,
        iterate: bool,
        iterate_count: Option<u32>,
        iterate_delta: Option<f64>,
    ) -> Result<(), String> {
        let old_value = self.model.get_iterative_calculation().clone();
        self.model
            .set_iterative_calculation(iterate, iterate_count, iterate_delta);
        let new_value = self.model.get_iterative_calculation().clone();
        self.push_diff_list(vec![Diff::SetCalcProperties {
            old_value: Box::new(old_value),
            new_value: Box::new(new_value),
        }]);
        self.evaluate_if_not_paused();
        Ok(())
    }
}
