//! Decode WK3 tokenised formulas into Excel-compatible formula strings.
//!
//! Mirrors `LotusToSc::Convert` in `sc/source/filter/lotus/lotform.cxx`,
//! using the WK3 token tables (`IndexToTypeWK123` / `IndexToTokenWK123`).
//!
//! Output is plain A1 notation suitable for `Model::set_user_input`. Tokens
//! that have no Excel equivalent are emitted as `LOTUS_FN_<n>` so the file
//! still parses; the cell will evaluate to `#NAME?`.

use crate::error::LotusError;
use crate::import::encoding::cp437_to_string;
use crate::import::tokens::snum32_to_double;

/// Origin of the formula — needed to resolve relative references.
#[derive(Debug, Clone, Copy)]
pub struct FormulaOrigin {
    pub sheet: u8,
    pub row: u16,
    pub column: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arity {
    Op,
    FuncFix(u8),
    FuncVar,
    Neg,
    Nop,
    NotImpl,
}

struct OpInfo {
    name: &'static str,
    /// `None` for prefix functions; `Some` for infix operators (`+`, `-` …).
    infix: Option<&'static str>,
}

const OP_NEG: OpInfo = OpInfo {
    name: "-",
    infix: None,
};

/// Operator/function metadata indexed by the WK3 function-token opcode.
fn op_for(index: u8) -> (Arity, OpInfo) {
    use Arity::*;
    match index {
        14 => (Neg, OP_NEG),
        15 => (
            Op,
            OpInfo {
                name: "+",
                infix: Some("+"),
            },
        ),
        16 => (
            Op,
            OpInfo {
                name: "-",
                infix: Some("-"),
            },
        ),
        17 => (
            Op,
            OpInfo {
                name: "*",
                infix: Some("*"),
            },
        ),
        18 => (
            Op,
            OpInfo {
                name: "/",
                infix: Some("/"),
            },
        ),
        19 => (
            Op,
            OpInfo {
                name: "^",
                infix: Some("^"),
            },
        ),
        20 => (
            Op,
            OpInfo {
                name: "=",
                infix: Some("="),
            },
        ),
        21 => (
            Op,
            OpInfo {
                name: "<>",
                infix: Some("<>"),
            },
        ),
        22 => (
            Op,
            OpInfo {
                name: "<=",
                infix: Some("<="),
            },
        ),
        23 => (
            Op,
            OpInfo {
                name: ">=",
                infix: Some(">="),
            },
        ),
        24 => (
            Op,
            OpInfo {
                name: "<",
                infix: Some("<"),
            },
        ),
        25 => (
            Op,
            OpInfo {
                name: ">",
                infix: Some(">"),
            },
        ),
        // Lotus & = AND, | = OR. Excel has no infix logical ops; rewrite to AND/OR.
        26 => (
            FuncFix(2),
            OpInfo {
                name: "AND",
                infix: None,
            },
        ),
        27 => (
            FuncFix(2),
            OpInfo {
                name: "OR",
                infix: None,
            },
        ),
        28 => (
            FuncFix(1),
            OpInfo {
                name: "NOT",
                infix: None,
            },
        ),
        29 => (
            Nop,
            OpInfo {
                name: "+",
                infix: None,
            },
        ), // unary plus, no-op
        30 => (
            Op,
            OpInfo {
                name: "&",
                infix: Some("&"),
            },
        ),
        31 => (
            FuncFix(0),
            OpInfo {
                name: "NA",
                infix: None,
            },
        ),
        32 => (
            FuncFix(0),
            OpInfo {
                name: "NA",
                infix: None,
            },
        ), // generic error
        33 => (
            FuncFix(1),
            OpInfo {
                name: "ABS",
                infix: None,
            },
        ),
        34 => (
            FuncFix(1),
            OpInfo {
                name: "INT",
                infix: None,
            },
        ),
        35 => (
            FuncFix(1),
            OpInfo {
                name: "SQRT",
                infix: None,
            },
        ),
        36 => (
            FuncFix(1),
            OpInfo {
                name: "LOG10",
                infix: None,
            },
        ),
        37 => (
            FuncFix(1),
            OpInfo {
                name: "LN",
                infix: None,
            },
        ),
        38 => (
            FuncFix(0),
            OpInfo {
                name: "PI",
                infix: None,
            },
        ),
        39 => (
            FuncFix(1),
            OpInfo {
                name: "SIN",
                infix: None,
            },
        ),
        40 => (
            FuncFix(1),
            OpInfo {
                name: "COS",
                infix: None,
            },
        ),
        41 => (
            FuncFix(1),
            OpInfo {
                name: "TAN",
                infix: None,
            },
        ),
        42 => (
            FuncFix(2),
            OpInfo {
                name: "ATAN2",
                infix: None,
            },
        ),
        43 => (
            FuncFix(1),
            OpInfo {
                name: "ATAN",
                infix: None,
            },
        ),
        44 => (
            FuncFix(1),
            OpInfo {
                name: "ASIN",
                infix: None,
            },
        ),
        45 => (
            FuncFix(1),
            OpInfo {
                name: "ACOS",
                infix: None,
            },
        ),
        46 => (
            FuncFix(1),
            OpInfo {
                name: "EXP",
                infix: None,
            },
        ),
        47 => (
            FuncFix(2),
            OpInfo {
                name: "MOD",
                infix: None,
            },
        ),
        48 => (
            FuncVar,
            OpInfo {
                name: "CHOOSE",
                infix: None,
            },
        ),
        49 => (
            FuncFix(1),
            OpInfo {
                name: "ISNA",
                infix: None,
            },
        ),
        50 => (
            FuncFix(1),
            OpInfo {
                name: "ISERROR",
                infix: None,
            },
        ),
        51 => (
            FuncFix(0),
            OpInfo {
                name: "FALSE",
                infix: None,
            },
        ),
        52 => (
            FuncFix(0),
            OpInfo {
                name: "TRUE",
                infix: None,
            },
        ),
        53 => (
            FuncFix(0),
            OpInfo {
                name: "RAND",
                infix: None,
            },
        ),
        54 => (
            FuncFix(3),
            OpInfo {
                name: "DATE",
                infix: None,
            },
        ),
        55 => (
            FuncFix(0),
            OpInfo {
                name: "TODAY",
                infix: None,
            },
        ),
        // Lotus financial functions take args in different order; emit Excel
        // names with a placeholder suffix so users can spot semantic drift.
        56 => (
            FuncFix(3),
            OpInfo {
                name: "_LOTUS_PMT",
                infix: None,
            },
        ),
        57 => (
            FuncFix(3),
            OpInfo {
                name: "_LOTUS_PV",
                infix: None,
            },
        ),
        58 => (
            FuncFix(3),
            OpInfo {
                name: "_LOTUS_FV",
                infix: None,
            },
        ),
        59 => (
            FuncFix(3),
            OpInfo {
                name: "IF",
                infix: None,
            },
        ),
        60 => (
            FuncFix(1),
            OpInfo {
                name: "DAY",
                infix: None,
            },
        ),
        61 => (
            FuncFix(1),
            OpInfo {
                name: "MONTH",
                infix: None,
            },
        ),
        62 => (
            FuncFix(1),
            OpInfo {
                name: "YEAR",
                infix: None,
            },
        ),
        63 => (
            FuncFix(2),
            OpInfo {
                name: "ROUND",
                infix: None,
            },
        ),
        64 => (
            FuncFix(3),
            OpInfo {
                name: "TIME",
                infix: None,
            },
        ),
        65 => (
            FuncFix(1),
            OpInfo {
                name: "HOUR",
                infix: None,
            },
        ),
        66 => (
            FuncFix(1),
            OpInfo {
                name: "MINUTE",
                infix: None,
            },
        ),
        67 => (
            FuncFix(1),
            OpInfo {
                name: "SECOND",
                infix: None,
            },
        ),
        68 => (
            FuncFix(1),
            OpInfo {
                name: "ISNUMBER",
                infix: None,
            },
        ),
        69 => (
            FuncFix(1),
            OpInfo {
                name: "ISTEXT",
                infix: None,
            },
        ),
        70 => (
            FuncFix(1),
            OpInfo {
                name: "LEN",
                infix: None,
            },
        ),
        71 => (
            FuncFix(1),
            OpInfo {
                name: "VALUE",
                infix: None,
            },
        ),
        72 => (
            FuncFix(2),
            OpInfo {
                name: "FIXED",
                infix: None,
            },
        ),
        73 => (
            FuncFix(3),
            OpInfo {
                name: "MID",
                infix: None,
            },
        ),
        74 => (
            FuncFix(1),
            OpInfo {
                name: "CHAR",
                infix: None,
            },
        ),
        75 => (
            FuncFix(1),
            OpInfo {
                name: "CODE",
                infix: None,
            },
        ),
        76 => (
            FuncFix(3),
            OpInfo {
                name: "FIND",
                infix: None,
            },
        ),
        77 => (
            FuncFix(1),
            OpInfo {
                name: "DATEVALUE",
                infix: None,
            },
        ),
        78 => (
            FuncFix(1),
            OpInfo {
                name: "TIMEVALUE",
                infix: None,
            },
        ),
        79 => (
            FuncFix(1),
            OpInfo {
                name: "_LOTUS_CELLPOINTER",
                infix: None,
            },
        ),
        80 => (
            FuncVar,
            OpInfo {
                name: "SUM",
                infix: None,
            },
        ),
        81 => (
            FuncVar,
            OpInfo {
                name: "AVERAGE",
                infix: None,
            },
        ),
        82 => (
            FuncVar,
            OpInfo {
                name: "COUNTA",
                infix: None,
            },
        ),
        83 => (
            FuncVar,
            OpInfo {
                name: "MIN",
                infix: None,
            },
        ),
        84 => (
            FuncVar,
            OpInfo {
                name: "MAX",
                infix: None,
            },
        ),
        85 => (
            FuncFix(3),
            OpInfo {
                name: "VLOOKUP",
                infix: None,
            },
        ),
        86 => (
            FuncFix(2),
            OpInfo {
                name: "NPV",
                infix: None,
            },
        ),
        87 => (
            FuncVar,
            OpInfo {
                name: "VAR",
                infix: None,
            },
        ),
        88 => (
            FuncVar,
            OpInfo {
                name: "STDEV",
                infix: None,
            },
        ),
        89 => (
            FuncFix(2),
            OpInfo {
                name: "IRR",
                infix: None,
            },
        ),
        90 => (
            FuncFix(3),
            OpInfo {
                name: "HLOOKUP",
                infix: None,
            },
        ),
        91 => (
            FuncVar,
            OpInfo {
                name: "DSUM",
                infix: None,
            },
        ),
        92 => (
            FuncVar,
            OpInfo {
                name: "DAVERAGE",
                infix: None,
            },
        ),
        93 => (
            FuncVar,
            OpInfo {
                name: "DCOUNTA",
                infix: None,
            },
        ),
        94 => (
            FuncVar,
            OpInfo {
                name: "DMIN",
                infix: None,
            },
        ),
        95 => (
            FuncVar,
            OpInfo {
                name: "DMAX",
                infix: None,
            },
        ),
        96 => (
            FuncVar,
            OpInfo {
                name: "DVAR",
                infix: None,
            },
        ),
        97 => (
            FuncVar,
            OpInfo {
                name: "DSTDEV",
                infix: None,
            },
        ),
        98 => (
            FuncVar,
            OpInfo {
                name: "INDEX",
                infix: None,
            },
        ),
        99 => (
            FuncFix(1),
            OpInfo {
                name: "COLUMNS",
                infix: None,
            },
        ),
        100 => (
            FuncFix(1),
            OpInfo {
                name: "ROWS",
                infix: None,
            },
        ),
        101 => (
            FuncFix(2),
            OpInfo {
                name: "REPT",
                infix: None,
            },
        ),
        102 => (
            FuncFix(1),
            OpInfo {
                name: "UPPER",
                infix: None,
            },
        ),
        103 => (
            FuncFix(1),
            OpInfo {
                name: "LOWER",
                infix: None,
            },
        ),
        104 => (
            FuncFix(2),
            OpInfo {
                name: "LEFT",
                infix: None,
            },
        ),
        105 => (
            FuncFix(2),
            OpInfo {
                name: "RIGHT",
                infix: None,
            },
        ),
        106 => (
            FuncFix(4),
            OpInfo {
                name: "REPLACE",
                infix: None,
            },
        ),
        107 => (
            FuncFix(1),
            OpInfo {
                name: "PROPER",
                infix: None,
            },
        ),
        108 => (
            FuncFix(2),
            OpInfo {
                name: "CELL",
                infix: None,
            },
        ),
        109 => (
            FuncFix(1),
            OpInfo {
                name: "TRIM",
                infix: None,
            },
        ),
        110 => (
            FuncFix(1),
            OpInfo {
                name: "CLEAN",
                infix: None,
            },
        ),
        111 => (
            FuncFix(1),
            OpInfo {
                name: "T",
                infix: None,
            },
        ),
        112 => (
            FuncFix(1),
            OpInfo {
                name: "N",
                infix: None,
            },
        ),
        113 => (
            FuncFix(2),
            OpInfo {
                name: "EXACT",
                infix: None,
            },
        ),
        115 => (
            FuncFix(1),
            OpInfo {
                name: "INDIRECT",
                infix: None,
            },
        ),
        116 => (
            FuncFix(3),
            OpInfo {
                name: "RATE",
                infix: None,
            },
        ),
        117 => (
            FuncFix(3),
            OpInfo {
                name: "_LOTUS_TERM",
                infix: None,
            },
        ),
        118 => (
            FuncFix(3),
            OpInfo {
                name: "_LOTUS_CTERM",
                infix: None,
            },
        ),
        119 => (
            FuncFix(3),
            OpInfo {
                name: "SLN",
                infix: None,
            },
        ),
        120 => (
            FuncFix(4),
            OpInfo {
                name: "SYD",
                infix: None,
            },
        ),
        121 => (
            FuncFix(4),
            OpInfo {
                name: "DDB",
                infix: None,
            },
        ),
        125 => (
            FuncVar,
            OpInfo {
                name: "SUMPRODUCT",
                infix: None,
            },
        ),
        137 => (
            FuncFix(2),
            OpInfo {
                name: "DAYS360",
                infix: None,
            },
        ),
        141 => (
            FuncFix(1),
            OpInfo {
                name: "WEEKDAY",
                infix: None,
            },
        ),
        142 => (
            FuncFix(3),
            OpInfo {
                name: "DATEDIF",
                infix: None,
            },
        ),
        143 => (
            FuncVar,
            OpInfo {
                name: "RANK",
                infix: None,
            },
        ),
        153 => (
            FuncVar,
            OpInfo {
                name: "AVERAGE",
                infix: None,
            },
        ),
        154 => (
            FuncVar,
            OpInfo {
                name: "COUNT",
                infix: None,
            },
        ),
        155 => (
            FuncVar,
            OpInfo {
                name: "MAX",
                infix: None,
            },
        ),
        156 => (
            FuncVar,
            OpInfo {
                name: "MIN",
                infix: None,
            },
        ),
        157 => (
            FuncVar,
            OpInfo {
                name: "STDEVP",
                infix: None,
            },
        ),
        158 => (
            FuncVar,
            OpInfo {
                name: "VARP",
                infix: None,
            },
        ),
        159 => (
            FuncVar,
            OpInfo {
                name: "STDEV",
                infix: None,
            },
        ),
        160 => (
            FuncVar,
            OpInfo {
                name: "VAR",
                infix: None,
            },
        ),
        166 => (
            FuncFix(2),
            OpInfo {
                name: "DAYS360",
                infix: None,
            },
        ),
        _ => (
            NotImpl,
            OpInfo {
                name: "",
                infix: None,
            },
        ),
    }
}

/// Walks the formula token stream and returns an Excel-style formula
/// (without the leading `=`) plus the number of bytes consumed.
pub fn decode_formula(bytes: &[u8], origin: FormulaOrigin) -> Result<(String, usize), LotusError> {
    let mut r = Cursor::new(bytes);
    let mut stack: Vec<String> = Vec::new();

    loop {
        let nop = r.read_u8()?;
        match nop {
            // FT_Const10Float — 8-byte IEEE double in WK123 mode
            0 => {
                let v = r.read_f64()?;
                stack.push(format_number(v));
            }
            // FT_Cref — cell reference (1 rel byte, then row/tab/col are absolute)
            1 => {
                let rel = r.read_u8()?;
                let row = r.read_u16()?;
                let tab = r.read_u8()?;
                let col = r.read_u8()?;
                stack.push(format_cref(rel, row, tab, col, origin));
            }
            // FT_Rref — range reference: 1 rel byte (low 3 bits → endpoint 1,
            // next 3 bits → endpoint 2), then two 4-byte addresses.
            2 => {
                let rel = r.read_u8()?;
                let row1 = r.read_u16()?;
                let tab1 = r.read_u8()?;
                let col1 = r.read_u8()?;
                let row2 = r.read_u16()?;
                let tab2 = r.read_u8()?;
                let col2 = r.read_u8()?;
                let a = format_cref(rel, row1, tab1, col1, origin);
                let b = format_cref(rel >> 3, row2, tab2, col2, origin);
                stack.push(format!("{a}:{b}"));
            }
            // FT_Return
            3 => break,
            // FT_Braces — wrap top of stack in parens
            4 => {
                let top = pop(&mut stack, origin, "(")?;
                stack.push(format!("({top})"));
            }
            // FT_Snum — packed 32-bit number
            5 => {
                let raw = r.read_u32()?;
                stack.push(format_number(snum32_to_double(raw)));
            }
            // FT_ConstString — null-terminated string
            6 => {
                let s = r.read_cstr()?;
                let escaped = s.replace('"', "\"\"");
                stack.push(format!("\"{escaped}\""));
            }
            // FT_Nrref — relative named range
            7 => {
                let s = r.read_cstr()?;
                stack.push(sanitize_name(&s));
            }
            // FT_Absnref — absolute named range (leading $ stripped)
            8 => {
                let s = r.read_cstr()?;
                let trimmed = s.strip_prefix('$').unwrap_or(&s);
                stack.push(sanitize_name(trimmed));
            }
            // FT_Erref / FT_Ecref / FT_Econstant — error placeholders
            9 => {
                r.skip(4)?;
                stack.push("#REF!".into());
            }
            10 => {
                r.skip(5)?;
                stack.push("#REF!".into());
            }
            11 => {
                r.skip(10)?;
                stack.push("#N/A".into());
            }
            // 12, 13 reserved
            12 | 13 => {}
            // 14..=30 are operators / NOPs / negation
            n if (14..=30).contains(&n) => {
                let (arity, op) = op_for(n);
                apply(&mut stack, arity, &op, origin)?;
            }
            // 31..=255 are functions
            n => {
                let (arity, op) = op_for(n);
                let arg_count = match arity {
                    Arity::FuncVar => r.read_u8()?,
                    Arity::FuncFix(c) => c,
                    Arity::Nop => continue,
                    Arity::Neg => 1,
                    Arity::Op => unreachable!("op handled above"),
                    Arity::NotImpl => 0,
                };
                if matches!(arity, Arity::NotImpl) {
                    // Unknown function — pop nothing, push a placeholder
                    // that produces #NAME? when evaluated.
                    stack.push(format!("LOTUS_FN_{n}()"));
                    continue;
                }
                apply_func(&mut stack, &op, arg_count)?;
            }
        }
    }

    let consumed = r.position();
    let result = stack.pop().ok_or_else(|| LotusError::Formula {
        sheet: origin.sheet as u32,
        row: origin.row as i32,
        column: origin.column as i32,
        reason: "empty formula stack".into(),
    })?;
    Ok((result, consumed))
}

fn apply(
    stack: &mut Vec<String>,
    arity: Arity,
    op: &OpInfo,
    origin: FormulaOrigin,
) -> Result<(), LotusError> {
    match arity {
        Arity::Op => {
            let b = pop(stack, origin, op.name)?;
            let a = pop(stack, origin, op.name)?;
            let infix = op.infix.unwrap_or(op.name);
            stack.push(format!("({a}{infix}{b})"));
        }
        Arity::FuncFix(n) => apply_func(stack, op, n)?,
        Arity::FuncVar => {} // handled inline
        Arity::Neg => {
            let a = pop(stack, origin, "-")?;
            stack.push(format!("(-{a})"));
        }
        Arity::Nop => {}
        Arity::NotImpl => {}
    }
    Ok(())
}

fn apply_func(stack: &mut Vec<String>, op: &OpInfo, n: u8) -> Result<(), LotusError> {
    let n = n as usize;
    if stack.len() < n {
        // Defensive: malformed input. Pop what we can.
        let take = stack.len();
        let mut args: Vec<String> = stack.drain(stack.len() - take..).collect();
        args.reverse();
        let joined = args.join(",");
        stack.push(format!("{}({joined})", op.name));
        return Ok(());
    }
    let mut args: Vec<String> = stack.drain(stack.len() - n..).collect();
    // Tokens are pushed in source order; for a fixed-arity call the rightmost
    // arg is on top of the stack, so the drain order is already left→right.
    let _ = &mut args; // (no reorder needed)
    let joined = args.join(",");
    stack.push(format!("{}({joined})", op.name));
    Ok(())
}

fn pop(stack: &mut Vec<String>, origin: FormulaOrigin, ctx: &str) -> Result<String, LotusError> {
    stack.pop().ok_or_else(|| LotusError::Formula {
        sheet: origin.sheet as u32,
        row: origin.row as i32,
        column: origin.column as i32,
        reason: format!("stack underflow at '{ctx}'"),
    })
}

fn format_number(v: f64) -> String {
    // Use the shortest representation that round-trips. IronCalc's parser
    // accepts standard f64 syntax.
    if v == v.trunc() && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let s = format!("{v}");
        // Avoid emitting "inf"/"NaN" — clamp to error.
        if !v.is_finite() {
            return "#NUM!".into();
        }
        s
    }
}

