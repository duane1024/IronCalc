#![allow(clippy::unwrap_used, clippy::expect_used)]

// Regression coverage for clipping whole-column/row CF ranges to the sheet's
// used dimension (`clip_ranges_to_dimension` in `conditional_formatting.rs`).
// See that function's doc comment for exactly what is (and deliberately isn't)
// clipped, and why.

use crate::{
    cf_types::{CfRuleInput, Cfvo, ColorScaleThreshold},
    test::util::new_empty_model,
    types::{Color, Dxf, Fill},
};

fn color_scale_rule() -> CfRuleInput {
    CfRuleInput::ColorScale {
        thresholds: vec![
            ColorScaleThreshold {
                cfvo: Cfvo::Min,
                color: Color::Rgb("#FF0000".to_string()),
            },
            ColorScaleThreshold {
                cfvo: Cfvo::Max,
                color: Color::Rgb("#00FF00".to_string()),
            },
        ],
    }
}

fn red_fill() -> Dxf {
    Dxf {
        fill: Some(Fill {
            color: Color::Rgb("#FF0000".to_string()),
        }),
        ..Dxf::default()
    }
}

/// The motivating regression: a whole-column ColorScale rule over a sheet with
/// data only in A1:C5 must (a) render exactly as it did before clipping was
/// added, and (b) not leave `cf_cache` entries for any row beyond the sheet's
/// used dimension -- that unclipped cache is the actual memory blowup.
#[test]
fn whole_column_color_scale_visible_result_unchanged_and_cache_stays_bounded() {
    let mut model = new_empty_model();
    for row in 1i32..=5 {
        for col in 1i32..=3 {
            model
                .set_user_input(0, row, col, (row * 10 + col).to_string())
                .unwrap();
        }
    }
    model.evaluate();

    model
        .add_conditional_formatting(0, "A1:A1048576", color_scale_rule())
        .unwrap();
    model.evaluate();

    // (a) Visible result inside the used range is unchanged: A1 is the min (red),
    // A5 is the max (green).
    let style_a1 = model.get_extended_style_for_cell(0, 1, 1).unwrap();
    assert_eq!(style_a1.style.fill.color, Color::Rgb("#FF0000".to_string()));
    let style_a5 = model.get_extended_style_for_cell(0, 5, 1).unwrap();
    assert_eq!(style_a5.style.fill.color, Color::Rgb("#00FF00".to_string()));

    // (b) The actual regression guard: cf_cache must not hold entries beyond the
    // used dimension (max_row = 5), even though the rule's literal declared
    // range extends to row 1,048,576.
    let max_cached_row = model
        .cf_cache
        .keys()
        .filter(|(sheet, _, _)| *sheet == 0)
        .map(|(_, row, _)| *row)
        .max()
        .unwrap_or(0);
    assert!(
        max_cached_row <= 5,
        "cf_cache must not hold entries beyond the used dimension, got row {max_cached_row}"
    );
}

/// A CF rule declared over an entirely empty sheet must not evaluate at all --
/// not even over the phantom (1,1,1,1) placeholder `Worksheet::dimension()`
/// returns for "no used cells".
#[test]
fn rule_over_a_genuinely_empty_sheet_produces_no_cache_entries() {
    let mut model = new_empty_model();
    model
        .add_conditional_formatting(0, "A1:A1048576", color_scale_rule())
        .unwrap();
    model.evaluate();

    assert!(
        model.cf_cache.is_empty(),
        "a rule over a genuinely empty sheet must not populate cf_cache"
    );
}

/// Scope guard: an ordinary bounded Blanks rule (not a whole-column/row
/// declaration) must keep matching blank cells that sit past the sheet's last
/// populated row -- these are commonly deliberate ("flag any blanks in this
/// column"), and Blanks keys off the absence of content, so the "no content =
/// no result" reasoning behind clipping does not apply to it. Only whole-
/// column/row ranges are clipped; this range (A1:A5, not a whole column) is
/// left untouched.
#[test]
fn bounded_blanks_rule_still_flags_blanks_past_the_last_populated_row() {
    let mut model = new_empty_model();
    // A1=1, A2=2, A3 empty, A4=4 -- last populated row is 4; A5 stays blank.
    model.set_user_input(0, 1, 1, "1".to_string()).unwrap();
    model.set_user_input(0, 2, 1, "2".to_string()).unwrap();
    model.set_user_input(0, 4, 1, "4".to_string()).unwrap();
    model.evaluate();

    model
        .add_conditional_formatting(
            0,
            "A1:A5",
            CfRuleInput::Blanks {
                format: red_fill(),
                stop_if_true: false,
            },
        )
        .unwrap();
    model.evaluate();

    // A5 is past the used dimension (max_row = 4) but the declared range (A1:A5)
    // is not a whole-column rule, so it must still be flagged as blank.
    let style_a5 = model.get_extended_style_for_cell(0, 5, 1).unwrap();
    assert_eq!(
        style_a5.style.fill.color,
        Color::Rgb("#FF0000".to_string()),
        "A5 (blank, past the used dimension) must still be flagged by a bounded Blanks rule"
    );
}
