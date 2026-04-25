//! WK3 record-level dispatcher.
//!
//! Reads `(opcode_u16, length_u16, payload)` records, recognises the WK3
//! header (BOF: opcode 0x0000, file_code 0x1000, file_sub 0x0004), and
//! dispatches the cell records (LABELCELL 0x16, NUMBERCELL 0x17,
//! SMALLNUMCELL 0x18, FORMULACELL 0x19, ERRCELL 0x14, NACELL 0x15) plus
//! COLUMNWIDTH, USERRANGE, and the NamedSheet sub-record of 0x1B.
//!
//! Mirrors `ImportLotus::parse` in `sc/source/filter/lotus/lotread.cxx`.

use ironcalc_base::Model;

use crate::error::LotusError;
use crate::import::encoding::cp437_to_string;
use crate::import::formula::{decode_formula, FormulaOrigin};
use crate::import::style::{
    apply_style_to_cell, build_style, decode_font_face, decode_font_size, decode_font_type,
    decode_pattern, LotAttr, PatternEntry, StyleCache,
};
use crate::import::tokens::snum_to_double;

const OP_BOF: u16 = 0x0000;
const OP_EOF: u16 = 0x0001;
const OP_PASSWORD: u16 = 0x0002;
const OP_COLUMNWIDTH: u16 = 0x0007;
const OP_HIDDENCOLUMN: u16 = 0x0008;
const OP_USERRANGE: u16 = 0x0009;
const OP_ERRCELL: u16 = 0x0014;
const OP_NACELL: u16 = 0x0015;
const OP_LABELCELL: u16 = 0x0016;
const OP_NUMBERCELL: u16 = 0x0017;
const OP_SMALLNUMCELL: u16 = 0x0018;
const OP_FORMULACELL: u16 = 0x0019;
const OP_EXTENDED_ATTR: u16 = 0x001B;
const OP_FONT_FACE: u16 = 174;
const OP_FONT_TYPE: u16 = 176;
const OP_FONT_SIZE: u16 = 177;
const OP_ROW_RECORD: u16 = 0x00C5;
const SUBTYPE_ROW_PRESENTATION: u16 = 2007;
const SUBTYPE_NAMED_SHEET: u16 = 14000;

pub fn load_into_model(bytes: &[u8], model: &mut Model) -> Result<(), LotusError> {
    let mut reader = RecReader::new(bytes);

    // First record must be BOF declaring this as a WK3 file.
    let bof = reader
        .next_record()?
        .ok_or_else(|| LotusError::NotWk3("empty file".into()))?;
    if bof.opcode != OP_BOF || bof.payload.len() < 4 {
        return Err(LotusError::NotWk3(format!(
            "expected BOF, got opcode 0x{:04x}",
            bof.opcode
        )));
    }
    let file_code = u16_le(bof.payload, 0);
    let file_sub = u16_le(bof.payload, 2);
    if file_code != 0x1000 || file_sub != 0x0004 {
        return Err(LotusError::NotWk3(format!(
            "file_code=0x{file_code:04x} file_sub=0x{file_sub:04x} (expected 0x1000/0x0004)"
        )));
    }

    let mut cache = StyleCache::default();

    while let Some(rec) = reader.next_record()? {
        match rec.opcode {
            OP_EOF => {
                // The main WK3 stream is over. LO continues to read FM3-style
                // font/format records (174/176/177/0xC5) from a sibling
                // stream — for single-file WK3 we just stop here.
                break;
            }
            OP_PASSWORD => {
                return Err(LotusError::NotWk3(
                    "encrypted WK3 files are not supported".into(),
                ));
            }
            OP_COLUMNWIDTH => handle_column_width(model, rec.payload)?,
            OP_HIDDENCOLUMN => handle_hidden_column(model, rec.payload)?,
            OP_USERRANGE => handle_user_range(model, rec.payload)?,
            OP_ERRCELL => handle_simple_cell(model, rec.payload, "#REF!")?,
            OP_NACELL => handle_simple_cell(model, rec.payload, "#N/A")?,
            OP_LABELCELL => handle_label_cell(model, rec.payload)?,
            OP_NUMBERCELL => handle_number_cell(model, rec.payload)?,
            OP_SMALLNUMCELL => handle_small_num_cell(model, rec.payload)?,
            OP_FORMULACELL => handle_formula_cell(model, rec.payload)?,
            OP_EXTENDED_ATTR => handle_extended_attr(model, rec.payload, &mut cache)?,
            OP_FONT_FACE => decode_font_face(rec.payload, &mut cache),
            OP_FONT_TYPE => decode_font_type(rec.payload, &mut cache),
            OP_FONT_SIZE => decode_font_size(rec.payload, &mut cache),
            OP_ROW_RECORD => handle_row_record(model, rec.payload, &cache)?,
            _ => {
                // Unknown / unhandled record — silently skip, matching LO.
            }
        }
    }
    Ok(())
}

