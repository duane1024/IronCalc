use std::io;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LotusError {
    #[error("I/O error: {0}")]
    IO(String),
    #[error("Not a Lotus WK3 file: {0}")]
    NotWk3(String),
    #[error(
        "Truncated record at offset {offset}: opcode 0x{opcode:04x} expected {expected} bytes"
    )]
    Truncated {
        offset: u64,
        opcode: u16,
        expected: usize,
    },
    #[error("Malformed formula in cell {sheet}!{column}{row}: {reason}")]
    Formula {
        sheet: u32,
        row: i32,
        column: i32,
        reason: String,
    },
    #[error("{0}")]
    Workbook(String),
}

impl From<io::Error> for LotusError {
    fn from(error: io::Error) -> Self {
        LotusError::IO(error.to_string())
    }
}
