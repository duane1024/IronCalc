//! Convert a Lotus 1-2-3 `.WK3` file to an IronCalc `.ic` snapshot.
//!
//! Mirrors `xlsx`'s `xlsx_2_icalc` binary.

use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use ironcalc_lotus::load_from_wk3;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let Some(input) = args.get(1).cloned() else {
        eprintln!("usage: wk3_2_icalc <file.WK3> [output.ic]");
        return ExitCode::from(2);
    };
    let in_path = Path::new(&input);
    let out_path = match args.get(2) {
        Some(p) => p.clone(),
        None => {
            let stem = in_path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "workbook".to_string());
            format!("{stem}.ic")
        }
    };

    let model = match load_from_wk3(&input, "en", "UTC", "en") {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    // bitcode-encoded `Workbook` is the canonical .ic format (matches
    // xlsx::load_from_icalc on the read side).
    let bytes = bitcode::encode(&model.workbook);
    if let Err(e) = fs::write(&out_path, &bytes) {
        eprintln!("could not write {out_path}: {e}");
        return ExitCode::FAILURE;
    }
    println!("wrote {out_path}");
    ExitCode::SUCCESS
}