struct Record<'a> {
    opcode: u16,
    payload: &'a [u8],
}

struct RecReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> RecReader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn next_record(&mut self) -> Result<Option<Record<'a>>, LotusError> {
        if self.pos >= self.buf.len() {
            return Ok(None);
        }
        if self.pos + 4 > self.buf.len() {
            return Err(LotusError::Truncated {
                offset: self.pos as u64,
                opcode: 0,
                expected: 4,
            });
        }
        let opcode = u16_le(self.buf, self.pos);
        let length = u16_le(self.buf, self.pos + 2) as usize;
        let body_start = self.pos + 4;
        let body_end = body_start + length;
        if body_end > self.buf.len() {
            return Err(LotusError::Truncated {
                offset: self.pos as u64,
                opcode,
                expected: length,
            });
        }
        let payload = &self.buf[body_start..body_end];
        self.pos = body_end;
        Ok(Some(Record { opcode, payload }))
    }
}

fn u16_le(buf: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([buf[at], buf[at + 1]])
}

fn read_address(payload: &[u8]) -> Result<(u8, u16, u8), LotusError> {
    if payload.len() < 4 {
        return Err(LotusError::IO(format!(
            "cell record too short ({} bytes)",
            payload.len()
        )));
    }
    let row = u16_le(payload, 0);
    let tab = payload[2];
    let col = payload[3];
    Ok((tab, row, col))
}

/// Ensures that `model` has at least `tab + 1` worksheets, adding sheets as
/// needed using Lotus 1-2-3's default naming convention (`A`, `B`, ..., `Z`,
/// `AA`, ...). Also renames the initial sheet to `A` on first encounter so a
/// freshly-loaded multi-sheet workbook reads naturally as `A`, `B`, `C`.
/// Returns the sheet index to use.
fn ensure_sheet(model: &mut Model, tab: u8) -> Result<u32, LotusError> {
    // Promote the auto-created "Sheet1" to "A" the first time we touch the
    // workbook. We only do this if the user hasn't given the sheet a custom
    // name yet (NamedSheet records, when present, run before any cells).
    if let Some(first) = model.workbook.worksheets.first() {
        if first.name == "Sheet1" {
            let _ = model.rename_sheet_by_index(0, "A");
        }
    }
    let needed = tab as usize + 1;
    while model.workbook.worksheets.len() < needed {
        let next_index = model.workbook.worksheets.len() as u32;
        let name = lotus_default_sheet_name(next_index);
        // Skip add if a sheet with that name already exists (rare, but safe).
        if !model.workbook.worksheets.iter().any(|w| w.name == name) {
            model.add_sheet(&name).map_err(LotusError::Workbook)?;
        } else {
            // Fall back to a unique numeric suffix.
            model
                .add_sheet(&format!("{name}_{next_index}"))
                .map_err(LotusError::Workbook)?;
        }
    }
    Ok(tab as u32)
}

/// Lotus 1-2-3 R3 names sheets `A`, `B`, ..., `Z`, `AA`, `AB`, ... — the same
/// alphabet the spreadsheet uses for columns. Index 0 = `A`.
fn lotus_default_sheet_name(index: u32) -> String {
    col_letters(index)
}

