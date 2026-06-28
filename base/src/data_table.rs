//! Evaluation of Excel Data Tables (What-If Analysis).

use std::collections::HashMap;

use crate::{
    calc_result::CalcResult,
    expressions::{
        parser::parse_range, token::Error, types::CellReferenceIndex, utils::parse_reference_a1,
    },
    model::Model,
    types::{Cell, DataTable, Worksheet},
};

struct ResolvedDataTable {
    sheet: u32,
    top: i32,
    left: i32,
    bottom: i32,
    right: i32,
    two_dimensional: bool,
    row_oriented: bool,
    r1: CellReferenceIndex,
    r2: Option<CellReferenceIndex>,
}

fn split_sheet_reference(reference: &str) -> Option<(Option<String>, &str)> {
    let reference = reference.trim();
    if reference.is_empty() {
        return None;
    }

    if let Some(rest) = reference.strip_prefix('\'') {
        let mut sheet_name = String::new();
        let mut chars = rest.char_indices().peekable();
        while let Some((index, ch)) = chars.next() {
            if ch == '\'' {
                if matches!(chars.peek(), Some((_, '\''))) {
                    sheet_name.push('\'');
                    let _ = chars.next();
                    continue;
                }
                let after_quote = &rest[index + ch.len_utf8()..];
                return after_quote
                    .strip_prefix('!')
                    .map(|cell_reference| (Some(sheet_name), cell_reference));
            }
            sheet_name.push(ch);
        }
        return None;
    }

    if let Some((sheet_name, cell_reference)) = reference.rsplit_once('!') {
        Some((Some(sheet_name.to_string()), cell_reference))
    } else {
        Some((None, reference))
    }
}

fn parse_cell_reference(
    model: &Model,
    default_sheet: u32,
    reference: &str,
) -> Option<CellReferenceIndex> {
    let (sheet_name, cell_reference) = split_sheet_reference(reference)?;
    let sheet = match sheet_name {
        Some(sheet_name) => model.get_sheet_index_by_name(&sheet_name)?,
        None => default_sheet,
    };
    let parsed = parse_reference_a1(&cell_reference.to_ascii_uppercase())?;
    Some(CellReferenceIndex {
        sheet,
        row: parsed.row,
        column: parsed.column,
    })
}

/// Returns the index of the data table on `worksheet` whose output range
/// contains `(row, column)`, if any.
fn data_table_index_at(worksheet: &Worksheet, row: i32, column: i32) -> Option<usize> {
    worksheet.data_tables.iter().position(|dt| {
        parse_range(&dt.range)
            .map(|(left, top, right, bottom)| {
                row >= top && row <= bottom && column >= left && column <= right
            })
            .unwrap_or(false)
    })
}

