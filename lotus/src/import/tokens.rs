//! Lotus number decoders used in formula constants and SMALLNUMCELL records.
//!
//! Mirrors `SnumToDouble` and `Snum32ToDouble` in
//! `sc/source/filter/lotus/tool.cxx`.

pub fn snum_to_double(n: i16) -> f64 {
    const FACTORS: [f64; 8] = [
        5000.0, 500.0, 0.05, 0.005, 0.0005, 0.00005, 0.0625, 0.015625,
    ];
    if n & 0x0001 != 0 {
        let factor = FACTORS[((n >> 1) & 0x0007) as usize];
        factor * f64::from(n >> 4)
    } else {
        f64::from(n >> 1)
    }
}

pub fn snum32_to_double(value: u32) -> f64 {
    let mut f = (value >> 6) as f64;
    let exp = value & 0x0f;
    if exp != 0 {
        let mult = 10f64.powi(exp as i32);
        if value & 0x0000_0010 != 0 {
            f /= mult;
        } else {
            f *= mult;
        }
    }
    if value & 0x0000_0020 != 0 {
        f = -f;
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    // SALES.WK3 row 2 col 1 has SMALLNUMCELL bytes 10 27 = 0x2710 = 10000.
    // The actual encoded value is 0x2710 with low bit clear, so >> 1 = 5000.
    #[test]
    fn snum_low_bit_clear_gives_signed_half() {
        assert_eq!(snum_to_double(0x2710), 5000.0);
        assert_eq!(snum_to_double(0), 0.0);
        assert_eq!(snum_to_double(2), 1.0);
    }

    #[test]
    fn snum32_zero() {
        assert_eq!(snum32_to_double(0), 0.0);
    }
}