fn handle_label_cell(model: &mut Model, payload: &[u8]) -> Result<(), LotusError> {
    let (tab, row, col) = read_address(payload)?;
    let sheet = ensure_sheet(model, tab)?;
    if payload.len() < 5 {
        return Ok(()); // empty label
    }
    // payload[4] is the alignment-prefix character (', ", ^, \, |). It is part
    // of the on-disk encoding; stripping it gives the user-visible text.
    let _align = payload[4];
    let raw = &payload[5..];
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    let text = cp437_to_string(&raw[..end]);
    // Quote-prefix forces IronCalc to treat the value as text even if the body
    // happens to look like a number or a formula (e.g. labels starting with `=`).
    set_text(model, sheet, row, col, &text)
}

fn handle_number_cell(model: &mut Model, payload: &[u8]) -> Result<(), LotusError> {
    let (tab, row, col) = read_address(payload)?;
    let sheet = ensure_sheet(model, tab)?;
    if payload.len() < 12 {
        return Err(LotusError::IO("NUMBERCELL too short".into()));
    }
    let mut a = [0u8; 8];
    a.copy_from_slice(&payload[4..12]);
    let v = f64::from_le_bytes(a);
    set_number(model, sheet, row, col, v)
}

fn handle_small_num_cell(model: &mut Model, payload: &[u8]) -> Result<(), LotusError> {
    let (tab, row, col) = read_address(payload)?;
    let sheet = ensure_sheet(model, tab)?;
    if payload.len() < 6 {
        return Err(LotusError::IO("SMALLNUMCELL too short".into()));
    }
    let raw = i16::from_le_bytes([payload[4], payload[5]]);
    let v = snum_to_double(raw);
    set_number(model, sheet, row, col, v)
}

fn handle_formula_cell(model: &mut Model, payload: &[u8]) -> Result<(), LotusError> {
    let (tab, row, col) = read_address(payload)?;
    let sheet = ensure_sheet(model, tab)?;
    // 4 address bytes + 10 skipped (8-byte cached double + 2 bytes of trailing
    // metadata as in `ImportLotus::Formulacell`).
    if payload.len() < 14 {
        return Err(LotusError::IO("FORMULACELL too short".into()));
    }
    let tokens = &payload[14..];
    let origin = FormulaOrigin {
        sheet: tab,
        row,
        column: col,
    };
    // The 10-byte cached-result area is an Intel 80-bit extended-precision
    // long double, NOT a 64-bit IEEE 754 double. (Lotus 1-2-3 R3 stored
    // calculation results in x87 native format.) Decode to f64 for storage.
    let cached = decode_long_double_le(&payload[4..14]);
    let formula = match decode_formula(tokens, origin) {
        Ok((f, _)) => f,
        Err(_) => return set_number(model, sheet, row, col, cached),
    };
    let cell_input = format!("={formula}");
    let display_row = row as i32 + 1;
    let display_col = col as i32 + 1;
    if let Err(e) = model.set_user_input(sheet, display_row, display_col, cell_input) {
        let _ = e;
        return set_number(model, sheet, row, col, cached);
    }
    Ok(())
}

fn handle_simple_cell(model: &mut Model, payload: &[u8], text: &str) -> Result<(), LotusError> {
    let (tab, row, col) = read_address(payload)?;
    let sheet = ensure_sheet(model, tab)?;
    set_text(model, sheet, row, col, text)
}

fn handle_column_width(model: &mut Model, payload: &[u8]) -> Result<(), LotusError> {
    if payload.len() < 4 {
        return Ok(());
    }
    let tab = payload[0];
    let window2 = payload[1];
    if window2 != 0 {
        return Ok(()); // matches LO behaviour: only window 0 applies
    }
    let sheet = ensure_sheet(model, tab)?;
    // 2 padding bytes after tab/window2, then [col, spaces] pairs.
    let mut i = 4;
    while i + 1 < payload.len() {
        let col = payload[i];
        let spaces = payload[i + 1];
        if spaces > 0 {
            // LO uses TWIPS_PER_CHAR * 1.28 * spaces. IronCalc's column width
            // is in Excel "character widths" (the same unit the WK3 record
            // already stores), so the conversion is the identity.
            let width = f64::from(spaces);
            let _ = model.set_column_width(sheet, col as i32 + 1, width);
        }
        i += 2;
    }
    Ok(())
}

