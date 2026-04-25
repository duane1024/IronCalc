//! Style accumulator for WK3 imports.
//!
//! WK3 splits styling across several record types whose effects compose:
//!
//! - **Font table** (records 174 / 176 / 177): a flat array of 8 fonts —
//!   `(name, ysize, type)` — referenced by index from per-row attribute
//!   records.
//! - **Pattern pool** (record 0x1B with sub-code 0x0FD2): named patterns
//!   carrying bold / italic / underline / horizontal+vertical alignment,
//!   referenced by ID from `OP_ApplyPatternArea123`-style records.
//! - **Row records** (opcode 0xC5): per-cell-run `LotAttrWK3` (font index,
//!   border lines, font colour index, background colour) applied to a slice
//!   of columns on a single row.
//!
//! None of the 20 DOS-era sample files we ship actually contain these
//! records (they were authored in CGA/text-mode Lotus with default styling),
//! so this code is exercised only by the synthetic test in
//! `tests/styling.rs`. The shape mirrors LibreOffice's `LotAttrCache` /
//! `LotusFontBuffer` / `aLotusPatternPool` so that real files which do
//! contain styling round-trip the same way LO would render them.

use ironcalc_base::types::{Border, BorderItem, BorderStyle, Fill, Style};
use ironcalc_base::Model;

use crate::import::encoding::cp437_to_string;

/// Maximum number of fonts in a WK3 font table (matches LO's
/// `LotusFontBuffer::nSize`).
pub const FONT_TABLE_SIZE: usize = 8;

#[derive(Default, Clone)]
pub struct FontEntry {
    pub name: Option<String>,
    pub size_twentieths: Option<u16>,
    pub type_flags: u16,
}

impl FontEntry {
    /// LO encodes bold/italic/underline as bits within the type word.
    /// Bit layout (matches LotusFontBuffer):
    ///   bit 0  – bold
    ///   bit 1  – italic
    ///   bit 2  – underline
    pub fn bold(&self) -> bool {
        self.type_flags & 0x0001 != 0
    }
    pub fn italic(&self) -> bool {
        self.type_flags & 0x0002 != 0
    }
    pub fn underline(&self) -> bool {
        self.type_flags & 0x0004 != 0
    }
}

#[derive(Default)]
pub struct StyleCache {
    pub fonts: [FontEntry; FONT_TABLE_SIZE],
    pub patterns: std::collections::HashMap<u16, PatternEntry>,
}

#[derive(Default, Clone)]
pub struct PatternEntry {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// Lotus alignment encoding (bits 0-2 of the 21st byte of CreatePattern):
    /// 1=Left, 2=Right, 3=Center, 4=Standard, 6=Justify, 0/other=Default.
    pub h_align: u8,
    /// Lotus vertical alignment (bits 0-2 of the 22nd byte): 1=Top, 2=Middle,
    /// 4=Bottom, 0=Default. Surfaced through `apply_pattern_v_align` when
    /// applying a pattern to a cell; stored here so the pipeline preserves
    /// what the file specified rather than dropping the byte on the floor.
    pub v_align: u8,
}

impl StyleCache {
    pub fn set_font_name(&mut self, idx: u8, name: String) {
        if let Some(slot) = self.fonts.get_mut(idx as usize) {
            slot.name = Some(name);
        }
    }
    pub fn set_font_type(&mut self, idx: usize, t: u16) {
        if let Some(slot) = self.fonts.get_mut(idx) {
            slot.type_flags = t;
        }
    }
    pub fn set_font_size(&mut self, idx: usize, size_twentieths: u16) {
        if let Some(slot) = self.fonts.get_mut(idx) {
            slot.size_twentieths = Some(size_twentieths);
        }
    }
    pub fn font(&self, idx: u8) -> Option<&FontEntry> {
        self.fonts.get(idx as usize)
    }
}

/// LotAttrWK3 — the 4-byte per-column attribute record carried by Row records
/// (opcode 0xC5). Fields match the layout in `lotattr.hxx`.
#[derive(Debug, Clone, Copy)]
pub struct LotAttr {
    pub font_index: u8,
    pub line_style: u8,
    pub font_color: u8,
    pub back_byte: u8,
}