fn sanitize_name(name: &str) -> String {
    // Excel-style defined names: replace invalid chars with `_`.
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

/// Render a Lotus cell reference. The on-disk row/tab/col bytes always store
/// the absolute address; the `rel` bits control whether the rendered Excel
/// reference uses `$` markers.
///
/// Bits in `rel` (low 3 bits): bit 0 = column relative, bit 1 = row relative,
/// bit 2 = sheet relative. (For range endpoints 2, the same encoding is used
/// after caller shifts the byte right by 3.)
fn format_cref(rel: u8, row: u16, tab: u8, col: u8, origin: FormulaOrigin) -> String {
    let col_rel = (rel & 0x01) != 0;
    let row_rel = (rel & 0x02) != 0;
    let tab_rel = (rel & 0x04) != 0;
    let _ = tab_rel; // not surfaced in the rendered string

    let mut out = String::new();
    if tab != origin.sheet {
        out.push_str(&format!("__LOTUS_TAB{tab}__!"));
    }
    if !col_rel {
        out.push('$');
    }
    out.push_str(&col_letters(col as u32));
    if !row_rel {
        out.push('$');
    }
    out.push_str(&((row as u32) + 1).to_string());
    out
}

fn col_letters(mut c: u32) -> String {
    let mut out = String::new();
    c += 1;
    while c > 0 {
        let r = ((c - 1) % 26) as u8;
        out.insert(0, (b'A' + r) as char);
        c = (c - 1) / 26;
    }
    out
}

/// Tiny LE byte-slice cursor — no `byteorder` dep needed.
struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    fn position(&self) -> usize {
        self.pos
    }
    fn read_u8(&mut self) -> Result<u8, LotusError> {
        if self.pos >= self.buf.len() {
            return Err(LotusError::IO("formula token cursor underflow".into()));
        }
        let b = self.buf[self.pos];
        self.pos += 1;
        Ok(b)
    }
    fn read_u16(&mut self) -> Result<u16, LotusError> {
        if self.pos + 2 > self.buf.len() {
            return Err(LotusError::IO("formula u16 underflow".into()));
        }
        let v = u16::from_le_bytes([self.buf[self.pos], self.buf[self.pos + 1]]);
        self.pos += 2;
        Ok(v)
    }
    fn read_u32(&mut self) -> Result<u32, LotusError> {
        if self.pos + 4 > self.buf.len() {
            return Err(LotusError::IO("formula u32 underflow".into()));
        }
        let mut a = [0u8; 4];
        a.copy_from_slice(&self.buf[self.pos..self.pos + 4]);
        self.pos += 4;
        Ok(u32::from_le_bytes(a))
    }
    fn read_f64(&mut self) -> Result<f64, LotusError> {
        if self.pos + 8 > self.buf.len() {
            return Err(LotusError::IO("formula f64 underflow".into()));
        }
        let mut a = [0u8; 8];
        a.copy_from_slice(&self.buf[self.pos..self.pos + 8]);
        self.pos += 8;
        Ok(f64::from_le_bytes(a))
    }
    fn read_cstr(&mut self) -> Result<String, LotusError> {
        let start = self.pos;
        while self.pos < self.buf.len() && self.buf[self.pos] != 0 {
            self.pos += 1;
        }
        let s = cp437_to_string(&self.buf[start..self.pos]);
        if self.pos < self.buf.len() {
            self.pos += 1; // consume the null terminator
        }
        Ok(s)
    }
    fn skip(&mut self, n: usize) -> Result<(), LotusError> {
        if self.pos + n > self.buf.len() {
            return Err(LotusError::IO("formula skip past end".into()));
        }
        self.pos += n;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn origin(row: u16, col: u8) -> FormulaOrigin {
        FormulaOrigin {
            sheet: 0,
            row,
            column: col,
        }
    }

    #[test]
    fn col_letters_basic() {
        assert_eq!(col_letters(0), "A");
        assert_eq!(col_letters(25), "Z");
        assert_eq!(col_letters(26), "AA");
        assert_eq!(col_letters(701), "ZZ");
    }

    #[test]
    fn absolute_cref() {
        // Cref token: rel=0, row=0, tab=0, col=2  →  $C$1
        let bytes = [
            0x01, 0x00, 0x00, 0x00, 0x00, 0x02, // Cref(rel=0, row=0, tab=0, col=2)
            0x03, // Return
        ];
        let (s, _) = decode_formula(&bytes, origin(0, 0)).unwrap();
        assert_eq!(s, "$C$1");
    }

    #[test]
    fn binary_addition_constants() {
        // Const(2.0) + Const(3.0)
        let mut bytes: Vec<u8> = vec![0x00];
        bytes.extend(2.0f64.to_le_bytes());
        bytes.push(0x00);
        bytes.extend(3.0f64.to_le_bytes());
        bytes.push(0x0F); // FT_Op +
        bytes.push(0x03); // Return
        let (s, _) = decode_formula(&bytes, origin(0, 0)).unwrap();
        assert_eq!(s, "(2+3)");
    }

    #[test]
    fn sum_var_arity() {
        // SUM(2,3): two consts then 0x50 + count=2
        let mut bytes: Vec<u8> = Vec::new();
        bytes.push(0x00);
        bytes.extend(2.0f64.to_le_bytes());
        bytes.push(0x00);
        bytes.extend(3.0f64.to_le_bytes());
        bytes.push(80); // SUM
        bytes.push(2); // arg count
        bytes.push(0x03); // Return
        let (s, _) = decode_formula(&bytes, origin(0, 0)).unwrap();
        assert_eq!(s, "SUM(2,3)");
    }
}