fn handle_user_range(model: &mut Model, payload: &[u8]) -> Result<(), LotusError> {
    // Layout (mirrors ImportLotus::Userrange):
    //   range_type (u16), name (16 bytes, null-padded),
    //   colSt (u16), rowSt (u16), tabSt (u8 + pad u8?),
    //   colEnd (u16), rowEnd (u16), tabEnd (u8 + pad u8?)
    // The trailing range layout is `ScRange`, which in the WK3 import path is
    // 12 bytes (two ScAddress slots of 4 bytes plus 4 padding). We handle the
    // common single-tab case and skip multi-tab ranges.
    if payload.len() < 18 {
        return Ok(());
    }
    let raw_name = &payload[2..18];
    let end = raw_name
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(raw_name.len());
    let name_text = cp437_to_string(&raw_name[..end]);
    if name_text.is_empty() {
        return Ok(());
    }
    let name = sanitize_defined_name(&name_text);

    // Parse the two endpoints if there's enough data; otherwise skip.
    if payload.len() < 18 + 8 {
        return Ok(());
    }
    let row_s = u16_le(payload, 18);
    let tab_s = payload[20];
    let col_s = payload[21];
    // Endpoint 2 follows the same 4-byte layout.
    let (row_e, tab_e, col_e) = if payload.len() >= 26 {
        (u16_le(payload, 22), payload[24], payload[25])
    } else {
        (row_s, tab_s, col_s)
    };
    if tab_s != tab_e {
        // 3D ranges aren't represented yet — skip to avoid creating a bad
        // defined name.
        return Ok(());
    }
    let _ = ensure_sheet(model, tab_s)?;
    let sheet_name = model
        .workbook
        .worksheets
        .get(tab_s as usize)
        .map(|w| w.name.clone())
        .unwrap_or_default();
    let formula = if row_s == row_e && col_s == col_e {
        format!(
            "{}!${}${}",
            quote_sheet(&sheet_name),
            col_letters(col_s as u32),
            row_s as u32 + 1
        )
    } else {
        format!(
            "{sheet}!${col_a}${row_a}:${col_b}${row_b}",
            sheet = quote_sheet(&sheet_name),
            col_a = col_letters(col_s as u32),
            row_a = row_s as u32 + 1,
            col_b = col_letters(col_e as u32),
            row_b = row_e as u32 + 1,
        )
    };
    let _ = model.new_defined_name(&name, None, &formula);
    Ok(())
}

fn handle_extended_attr(
    model: &mut Model,
    payload: &[u8],
    cache: &mut StyleCache,
) -> Result<(), LotusError> {
    if payload.len() < 2 {
        return Ok(());
    }
    let subtype = u16_le(payload, 0);
    match subtype {
        SUBTYPE_NAMED_SHEET => {
            // NamedSheet: u16 subtype, u16 sheet number, null-terminated name.
            if payload.len() < 4 {
                return Ok(());
            }
            let sheet_num = u16_le(payload, 2) as u8;
            let raw = &payload[4..];
            let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            let name = cp437_to_string(&raw[..end]);
            if name.is_empty() {
                return Ok(());
            }
            let sheet = ensure_sheet(model, sheet_num)?;
            let safe = sanitize_sheet_name(&name);
            let _ = model.rename_sheet_by_index(sheet, &safe);
        }
        SUBTYPE_ROW_PRESENTATION => handle_row_presentation(model, &payload[2..])?,
        _ => {
            // The CreatePattern record uses opcode 0x1B with sub-CODE 0x0FD2
            // (a sub-code, not a sub-type — same first u16 slot but different
            // semantic). decode_pattern checks the sub-code internally.
            if let Some((pattern_id, entry)) = decode_pattern(payload) {
                cache.patterns.insert(pattern_id, entry);
            }
        }
    }
    Ok(())
}

