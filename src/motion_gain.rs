const GAIN_SCALE: i32 = 256;

/// Independent sensitivity control. The default is unity; the intentional
/// 1600-CPI sensor setting is paired with half gain to retain sub-count motion
/// while keeping the overall sensitivity near the previous 800-CPI baseline.
pub const TRACKBALL_GAIN_NUMERATOR: i32 = GAIN_SCALE / 2;

#[derive(Default)]
pub struct MotionGain {
    numerator: i32,
    x_remainder: i64,
    y_remainder: i64,
    calibration: Option<([u16; 4], u8)>,
}

impl MotionGain {
    pub const fn new() -> Self {
        Self::with_gain(TRACKBALL_GAIN_NUMERATOR)
    }

    pub const fn with_gain(numerator: i32) -> Self {
        Self {
            numerator,
            x_remainder: 0,
            y_remainder: 0,
            calibration: None,
        }
    }

    #[cfg(test)]
    pub fn apply(&mut self, x: i16, y: i16) -> (i16, i16) {
        self.apply_calibrated(x, y, [1000; 4], 100)
    }

    pub fn apply_calibrated(
        &mut self,
        x: i16,
        y: i16,
        gains: [u16; 4],
        sensitivity: u8,
    ) -> (i16, i16) {
        if self.calibration != Some((gains, sensitivity)) {
            self.x_remainder = 0;
            self.y_remainder = 0;
            self.calibration = Some((gains, sensitivity));
        }
        let x = i64::from(x);
        let y = i64::from(y);
        let length_squared = x * x + y * y;
        if length_squared == 0 {
            return (0, 0);
        }
        const Q: i64 = 1 << 16;
        const DENOMINATOR: i64 = Q * 1000 * 100 * GAIN_SCALE as i64;
        // Match the web preview's squared-direction interpolation, with one
        // final quantization after directional, user and baseline half gains.
        let gain = (x * x * i64::from(gains[usize::from(x < 0)])
            + y * y * i64::from(gains[2 + usize::from(y < 0)]))
            * Q
            / length_squared;
        let scale = gain * i64::from(sensitivity) * i64::from(self.numerator);
        let x_numerator = x * scale + self.x_remainder;
        let y_numerator = y * scale + self.y_remainder;
        let x = x_numerator / DENOMINATOR;
        let y = y_numerator / DENOMINATOR;

        self.x_remainder = x_numerator - x * DENOMINATOR;
        self.y_remainder = y_numerator - y * DENOMINATOR;

        (
            x.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16,
            y.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::MotionGain;

    #[test]
    fn applies_one_and_a_half_gain_with_remainders() {
        let mut gain = MotionGain::with_gain(384);
        let mut total = 0_i32;
        for _ in 0..256 {
            total += i32::from(gain.apply(1, 0).0);
        }
        assert!((383..=385).contains(&total));
    }

    #[test]
    fn default_gain_is_half() {
        let mut gain = MotionGain::new();
        assert_eq!(gain.apply(12, -7), (6, -3));
    }

    #[test]
    fn default_gain_retains_half_count_motion() {
        let mut gain = MotionGain::new();
        assert_eq!(gain.apply(1, 0), (0, 0));
        assert_eq!(gain.apply(1, 0), (1, 0));
    }

    #[test]
    fn calibrated_cardinals_and_diagonal() {
        for ((x, y), expected) in [
            ((100, 0), (50, 0)),
            ((-100, 0), (-25, 0)),
            ((0, 80), (0, 50)),
            ((0, -100), (0, -100)),
            ((100, 100), (56, 56)),
        ] {
            let mut gain = MotionGain::new();
            assert_eq!(
                gain.apply_calibrated(x, y, [1000, 500, 1250, 2000], 100),
                expected
            );
        }
    }

    #[test]
    fn micro_motion_and_config_change() {
        let mut gain = MotionGain::new();
        let mut sum = 0;
        for _ in 0..1000 {
            sum += i32::from(gain.apply_calibrated(0, 1, [1000, 1000, 1250, 1000], 100).1);
        }
        assert_eq!(sum, 625);
        gain.apply_calibrated(1, 0, [1000; 4], 100);
        assert_eq!(gain.apply_calibrated(1, 0, [500; 4], 100), (0, 0));
    }

    #[test]
    fn extreme_input_is_bounded() {
        let mut gain = MotionGain::new();
        assert_eq!(
            gain.apply_calibrated(i16::MIN, i16::MAX, [2000; 4], 150),
            (i16::MIN, i16::MAX)
        );
        assert_eq!(gain.apply_calibrated(0, 0, [2000; 4], 150), (0, 0));
    }

    #[test]
    fn matches_browser_formula_in_all_quadrants() {
        let gains = [500u16, 2000, 1237, 825];
        for x in [-1000i16, -37, -1, 0, 1, 37, 1000] {
            for y in [-1000i16, -37, -1, 0, 1, 37, 1000] {
                if x == 0 && y == 0 {
                    continue;
                }
                let (u, v) = MotionGain::new().apply_calibrated(x, y, gains, 135);
                let a = f64::from(x);
                let b = f64::from(y);
                let scalar = (a * a * f64::from(gains[usize::from(x < 0)])
                    + b * b * f64::from(gains[2 + usize::from(y < 0)]))
                    / (a * a + b * b)
                    / 1000.0
                    * 1.35
                    * 0.5;
                assert!((f64::from(u) - a * scalar).abs() < 1.001);
                assert!((f64::from(v) - b * scalar).abs() < 1.001);
            }
        }
    }
}
