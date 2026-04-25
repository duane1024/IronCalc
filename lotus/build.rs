#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

//! Generate one `#[test]` per `.WK3` file under `tests/wk3/`.
//!
//! Mirrors the pattern used in `xlsx/build.rs`: drop a sample file into the
//! test directory and a corresponding regression test appears automatically.
//! Each generated test loads the file, evaluates the model, and asserts the
//! load + evaluation step do not panic. Per-cell value comparisons against
//! the cached results in the file are deferred until v2 (the cached values
//! are 80-bit long doubles, which we round-trip via f64 — the lossy step
//! makes per-cell equality flaky).

use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::Path;

fn sanitize(stem: &str) -> String {
    stem.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn main() {
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    let dest = Path::new(&out_dir).join("generated_tests.rs");

    println!("cargo:rerun-if-changed=tests/wk3");

    let mut code = String::new();

    let dir_path = Path::new("tests/wk3");
    let mut entries: Vec<_> = match fs::read_dir(dir_path) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(_) => {
            fs::write(&dest, code).expect("write empty generated_tests.rs");
            return;
        }
    };
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let file_name = entry.file_name();
        let file_name_str = file_name.to_string_lossy().to_string();
        if !file_name_str.to_uppercase().ends_with(".WK3") {
            continue;
        }
        let stem = &file_name_str[..file_name_str.len() - 4];
        let fn_name = format!("test_wk3_{}", sanitize(stem).to_lowercase());
        let file_path = format!("tests/wk3/{file_name_str}");

        writeln!(
            code,
            r#"#[allow(non_snake_case)]
#[test]
fn {fn_name}() {{
    let bytes = std::fs::read("{file_path}").unwrap();
    let mut model =
        ironcalc_lotus::load_from_wk3_bytes(&bytes, "{stem}", "en", "UTC", "en")
            .unwrap_or_else(|e| panic!("load failed: {{e}}"));
    model.evaluate();
    assert!(!model.workbook.worksheets.is_empty());
}}
"#
        )
        .unwrap();
    }

    fs::write(&dest, code).expect("failed to write generated_tests.rs");
}