/// RowPresentation (0x1B subtype 2007). Layout after subtype is consumed:
///   tab(u8), pad(u8), then N entries of 8 bytes each:
///     row(u16), height(u16, low 12 bits), 2-byte skip,
///     flags(u8, bit 1 = fixed), 1-byte skip.
fn handle_row_presentation(model: &mut Model, body: &[u8]) -> Result<(), LotusError> {
    if body.len() < 2 {
        return Ok(());
    }
    let tab = body[0];
    let sheet = ensure_sheet(model, tab)?;
    let mut i = 2;
    while i + 8 <= body.len() {
        let row = u16_le(body, i);
        let raw_height = u16_le(body, i + 2);
        let flags = body[i + 6];
        i += 8;
        if flags & 0x02 == 0 {
            // Auto-fit row — no explicit height to set.
            continue;
        }
        // Lotus stores height in 1/32 of a point; LO converts via *20/32 to
        // get TWIPS. IronCalc's row height is in points, so convert as
        // (raw * 20 / 32) / 20 = raw / 32 ... but Excel uses points-as-f64,
        // so just use raw_height & 0x0FFF directly in points (close enough
        // for spreadsheet display purposes).
        let height_points = f64::from(raw_height & 0x0FFF) / 32.0 * 12.0; // ~12pt baseline
        if height_points > 0.0 {
            let _ = model.set_row_height(sheet, row as i32 + 1, height_points);
        }
    }
    Ok(())
}

/// HIDDENCOLUMN (opcode 0x08). Layout:
///   tab(u8), window2(u8), pad(u16), then a list of u8 column indices.
fn handle_hidden_column(model: &mut Model, payload: &[u8]) -> Result<(), LotusError> {
    if payload.len() < 4 {
        return Ok(());
    }
    let tab = payload[0];
    let window2 = payload[1];
    if window2 != 0 {
        return Ok(());
    }
    let sheet = ensure_sheet(model, tab)?;
    for &col in &payload[4..] {
        let _ = model.set_column_hidden(sheet, col as i32 + 1, true);
    }
    Ok(())
}

/// Per-row attribute record (opcode 0xC5). Carries per-column-run
/// `LotAttrWK3` values applied to a single row. Layout:
///   row(u16), height(u16, low 12 bits, *22 for TWIPS),
///   then N entries of `LotAttrWK3 (4 bytes)` + `repeats (u8)` = 5 bytes each.
///
/// Only used in WK3 files that ship a separate FM3 styling stream — none of
/// the bundled DOS samples have one. Implemented for completeness so files
/// that do carry styling round-trip with bold/italic/borders/colours intact.
fn handle_row_record(
    model: &mut Model,
    payload: &[u8],
    cache: &StyleCache,
) -> Result<(), LotusError> {
    if payload.len() < 4 {
        return Ok(());
    }
    let tab = 0u8; // Row records don't carry a tab; use the active sheet.
    let sheet = ensure_sheet(model, tab)?;
    let row = u16_le(payload, 0);
    let raw_height = u16_le(payload, 2) & 0x0FFF;
    if raw_height != 0 {
        // Same conversion as RowPresentation.
        let height_points = f64::from(raw_height) / 32.0 * 12.0;
        let _ = model.set_row_height(sheet, row as i32 + 1, height_points);
    }

    let mut i = 4;
    let mut col: u32 = 0;
    let base_style = model
        .get_style_for_cell(sheet, row as i32 + 1, 1)
        .unwrap_or_default();
    while i + 5 <= payload.len() {
        let attr =
            LotAttr::from_bytes([payload[i], payload[i + 1], payload[i + 2], payload[i + 3]]);
        let repeats = payload[i + 4];
        i += 5;

        if attr.has_styles() {
            let style = build_style(cache, &attr, None::<&PatternEntry>, &base_style);
            for c in col..=col + repeats as u32 {
                let _ = apply_style_to_cell(model, sheet, row as u32 + 1, c + 1, &style);
            }
        }
        col += repeats as u32 + 1;
    }
    Ok(())
}

