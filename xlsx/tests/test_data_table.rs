#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

use std::io::{Cursor, Read, Write};

use ironcalc::export::save_xlsx_to_writer;
use ironcalc::import::load_from_xlsx_bytes;
use ironcalc_base::cell::CellValue;
use ironcalc_base::Model;
use zip::write::FileOptions;

fn add_zip_file(zip: &mut zip::ZipWriter<Cursor<Vec<u8>>>, path: &str, contents: &str) {
    zip.start_file(path, FileOptions::default()).unwrap();
    zip.write_all(contents.as_bytes()).unwrap();
}

fn data_table_xlsx() -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));

    add_zip_file(
        &mut zip,
        "[Content_Types].xml",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
</Types>"#,
    );
    add_zip_file(
        &mut zip,
        "_rels/.rels",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#,
    );
    add_zip_file(
        &mut zip,
        "xl/_rels/workbook.xml.rels",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#,
    );
    add_zip_file(
        &mut zip,
        "xl/workbook.xml",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <bookViews><workbookView activeTab="0"/></bookViews>
  <sheets><sheet name="Sheet" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#,
    );
    add_zip_file(
        &mut zip,
        "xl/styles.xml",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <fonts count="1"><font><sz val="11"/><name val="Calibri"/><family val="2"/></font></fonts>
  <fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills>
  <borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>
  <cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
  <cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs>
  <cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>
</styleSheet>"#,
    );
    add_zip_file(
        &mut zip,
        "xl/worksheets/sheet1.xml",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <dimension ref="A1:B4"/>
  <sheetViews><sheetView workbookViewId="0"><selection activeCell="A1" sqref="A1"/></sheetView></sheetViews>
  <sheetData>
    <row r="1"><c r="A1"><v>0</v></c><c r="B1"><f>A1*2</f><v>0</v></c></row>
    <row r="2"><c r="A2"><v>1</v></c><c r="B2"><f t="dataTable" ref="B2:B4" dt2D="0" dtr="0" r1="A1" ca="1"/><v>999</v></c></row>
    <row r="3"><c r="A3"><v>2</v></c><c r="B3"><v>999</v></c></row>
    <row r="4"><c r="A4"><v>3</v></c><c r="B4"><v>999</v></c></row>
  </sheetData>
</worksheet>"#,
    );

    zip.finish().unwrap().into_inner()
}

fn number_at(model: &ironcalc_base::Model, reference: &str) -> f64 {
    match model.get_cell_value_by_ref(reference).unwrap() {
        CellValue::Number(value) => value,
        other => panic!("{reference} is not a number: {other:?}"),
    }
}

#[test]
fn imports_and_calculates_data_table() {
    let bytes = data_table_xlsx();
    let workbook = load_from_xlsx_bytes(&bytes, "data_table", "en", "UTC").unwrap();
    let mut model = Model::from_workbook(workbook, "en").unwrap();

    assert_eq!(model.workbook.worksheets[0].data_tables.len(), 1);
    model.evaluate();

    assert_eq!(number_at(&model, "Sheet!B2"), 2.0);
    assert_eq!(number_at(&model, "Sheet!B3"), 4.0);
    assert_eq!(number_at(&model, "Sheet!B4"), 6.0);
}

#[test]
fn exports_data_table_formula_anchor() {
    let bytes = data_table_xlsx();
    let workbook = load_from_xlsx_bytes(&bytes, "data_table", "en", "UTC").unwrap();
    let mut model = Model::from_workbook(workbook, "en").unwrap();
    model.evaluate();

    let writer = save_xlsx_to_writer(&model, Cursor::new(Vec::new())).unwrap();
    let mut archive = zip::ZipArchive::new(Cursor::new(writer.into_inner())).unwrap();
    let mut sheet_xml = String::new();
    archive
        .by_name("xl/worksheets/sheet1.xml")
        .unwrap()
        .read_to_string(&mut sheet_xml)
        .unwrap();

    assert!(sheet_xml.contains(r#"<f t="dataTable" ref="B2:B4" dt2D="0" dtr="0" r1="A1" ca="1"/>"#));
    assert!(sheet_xml.contains(r#"<c r="B2"><f t="dataTable""#));
}
