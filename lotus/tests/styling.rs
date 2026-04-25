//! Synthetic styling tests.
//!
//! None of the bundled DOS-era WK3 files contain any of the styling records
//! (fonts 174/176/177, CreatePattern 0x1B sub-code 0x0FD2, per-row 0xC5).
//! These tests build a tiny WK3-shaped byte stream by hand to exercise the
//! styling pipeline so it doesn't bit-rot. The byte layouts mirror those in
//! `sc/source/filter/lotus/lotattr.cxx` and `lotimpop.cxx::Row_`.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

use ironcalc_lotus::base::types::{BorderStyle, HorizontalAlignment};
use ironcalc_lotus::load_from_wk3_bytes;

/// 26-byte WK3 BOF body marking this as Lotus 1-2-3 R3 (file_code=0x1000,
/// file_sub=0x0004). The remaining bytes are zero — they describe the
/// active range and metadata none of which we read on import.
fn bof_body() -> [u8; 26] {
    let mut b = [0u8; 26];
    b[0..2].copy_from_slice(&0x1000u16.to_le_bytes()); // file_code
    b[2..4].copy_from_slice(&0x0004u16.to_le_bytes()); // file_sub
    b
}

fn write_record(out: &mut Vec<u8>, opcode: u16, body: &[u8]) {
    out.extend_from_slice(&opcode.to_le_bytes());
    out.extend_from_slice(&(body.len() as u16).to_le_bytes());
    out.extend_from_slice(body);
}

/// Build a minimal WK3 byte stream containing:
///   - BOF
///   - LABELCELL at (0,0,0) "Hello"
///   - Font Face record (174): font 0 = "Courier"
///   - Font Type record (176): font 0 has bold (bit 0)
///   - Font Size record (177): font 0 = 14pt (in 1/20ths = 280)
///   - Row record (0xC5) for row 0: applies LotAttrWK3 with font_index=0
///     and a thin border on all four sides to columns 0..2
///   - EOF
fn synthetic_styled_wk3() -> Vec<u8> {
    let mut out = Vec::new();
    write_record(&mut out, 0x0000, &bof_body());

    // LABELCELL at row=0, tab=0, col=0, align=', text="Hello"
    let mut label = Vec::new();
    label.extend_from_slice(&0u16.to_le_bytes()); // row
    label.push(0); // tab
    label.push(0); // col
    label.push(b'\''); // alignment prefix (left)
    label.extend_from_slice(b"Hello\0");
    write_record(&mut out, 0x0016, &label);

    // FONT_FACE: idx=0, name="Courier\0"
    let mut font_face = Vec::new();
    font_face.push(0); // font index
    font_face.extend_from_slice(b"Courier\0");
    write_record(&mut out, 174, &font_face);

    // FONT_TYPE: 8 × u16 — slot 0 has bold flag (0x0001)
    let mut font_type = vec![0u8; 16];
    font_type[0..2].copy_from_slice(&0x0001u16.to_le_bytes());
    write_record(&mut out, 176, &font_type);

    // FONT_SIZE: 8 × u16 — slot 0 = 280 (14pt × 20)
    let mut font_size = vec![0u8; 16];
    font_size[0..2].copy_from_slice(&280u16.to_le_bytes());
    write_record(&mut out, 177, &font_size);

    // ROW_RECORD (0xC5): row=0, height=0, then one (LotAttrWK3, repeats=1) entry
    // covering columns 0 and 1.
    //   LotAttrWK3 = font_index=0, line_style=0b01010101 (thin on all sides),
    //                font_color=4 (red), back_byte=0x82 (centered + back palette 2)
    let mut row_record = Vec::new();
    row_record.extend_from_slice(&0u16.to_le_bytes()); // row=0
    row_record.extend_from_slice(&0u16.to_le_bytes()); // height=0
    row_record.push(0); // font_index
    row_record.push(0b01010101); // line_style: 01 left, 01 right, 01 top, 01 bottom (all thin)
    row_record.push(4); // font_color (palette index 4 = red)
    row_record.push(0x82); // back_byte: bit 7 = centered, low bits = palette 2
    row_record.push(1); // repeats=1 → covers columns 0 and 1
    write_record(&mut out, 0x00C5, &row_record);

    // EOF
    write_record(&mut out, 0x0001, &[]);
    out
}

#[test]
fn synthetic_styling_pipeline() {
    let bytes = synthetic_styled_wk3();
    let model = load_from_wk3_bytes(&bytes, "synthetic", "en", "UTC", "en")
        .expect("load synthetic WK3 bytes");

    // The label cell loaded fine.
    let val = model.get_cell_value_by_index(0, 1, 1).unwrap();
    match val {
        ironcalc_lotus::base::cell::CellValue::String(s) => assert_eq!(s, "Hello"),
        other => panic!("expected 'Hello' at A1, got {other:?}"),
    }

    // Style at A1 should have:
    //   - font.name = Courier, font.b = true, font.sz = 14
    //   - red font color, palette-2 background
    //   - thin borders on all four sides
    //   - centered horizontal alignment
    let style = model.get_style_for_cell(0, 1, 1).expect("style A1");
    assert_eq!(style.font.name, "Courier", "font name");
    assert!(style.font.b, "bold");
    assert_eq!(style.font.sz, 14, "font size");

    assert_eq!(
        style.border.left.as_ref().map(|b| &b.style),
        Some(&BorderStyle::Thin),
        "left border"
    );
    assert_eq!(
        style.border.right.as_ref().map(|b| &b.style),
        Some(&BorderStyle::Thin),
        "right border"
    );
    assert_eq!(
        style.border.top.as_ref().map(|b| &b.style),
        Some(&BorderStyle::Thin),
        "top border"
    );
    assert_eq!(
        style.border.bottom.as_ref().map(|b| &b.style),
        Some(&BorderStyle::Thin),
        "bottom border"
    );

    // back_byte = 0x82: low 5 bits = 2 → light green palette.
    assert_eq!(
        style.fill.fg_color.as_deref(),
        Some("#00FF00"),
        "background"
    );

    // Centered alignment from bit 7 of nBack.
    assert_eq!(
        style
            .alignment
            .as_ref()
            .map(|a| a.horizontal.clone())
            .unwrap_or_default(),
        HorizontalAlignment::Center
    );

    // The repeat=1 means the same style applies to A1 and B1.
    let style_b1 = model.get_style_for_cell(0, 1, 2).expect("style B1");
    assert_eq!(style_b1.font.name, "Courier");
    assert!(style_b1.font.b);
}
