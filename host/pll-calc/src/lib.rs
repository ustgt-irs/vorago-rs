//! Copy of the PLL calculation in `va416xx-hal/src/clock.rs`, so it can be tested on the host.
//!
//! The HAL crate can only be built for the target. Keep this file in sync with the original.

/// Stand-in for the HAL frequency type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hertz(u32);

impl Hertz {
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn to_raw(self) -> u32 {
        self.0
    }
}

/// PLL configuration, see [ClockConfigurator::pll_cfg].
///
/// The PLL divides the input by the reference divider, multiplies it with the feedback divider
/// to get the VCO frequency and divides that by the output divider. This is the generic formula
/// of an integer-N PLL with internal feedback:
///
/// `output = input * (clkf + 1) / ((clkr + 1) * (clkod + 1))`
///
/// The field names are the register names of the CLKGEN peripheral. Like in the registers, the
/// dividers are stored as the actual divider minus one.
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq)]
pub struct PllConfig {
    /// Reference divider, `PLL_CLKR`. The input is divided by `clkr + 1`, range 1 to 16.
    pub clkr: u8,
    /// Feedback divider, `PLL_CLKF`. The VCO runs at `clkf + 1` times the reference frequency,
    /// range 1 to 64.
    pub clkf: u8,
    /// Output divider, `PLL_CLKOD`. The VCO frequency is divided by `clkod + 1`, range 1 to 16.
    pub clkod: u8,
    /// Bandwidth adjustment, `PLL_BWADJ`.
    pub bwadj: u8,
}

/// Maximum input frequency of the PLL, see section 6.4 of the datasheet.
const PLL_IN_MAX_HZ: u64 = 100_000_000;
/// Maximum output frequency of the PLL, see section 6.4 of the datasheet.
const PLL_OUT_MAX_HZ: u64 = 100_000_000;
/// Frequency range of the internal VCO. Taken from `VCO_MIN` and `VCO_MAX` of the PLL
/// calculation in the Vorago C HAL.
const PLL_VCO_MIN_HZ: u64 = 110_000_000;
const PLL_VCO_MAX_HZ: u64 = 550_000_000;
/// Minimum frequency at the phase frequency detector, after the reference divider. Taken from
/// `REF_MIN` in the Vorago C HAL. The datasheet lists no minimum input frequency, only 4 MHz as
/// a typical value. This is also the lower limit for the input frequency.
const PLL_REF_MIN_HZ: u64 = 4_296_880;

/// Error type for [PllConfig::calculate].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PllCalcError {
    /// The input frequency is below 4.29688 MHz or above 100 MHz.
    InputFrequencyOutOfRange,
    /// The output frequency is zero or above 100 MHz.
    OutputFrequencyOutOfRange,
    /// No divider combination keeps all parts of the PLL inside their limits.
    NoValidConfig,
}

impl PllConfig {
    /// Frequency of the PLL output for a given input frequency.
    ///
    /// The result saturates at [u32::MAX] for dividers which are out of range.
    pub const fn output_freq(&self, input: Hertz) -> Hertz {
        let ref_div = self.clkr as u64 + 1;
        let fb_div = self.clkf as u64 + 1;
        let out_div = self.clkod as u64 + 1;
        let out = input.to_raw() as u64 * fb_div / (ref_div * out_div);
        if out > u32::MAX as u64 {
            return Hertz::from_raw(u32::MAX);
        }
        Hertz::from_raw(out as u32)
    }

