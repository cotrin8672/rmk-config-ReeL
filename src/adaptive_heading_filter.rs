const DIRECTION_SCALE: i64 = 1 << 14;
const DIRECTION_SCALE_SQUARED: i64 = DIRECTION_SCALE * DIRECTION_SCALE;
const COS_5_DEG: i64 = 16_322;
const COS_15_DEG: i64 = 15_826;
const COS_30_DEG: i64 = 14_189;

pub struct AdaptiveHeadingFilter {
    heading_x: i32,
    heading_y: i32,
    initialized: bool,
    output_x_remainder: i64,
    output_y_remainder: i64,
}

impl Default for AdaptiveHeadingFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl AdaptiveHeadingFilter {
    pub const fn new() -> Self {
        Self {
            heading_x: 0,
            heading_y: 0,
            initialized: false,
            output_x_remainder: 0,
            output_y_remainder: 0,
        }
    }

    pub fn apply(&mut self, x: i16, y: i16) -> (i16, i16) {
        if x == 0 && y == 0 {
            return (0, 0);
        }

        let x_i64 = i64::from(x);
        let y_i64 = i64::from(y);
        let magnitude_q14 =
            integer_sqrt(((x_i64 * x_i64 + y_i64 * y_i64) * DIRECTION_SCALE_SQUARED) as u64) as i64;
        let input_heading_x = (x_i64 * DIRECTION_SCALE_SQUARED / magnitude_q14) as i32;
        let input_heading_y = (y_i64 * DIRECTION_SCALE_SQUARED / magnitude_q14) as i32;

        if !self.initialized {
            self.set_heading(input_heading_x, input_heading_y);
            return (x, y);
        }

        let dot = i64::from(self.heading_x) * i64::from(input_heading_x)
            + i64::from(self.heading_y) * i64::from(input_heading_y);
        let alpha_quarters = if dot >= COS_5_DEG * DIRECTION_SCALE {
            1
        } else if dot >= COS_15_DEG * DIRECTION_SCALE {
            2
        } else if dot >= COS_30_DEG * DIRECTION_SCALE {
            3
        } else {
            4
        };

        if alpha_quarters == 4 {
            self.set_heading(input_heading_x, input_heading_y);
            return (x, y);
        }

        let retained_quarters = 4 - alpha_quarters;
        let blended_x = (self.heading_x * retained_quarters + input_heading_x * alpha_quarters) / 4;
        let blended_y = (self.heading_y * retained_quarters + input_heading_y * alpha_quarters) / 4;
        let blended_length = integer_sqrt(
            (i64::from(blended_x) * i64::from(blended_x)
                + i64::from(blended_y) * i64::from(blended_y)) as u64,
        ) as i64;

        self.heading_x = (i64::from(blended_x) * DIRECTION_SCALE / blended_length) as i32;
        self.heading_y = (i64::from(blended_y) * DIRECTION_SCALE / blended_length) as i32;

        let output_x_numerator =
            magnitude_q14 * i64::from(self.heading_x) + self.output_x_remainder;
        let output_y_numerator =
            magnitude_q14 * i64::from(self.heading_y) + self.output_y_remainder;
        let output_x = output_x_numerator / DIRECTION_SCALE_SQUARED;
        let output_y = output_y_numerator / DIRECTION_SCALE_SQUARED;

        self.output_x_remainder = output_x_numerator - output_x * DIRECTION_SCALE_SQUARED;
        self.output_y_remainder = output_y_numerator - output_y * DIRECTION_SCALE_SQUARED;

        (clamp_i16(output_x), clamp_i16(output_y))
    }

    pub fn reset_heading(&mut self) {
        self.heading_x = 0;
        self.heading_y = 0;
        self.initialized = false;
        self.output_x_remainder = 0;
        self.output_y_remainder = 0;
    }

    fn set_heading(&mut self, x: i32, y: i32) {
        self.heading_x = x;
        self.heading_y = y;
        self.initialized = true;
        self.output_x_remainder = 0;
        self.output_y_remainder = 0;
    }
}

fn clamp_i16(value: i64) -> i16 {
    value.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16
}

fn integer_sqrt(value: u64) -> u64 {
    if value == 0 {
        return 0;
    }

    let bit_length = 64 - value.leading_zeros();
    let mut estimate = 1_u64 << ((bit_length + 1) / 2);
    loop {
        let next = (estimate + value / estimate) / 2;
        if next >= estimate {
            return estimate;
        }
        estimate = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn length_squared(x: i16, y: i16) -> i64 {
        i64::from(x) * i64::from(x) + i64::from(y) * i64::from(y)
    }

    #[test]
    fn first_vector_is_unchanged() {
        let mut filter = AdaptiveHeadingFilter::new();
        assert_eq!(filter.apply(921, 391), (921, 391));
    }

    #[test]
    fn straight_motion_is_unchanged() {
        let mut filter = AdaptiveHeadingFilter::new();
        for _ in 0..100 {
            assert_eq!(filter.apply(1000, 0), (1000, 0));
        }
    }

    #[test]
    fn small_heading_jitter_is_reduced() {
        let mut filter = AdaptiveHeadingFilter::new();
        let mut raw_error = 0_i64;
        let mut filtered_error = 0_i64;

        for y in [40, -35, 30, -40, 35, -30, 25, -35] {
            raw_error += i64::from(y) * i64::from(y);
            let (_, filtered_y) = filter.apply(1000, y);
            filtered_error += i64::from(filtered_y) * i64::from(filtered_y);
        }

        assert!(filtered_error < raw_error / 2);
    }

    #[test]
    fn arbitrary_heading_does_not_snap_to_an_axis() {
        let mut filter = AdaptiveHeadingFilter::new();
        let mut total_x = 0_i64;
        let mut total_y = 0_i64;

        for _ in 0..100 {
            let (x, y) = filter.apply(921, 391);
            total_x += i64::from(x);
            total_y += i64::from(y);
        }

        assert!((total_y * 921 - total_x * 391).abs() < total_x.abs());
    }

    #[test]
    fn right_angle_turn_is_immediate() {
        let mut filter = AdaptiveHeadingFilter::new();
        for _ in 0..3 {
            assert_eq!(filter.apply(1000, 0), (1000, 0));
        }
        assert_eq!(filter.apply(0, 1000), (0, 1000));
    }

    #[test]
    fn reversal_is_immediate() {
        let mut filter = AdaptiveHeadingFilter::new();
        assert_eq!(filter.apply(1000, 0), (1000, 0));
        assert_eq!(filter.apply(1000, 0), (1000, 0));
        assert_eq!(filter.apply(-1000, 0), (-1000, 0));
    }

    #[test]
    fn filtered_vector_preserves_magnitude() {
        let mut filter = AdaptiveHeadingFilter::new();
        filter.apply(1000, 0);
        let input = (1000, 176);
        let output = filter.apply(input.0, input.1);
        assert!(
            (length_squared(input.0, input.1) - length_squared(output.0, output.1)).abs() < 2500
        );
    }

    #[test]
    fn handles_i16_extremes() {
        let mut filter = AdaptiveHeadingFilter::new();
        assert_eq!(filter.apply(i16::MAX, i16::MAX), (i16::MAX, i16::MAX));
        let _ = filter.apply(i16::MAX, 30_000);
    }

    #[test]
    fn reset_discards_previous_heading() {
        let mut filter = AdaptiveHeadingFilter::new();
        filter.apply(1000, 0);
        filter.reset_heading();
        assert_eq!(filter.apply(921, 391), (921, 391));
    }
}
