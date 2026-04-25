//! WK3 file import.
//!
//! Layout mirrors `xlsx/src/import/`: a public `load_from_wk3` entry point
//! parses bytes into an `ironcalc_base::Model`. The internal structure is
//! split into:
//!
//! - [`encoding`]: CP437 → UTF-8 transcoding (Lotus DOS files use CP437).
//! - [`tokens`]: Lotus packed-number decoders.
//! - [`formula`]: tokenised-formula → Excel-formula-string decoder.
//! - [`record`]: top-level WK3 record dispatcher.

mod encoding;
mod formula;
mod record;
mod style;
mod tokens;

use std::fs;

use ironcalc_base::Model;

use crate::error::LotusError;

/// Loads a [`Model`] from a `.WK3` file on disk.
///
/// The workbook's `name` field is set to the full `file_name` argument; if
/// you want a cleaner name (e.g. just the file stem), read the bytes
/// yourself and call [`load_from_wk3_bytes`].
pub fn load_from_wk3<'a>(
    file_name: &'a str,
    locale: &'a str,
    tz: &'a str,
    language: &'a str,
) -> Result<Model<'a>, LotusError> {
    let bytes = fs::read(file_name)?;
    let mut model =
        Model::new_empty(file_name, locale, tz, language).map_err(LotusError::Workbook)?;
    record::load_into_model(&bytes, &mut model)?;
    Ok(model)
}

/// Loads a [`Model`] from in-memory `.WK3` bytes.
pub fn load_from_wk3_bytes<'a>(
    bytes: &[u8],
    name: &'a str,
    locale: &'a str,
    tz: &'a str,
    language: &'a str,
) -> Result<Model<'a>, LotusError> {
    let mut model = Model::new_empty(name, locale, tz, language).map_err(LotusError::Workbook)?;
    record::load_into_model(bytes, &mut model)?;
    Ok(model)
}