impl Model<'_> {
    pub(crate) fn compute_data_tables(&mut self) {
        let tables = self.resolve_data_tables();
        if tables.is_empty() {
            return;
        }

        for table in &tables {
            let mut outputs: Vec<(CellReferenceIndex, CalcResult)> = Vec::new();
            self.compute_one_data_table(table, &mut outputs);
            for (cell_reference, value) in &outputs {
                let style = self
                    .workbook
                    .worksheet(cell_reference.sheet)
                    .ok()
                    .and_then(|ws| ws.cell(cell_reference.row, cell_reference.column))
                    .map_or(0, Cell::get_style);
                self.write_value(*cell_reference, value, style);
            }

            // The per-scenario recompute persisted override-influenced values into
            // the governing/intermediate formula cells. Settle the workbook so they
            // return to their real (non-substituted) values, and so formulas that
            // consume this table's outputs observe them before the next table.
            // (Input cells were never mutated, so there is nothing to restore.)
            // Iterative settle so a circular workbook re-converges after the
            // per-scenario recomputes dirtied the governing/intermediate cells.
            self.evaluate_workbook_cells_iterative();
        }
    }

    fn resolve_data_tables(&self) -> Vec<ResolvedDataTable> {
        let mut resolved = Vec::new();
        for (sheet_index, worksheet) in self.workbook.worksheets.iter().enumerate() {
            let sheet = sheet_index as u32;
            for table in &worksheet.data_tables {
                let (Ok((left, top, right, bottom)), Some(r1)) = (
                    parse_range(&table.range),
                    parse_cell_reference(self, sheet, &table.r1),
                ) else {
                    continue;
                };
                if top <= 1 || left <= 1 || bottom < top || right < left {
                    continue;
                }
                let r2 = match &table.r2 {
                    Some(reference) => match parse_cell_reference(self, sheet, reference) {
                        Some(reference) => Some(reference),
                        None => continue,
                    },
                    None => None,
                };
                resolved.push(ResolvedDataTable {
                    sheet,
                    top,
                    left,
                    bottom,
                    right,
                    two_dimensional: table.two_dimensional,
                    row_oriented: table.row_oriented,
                    r1,
                    r2,
                });
            }
        }
        resolved
    }

    fn compute_one_data_table(
        &mut self,
        table: &ResolvedDataTable,
        outputs: &mut Vec<(CellReferenceIndex, CalcResult)>,
    ) {
        let sheet = table.sheet;
        let rows: Vec<i32> = (table.top..=table.bottom).collect();
        let columns: Vec<i32> = (table.left..=table.right).collect();

        if table.two_dimensional {
            let Some(r2) = table.r2 else { return };
            let master = CellReferenceIndex {
                sheet,
                row: table.top - 1,
                column: table.left - 1,
            };
            let row_inputs: Vec<CalcResult> = columns
                .iter()
                .map(|&column| {
                    self.read_value(CellReferenceIndex {
                        sheet,
                        row: table.top - 1,
                        column,
                    })
                })
                .collect();
            let column_inputs: Vec<CalcResult> = rows
                .iter()
                .map(|&row| {
                    self.read_value(CellReferenceIndex {
                        sheet,
                        row,
                        column: table.left - 1,
                    })
                })
                .collect();

            for (row_index, &row) in rows.iter().enumerate() {
                for (column_index, &column) in columns.iter().enumerate() {
                    // Redirect reads of BOTH input cells to this scenario's
                    // row/column header values; never touch the grid.
                    let overrides = HashMap::from([
                        (
                            (sheet, table.r1.row, table.r1.column),
                            row_inputs[column_index].clone(),
                        ),
                        ((sheet, r2.row, r2.column), column_inputs[row_index].clone()),
                    ]);
                    let value = self
                        .recompute_with_overrides(&[master], overrides)
                        .remove(0);
                    outputs.push((CellReferenceIndex { sheet, row, column }, value));
                }
            }
        } else if table.row_oriented {
            let inputs: Vec<CalcResult> = columns
                .iter()
                .map(|&column| {
                    self.read_value(CellReferenceIndex {
                        sheet,
                        row: table.top - 1,
                        column,
                    })
                })
                .collect();
            let masters: Vec<CellReferenceIndex> = rows
                .iter()
                .map(|&row| CellReferenceIndex {
                    sheet,
                    row,
                    column: table.left - 1,
                })
                .collect();

            for (column_index, &column) in columns.iter().enumerate() {
                let overrides = HashMap::from([(
                    (sheet, table.r1.row, table.r1.column),
                    inputs[column_index].clone(),
                )]);
                let values = self.recompute_with_overrides(&masters, overrides);
                for (&row, value) in rows.iter().zip(values) {
                    outputs.push((CellReferenceIndex { sheet, row, column }, value));
                }
            }
        } else {
            let inputs: Vec<CalcResult> = rows
                .iter()
                .map(|&row| {
                    self.read_value(CellReferenceIndex {
                        sheet,
                        row,
                        column: table.left - 1,
                    })
                })
                .collect();
            let masters: Vec<CellReferenceIndex> = columns
                .iter()
                .map(|&column| CellReferenceIndex {
                    sheet,
                    row: table.top - 1,
                    column,
                })
                .collect();

            for (row_index, &row) in rows.iter().enumerate() {
                let overrides = HashMap::from([(
                    (sheet, table.r1.row, table.r1.column),
                    inputs[row_index].clone(),
                )]);
                let values = self.recompute_with_overrides(&masters, overrides);
                for (&column, value) in columns.iter().zip(values) {
                    outputs.push((CellReferenceIndex { sheet, row, column }, value));
                }
            }
        }
    }

    fn read_value(&self, cell_reference: CellReferenceIndex) -> CalcResult {
        match self
            .workbook
            .worksheet(cell_reference.sheet)
            .ok()
            .and_then(|ws| ws.cell(cell_reference.row, cell_reference.column))
        {
            Some(cell) => self.get_cell_value(cell, cell_reference),
            None => CalcResult::EmptyCell,
        }
    }

    fn write_value(&mut self, cell_reference: CellReferenceIndex, value: &CalcResult, style: i32) {
        let sheet = cell_reference.sheet;
        let row = cell_reference.row;
        let column = cell_reference.column;
        match value {
            CalcResult::Number(number) => {
                if let Ok(worksheet) = self.workbook.worksheet_mut(sheet) {
                    let _ = worksheet.set_cell_with_number(row, column, *number, style);
                }
            }
            CalcResult::Boolean(boolean) => {
                if let Ok(worksheet) = self.workbook.worksheet_mut(sheet) {
                    let _ = worksheet.set_cell_with_boolean(row, column, *boolean, style);
                }
            }
            CalcResult::String(text) => {
                let _ = self.set_cell_with_string(sheet, row, column, text, style);
            }
            CalcResult::Error { error, .. } => {
                if let Ok(worksheet) = self.workbook.worksheet_mut(sheet) {
                    let _ = worksheet.set_cell_with_error(row, column, error.clone(), style);
                }
            }
            CalcResult::EmptyCell | CalcResult::EmptyArg => {
                if let Ok(worksheet) = self.workbook.worksheet_mut(sheet) {
                    let _ = worksheet.cell_clear_contents_with_style(row, column, style);
                }
            }
            CalcResult::Range { .. } | CalcResult::Array(_) | CalcResult::Lambda(_) => {
                if let Ok(worksheet) = self.workbook.worksheet_mut(sheet) {
                    let _ = worksheet.set_cell_with_error(row, column, Error::VALUE, style);
                }
            }
        }
    }

    /// Adds a What-If data table to `sheet`, replacing any existing table
    /// anchored at the same top-left cell. Returns the replaced table, if any
    /// (for undo). The orientation/inputs come from `data_table`; the output
    /// range and input cell references are validated.
    pub fn set_data_table(
        &mut self,
        sheet: u32,
        data_table: DataTable,
    ) -> Result<Option<DataTable>, String> {
        let (left, top, right, bottom) = parse_range(&data_table.range)
            .map_err(|_| format!("Invalid data table range: '{}'", data_table.range))?;
        if top <= 1 || left <= 1 || bottom < top || right < left {
            return Err(format!(
                "Invalid data table range '{}': it needs a row above and a column to its left",
                data_table.range
            ));
        }
        if parse_cell_reference(self, sheet, &data_table.r1).is_none() {
            return Err(format!(
                "Invalid data table input cell: '{}'",
                data_table.r1
            ));
        }
        if let Some(r2) = &data_table.r2 {
            if parse_cell_reference(self, sheet, r2).is_none() {
                return Err(format!("Invalid data table input cell: '{r2}'"));
            }
        }
        let worksheet = self.workbook.worksheet_mut(sheet)?;
        // Replace a table anchored at the same top-left cell, if present.
        let replaced = worksheet
            .data_tables
            .iter()
            .position(|dt| {
                parse_range(&dt.range)
                    .map(|(l, t, _, _)| t == top && l == left)
                    .unwrap_or(false)
            })
            .map(|pos| worksheet.data_tables.remove(pos));
        worksheet.data_tables.push(data_table);
        Ok(replaced)
    }

    /// Removes the data table whose output range contains `(row, column)` on
    /// `sheet`, clearing its body cells and returning the removed table (for
    /// undo). Errors if there is no data table at that position.
    pub fn delete_data_table(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Result<DataTable, String> {
        let worksheet = self.workbook.worksheet_mut(sheet)?;
        let pos = data_table_index_at(worksheet, row, column).ok_or_else(|| {
            format!("No data table at row {row}, column {column} on sheet {sheet}")
        })?;
        let removed = worksheet.data_tables.remove(pos);
        // Clear the (now orphaned) output cells, leaving their styles. A removed
        // table's body would otherwise keep stale computed values.
        if let Ok((left, top, right, bottom)) = parse_range(&removed.range) {
            for r in top..=bottom {
                for c in left..=right {
                    let _ = worksheet.cell_clear_contents(r, c);
                }
            }
        }
        Ok(removed)
    }

    /// Returns a copy of the data table whose output range contains
    /// `(row, column)` on `sheet`, or `None`.
    pub fn get_data_table(
        &self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Result<Option<DataTable>, String> {
        let worksheet = self.workbook.worksheet(sheet)?;
        Ok(data_table_index_at(worksheet, row, column)
            .map(|pos| worksheet.data_tables[pos].clone()))
    }
}