impl LotAttr {
    pub fn from_bytes(b: [u8; 4]) -> Self {
        Self {
            font_index: b[0],
            line_style: b[1],
            font_color: b[2],
            back_byte: b[3],
        }
    }
    /// True when ANY of the four attribute slots carries non-default data —
    /// matches LO's `LotAttrWK3::HasStyles`.
    pub fn has_styles(&self) -> bool {
        self.font_index != 0
            || self.line_style != 0
            || (self.font_color & 0x07) != 0
            || (self.back_byte & 0xBF) != 0
    }
    pub fn centered(&self) -> bool {
        self.back_byte & 0x80 != 0
    }
}

/// Lotus 8-color CGA palette (white at 0 → black at 7) used for both font
/// colour and background indices. Mirrors `LotAttrCache::pColTab`.
const PALETTE: [&str; 8] = [
    "#FFFFFF", // 0 white
    "#0000FF", // 1 light blue
    "#00FF00", // 2 light green
    "#00FFFF", // 3 light cyan
    "#FF0000", // 4 light red
    "#FF00FF", // 5 light magenta
    "#FFFF00", // 6 yellow
    "#000000", // 7 black
];

fn palette(idx: u8) -> &'static str {
    PALETTE[(idx & 0x07) as usize]
}

/// Translate the low 2 bits of a Lotus border slot into IronCalc's
/// `BorderStyle`. 0 = no line, 1 = thin, 2 = medium, 3 = double-thin.
fn border_style(bits: u8) -> Option<BorderStyle> {
    match bits & 0x03 {
        0 => None,
        1 => Some(BorderStyle::Thin),
        2 => Some(BorderStyle::Medium),
        _ => Some(BorderStyle::Double),
    }
}

fn border_item(bits: u8) -> Option<BorderItem> {
    border_style(bits).map(|style| BorderItem {
        style,
        color: Some("#000000".to_string()),
    })
}

/// Build an IronCalc [`Style`] from a [`LotAttr`] resolved against the
/// supplied font table and (optional) named pattern.
pub fn build_style(
    cache: &StyleCache,
    attr: &LotAttr,
    pattern: Option<&PatternEntry>,
    base: &Style,
) -> Style {
    let mut style = base.clone();

    // Font: pick from the font table (if present) and overlay pattern flags.
    if let Some(font_entry) = cache.font(attr.font_index) {
        if let Some(name) = &font_entry.name {
            if !name.is_empty() {
                style.font.name = name.clone();
            }
        }
        if let Some(size) = font_entry.size_twentieths {
            // Lotus stores size in 1/20th of a point; round to whole points.
            style.font.sz = ((size as i32) + 10) / 20;
        }
        style.font.b = font_entry.bold();
        style.font.i = font_entry.italic();
        style.font.u = font_entry.underline();
    }
    if let Some(p) = pattern {
        if p.bold {
            style.font.b = true;
        }
        if p.italic {
            style.font.i = true;
        }
        if p.underline {
            style.font.u = true;
        }
    }

    // Font colour: 0 = no override; 1..6 = palette; 7 = explicit white.
    let font_col = attr.font_color & 0x07;
    if font_col != 0 {
        style.font.color = Some(palette(font_col).to_string());
    }

    // Background: low 5 bits of nBack pick a palette colour; bit 7 is centered.
    let back = attr.back_byte & 0x1F;
    if back != 0 {
        style.fill = Fill {
            pattern_type: "solid".to_string(),
            fg_color: Some(palette(back & 0x07).to_string()),
            bg_color: Some(palette(back & 0x07).to_string()),
        };
    }

    // Borders: each pair of bits from the low nibble outwards encodes left,
    // right, top, bottom in that order (LO `LotAttrCache::GetPattAttr`).
    if attr.line_style != 0 {
        let left = attr.line_style;
        let right = left >> 2;
        let top = right >> 2;
        let bottom = top >> 2;
        style.border = Border {
            left: border_item(left),
            right: border_item(right),
            top: border_item(top),
            bottom: border_item(bottom),
            ..Default::default()
        };
    }

    // Center alignment from bit 7 of nBack OR from a pattern's H-align byte.
    if attr.centered() || pattern.map(|p| p.h_align).unwrap_or(0) == 3 {
        let mut alignment = style.alignment.clone().unwrap_or_default();
        alignment.horizontal = ironcalc_base::types::HorizontalAlignment::Center;
        style.alignment = Some(alignment);
    }
    if let Some(p) = pattern {
        // Lotus V-align values: 1=Top, 2=Middle, 4=Bottom (else default).
        match p.v_align & 0x07 {
            1 => {
                let mut alignment = style.alignment.clone().unwrap_or_default();
                alignment.vertical = ironcalc_base::types::VerticalAlignment::Top;
                style.alignment = Some(alignment);
            }
            2 => {
                let mut alignment = style.alignment.clone().unwrap_or_default();
                alignment.vertical = ironcalc_base::types::VerticalAlignment::Center;
                style.alignment = Some(alignment);
            }
            4 => {
                let mut alignment = style.alignment.clone().unwrap_or_default();
                alignment.vertical = ironcalc_base::types::VerticalAlignment::Bottom;
                style.alignment = Some(alignment);
            }
            _ => {}
        }
    }

    style
}

