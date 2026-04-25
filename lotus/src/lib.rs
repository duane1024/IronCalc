//! Lotus 1-2-3 file format import for IronCalc.
//!
//! Currently supports `.WK3` (Lotus 1-2-3 release 3.x for DOS). The crate is
//! laid out so other Lotus formats (WK1, WK4, 123) can be added later, but
//! today only WK3 is recognised.
//!
//! Entry point: [`load_from_wk3`].

pub mod error;
mod import;

pub use error::LotusError;
pub use import::{load_from_wk3, load_from_wk3_bytes};
pub use ironcalc_base as base;