fn set_number(model: &mut Model, sheet: u32, row: u16, col: u8, v: f64) -> Result<(), LotusError> {
    model
        .set_user_input(sheet, row as i32 + 1, col as i32 + 1, format_number(v))
        .map_err(LotusError::Workbook)
}

fn set_text(
    model: &mut Model,
    sheet: u32,
    row: u16,
    col: u8,
    text: &str,
) -> Result<(), LotusError> {
    // Force-text via the leading-quote convention so labels like `=foo` or `1.0`
    // don't get reinterpreted as formulas/numbers.
    let mut payload = String::with_capacity(text.len() + 1);
    payload.push('\'');
    payload.push_str(text);
    model
        .set_user_input(sheet, row as i32 + 1, col as i32 + 1, payload)
        .map_err(LotusError::Workbook)
}

fn format_number(v: f64) -> String {
    if !v.is_finite() {
        return "#NUM!".into();
    }
    if v == v.trunc() && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

fn sanitize_defined_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for (i, c) in name.chars().enumerate() {
        let ok = c.is_alphanumeric() || c == '_' || c == '.';
        if i == 0 && c.is_ascii_digit() {
            out.push('_');
        }
        out.push(if ok { c } else { '_' });
    }
    if out.is_empty() {
        "_".into()
    } else {
        out
    }
}

fn sanitize_sheet_name(name: &str) -> String {
    // Excel disallows: \ / ? * [ ] : and length > 31.
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if matches!(c, '\\' | '/' | '?' | '*' | '[' | ']' | ':') {
            out.push('_');
        } else {
            out.push(c);
        }
    }
    if out.is_empty() {
        return "Sheet".into();
    }
    if out.chars().count() > 31 {
        out = out.chars().take(31).collect();
    }
    out
}

fn quote_sheet(name: &str) -> String {
    if name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
    {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

/// Decode a little-endian Intel 80-bit extended-precision float into f64.
///
/// Layout (LE): bytes 0..8 = 64-bit significand (with explicit integer bit
/// at MSB), bytes 8..10 = 16-bit field where bit 15 is the sign and bits
/// 0..14 are the biased exponent (bias 16383). Subnormal/Inf/NaN handling
/// is approximate — sufficient for cached spreadsheet results, where these
/// edge cases shouldn't appear in valid data.
pub(super) fn decode_long_double_le(b: &[u8]) -> f64 {
    if b.len() < 10 {
        return 0.0;
    }
    let mut m = [0u8; 8];
    m.copy_from_slice(&b[0..8]);
    let mantissa = u64::from_le_bytes(m);
    let raw_exp = u16::from_le_bytes([b[8], b[9]]);
    let sign = (raw_exp >> 15) & 1;
    let biased_exp = (raw_exp & 0x7FFF) as i32;

    if biased_exp == 0 && mantissa == 0 {
        return if sign == 1 { -0.0 } else { 0.0 };
    }
    if biased_exp == 0x7FFF {
        // Inf or NaN — represent as f64::NAN; spreadsheet importers rarely care.
        return f64::NAN;
    }

    let unbiased = biased_exp - 16383;
    // Significand = mantissa as a Q63 fixed-point in [0, 2): we treat the top
    // bit as the integer bit and bits 62..0 as the fractional part.
    let sig = (mantissa as f64) / (1u64 << 63) as f64;
    let mut v = sig * 2f64.powi(unbiased);
    if sign == 1 {
        v = -v;
    }
    v
}

pub(super) fn col_letters(mut c: u32) -> String {
    let mut out = String::new();
    c += 1;
    while c > 0 {
        let r = ((c - 1) % 26) as u8;
        out.insert(0, (b'A' + r) as char);
        c = (c - 1) / 26;
    }
    out
}