/// Apply a constructed [`Style`] to a single cell on `model`.
pub fn apply_style_to_cell(
    model: &mut Model,
    sheet: u32,
    row: u32,
    column: u32,
    style: &Style,
) -> Result<(), String> {
    model.set_cell_style(sheet, row as i32, column as i32, style)
}

/// Decode a Pattern record body (`OP_CreatePattern123`-style). Returns the
/// pattern ID and its parsed entry, or `None` if the body is not a
/// recognised pattern (sub-code != 0x0FD2 or too short).
pub fn decode_pattern(body: &[u8]) -> Option<(u16, PatternEntry)> {
    if body.len() < 22 {
        return None;
    }
    let sub_code = u16::from_le_bytes([body[0], body[1]]);
    if sub_code != 0x0FD2 {
        return None;
    }
    let pattern_id = u16::from_le_bytes([body[2], body[3]]);
    // Bytes 4..16 are reserved per LO; byte 16 is the bold/italic/underline
    // mask; 17..20 reserved; 20 is the H-alignment, 21 is the V-alignment.
    let style_mask = body[16];
    let h_align = body[20];
    let v_align = body[21];
    Some((
        pattern_id,
        PatternEntry {
            bold: style_mask & 0x01 != 0,
            italic: style_mask & 0x02 != 0,
            underline: style_mask & 0x04 != 0,
            h_align,
            v_align,
        },
    ))
}

/// Decode a Font Face record (opcode 174). Format: `idx(u8), name(c-string)`.
pub fn decode_font_face(payload: &[u8], cache: &mut StyleCache) {
    if payload.is_empty() {
        return;
    }
    let idx = payload[0];
    let raw = &payload[1..];
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    cache.set_font_name(idx, cp437_to_string(&raw[..end]));
}

/// Decode a Font Type record (opcode 176). Format: 8 × u16 of type flags
/// (one per font slot).
pub fn decode_font_type(payload: &[u8], cache: &mut StyleCache) {
    for i in 0..FONT_TABLE_SIZE {
        let off = i * 2;
        if off + 2 > payload.len() {
            break;
        }
        let t = u16::from_le_bytes([payload[off], payload[off + 1]]);
        cache.set_font_type(i, t);
    }
}

/// Decode a Font Size record (opcode 177). Same layout as Font Type — 8 × u16
/// values in 1/20th of a point.
pub fn decode_font_size(payload: &[u8], cache: &mut StyleCache) {
    for i in 0..FONT_TABLE_SIZE {
        let off = i * 2;
        if off + 2 > payload.len() {
            break;
        }
        let sz = u16::from_le_bytes([payload[off], payload[off + 1]]);
        cache.set_font_size(i, sz);
    }
}