    /// Find the divider settings which get closest to the requested output frequency.
    ///
    /// The result is not necessarily exact. Use [Self::output_freq] to get the actual frequency.
    ///
    /// Candidates are compared in this order, a later criterion never overrides an earlier one:
    ///
    /// 1. Smallest frequency error.
    /// 2. Lowest reference divider. This gives the highest reference frequency at the phase
    ///    detector, which minimizes the cycle-to-cycle jitter according to the programmers guide.
    /// 3. Highest feedback divider. For the same reference divider and output frequency, this is
    ///    the highest VCO frequency like in the Vorago examples and C HAL. The programmers guide
    ///    only says that a lower VCO frequency saves power.
    ///
    /// The bandwidth adjustment is set to the feedback divider like in the examples from
    /// Vorago. The programmers guide says that the lowest possible value minimizes the
    /// long-term jitter. The valid lower limit is not known.
    pub fn calculate(input: Hertz, output: Hertz) -> Result<Self, PllCalcError> {
        let input_hz = input.to_raw() as u64;
        let target_hz = output.to_raw() as u64;
        if !(PLL_REF_MIN_HZ..=PLL_IN_MAX_HZ).contains(&input_hz) {
            return Err(PllCalcError::InputFrequencyOutOfRange);
        }
        if target_hz == 0 || target_hz > PLL_OUT_MAX_HZ {
            return Err(PllCalcError::OutputFrequencyOutOfRange);
        }
        // The frequency error is kept as a fraction to avoid rounding:
        // error_hz = err_scaled / (ref_div * out_div)
        struct Candidate {
            err_scaled: u64,
            ref_div: u64,
            fb_div: u64,
            out_div: u64,
        }
        let mut best: Option<Candidate> = None;
        for ref_div in 1..=16_u64 {
            if input_hz < PLL_REF_MIN_HZ * ref_div {
                break;
            }
            for fb_div in 1..=64_u64 {
                let vco_hz = input_hz * fb_div / ref_div;
                if !(PLL_VCO_MIN_HZ..=PLL_VCO_MAX_HZ).contains(&vco_hz) {
                    continue;
                }
                for out_div in 1..=16_u64 {
                    let total_div = ref_div * out_div;
                    // Output frequency multiplied by total_div.
                    let out_scaled = input_hz * fb_div;
                    if out_scaled > PLL_OUT_MAX_HZ * total_div {
                        continue;
                    }
                    let err_scaled = out_scaled.abs_diff(target_hz * total_div);
                    let is_better = match &best {
                        None => true,
                        Some(b) => {
                            let cand_err = err_scaled * b.ref_div * b.out_div;
                            let best_err = b.err_scaled * total_div;
                            // For equal errors, prefer the lower reference divider. For an equal
                            // reference divider, a higher feedback divider means a higher VCO
                            // frequency.
                            cand_err < best_err
                                || (cand_err == best_err
                                    && (ref_div < b.ref_div
                                        || (ref_div == b.ref_div && fb_div > b.fb_div)))
                        }
                    };
                    if is_better {
                        best = Some(Candidate {
                            err_scaled,
                            ref_div,
                            fb_div,
                            out_div,
                        });
                    }
                }
            }
        }
        let best = best.ok_or(PllCalcError::NoValidConfig)?;
        Ok(Self {
            clkr: (best.ref_div - 1) as u8,
            clkf: (best.fb_div - 1) as u8,
            clkod: (best.out_div - 1) as u8,
            bwadj: (best.fb_div - 1) as u8,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INPUT: Hertz = Hertz::from_raw(10_000_000);

    /// Examples from Vorago for a 10 MHz input.
    #[test]
    fn vorago_example_100_mhz() {
        let cfg = PllConfig::calculate(INPUT, Hertz::from_raw(100_000_000)).unwrap();
        assert_eq!(
            cfg,
            PllConfig {
                clkr: 0b0000,
                clkf: 0x31,
                clkod: 0b0100,
                bwadj: 0x31,
            }
        );
        assert_eq!(cfg.output_freq(INPUT), Hertz::from_raw(100_000_000));
    }

    #[test]
    fn vorago_example_50_mhz() {
        let cfg = PllConfig::calculate(INPUT, Hertz::from_raw(50_000_000)).unwrap();
        assert_eq!(
            cfg,
            PllConfig {
                clkr: 0b0000,
                clkf: 0x36,
                clkod: 0b1010,
                bwadj: 0x36,
            }
        );
        assert_eq!(cfg.output_freq(INPUT), Hertz::from_raw(50_000_000));
    }

    /// External clock of the PEB1 board, used by the UART example.
    #[test]
    fn peb1_extclk_100_mhz() {
        let input = Hertz::from_raw(40_000_000);
        let cfg = PllConfig::calculate(input, Hertz::from_raw(100_000_000)).unwrap();
        assert_eq!(
            cfg,
            PllConfig {
                clkr: 0,
                clkf: 9,
                clkod: 3,
                bwadj: 9,
            }
        );
        assert_eq!(cfg.output_freq(input), Hertz::from_raw(100_000_000));
    }
}
