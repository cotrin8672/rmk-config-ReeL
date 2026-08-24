use crate::calibration_config::MatrixCoefficients;

pub const MATRIX_SCALE: i64 = 1000;

const DIRECTION_SCALE: i64 = 1 << 14;
const LUT_BUCKETS: usize = 64;
const LUT_OCTANTS: usize = 8;
const LUT_SIZE: usize = LUT_BUCKETS * LUT_OCTANTS;
const RATIO_SCALE: i64 = 1 << 12;
const MATRIX_SCALE_U64: u64 = MATRIX_SCALE as u64;

/// `Automatic` uses a precomputed rotation only for an orthogonal calibration
/// matrix. Non-orthogonal matrices use the full normalized direction LUT.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransformMode {
    DirectionLut,
    Rotation,
    Automatic,
}

pub const DEFAULT_TRANSFORM_MODE: TransformMode = TransformMode::Automatic;

#[derive(Clone, Copy)]
struct DirectionEntry {
    x: i16,
    y: i16,
}

impl DirectionEntry {
    const ZERO: Self = Self { x: 0, y: 0 };
}

#[derive(Clone, Copy)]
struct RotationCoefficients {
    cos: i32,
    sin: i32,
}

impl RotationCoefficients {
    const IDENTITY: Self = Self {
        cos: DIRECTION_SCALE as i32,
        sin: 0,
    };
}

pub struct TrackballTransform {
    output_x_remainder: i64,
    output_y_remainder: i64,
    cached_matrix: Option<MatrixCoefficients>,
    direction_lut: [DirectionEntry; LUT_SIZE],
    rotation: RotationCoefficients,
    mode: TransformMode,
    active_mode: TransformMode,
}

impl Default for TrackballTransform {
    fn default() -> Self {
        Self::new()
    }
}

impl TrackballTransform {
    pub const fn new() -> Self {
        Self::with_mode(DEFAULT_TRANSFORM_MODE)
    }

    pub const fn with_mode(mode: TransformMode) -> Self {
        Self {
            output_x_remainder: 0,
            output_y_remainder: 0,
            cached_matrix: None,
            direction_lut: [DirectionEntry::ZERO; LUT_SIZE],
            rotation: RotationCoefficients::IDENTITY,
            mode,
            active_mode: mode,
        }
    }

    pub fn apply(&mut self, raw_x: i16, raw_y: i16, matrix: MatrixCoefficients) -> (i16, i16) {
        if raw_x == 0 && raw_y == 0 {
            return (0, 0);
        }

        self.ensure_cache(matrix);

        match self.active_mode {
            TransformMode::DirectionLut => self.apply_direction_lut(raw_x, raw_y),
            TransformMode::Rotation => self.apply_rotation(raw_x, raw_y),
            TransformMode::Automatic => unreachable!(),
        }
    }

    fn ensure_cache(&mut self, matrix: MatrixCoefficients) {
        if self.cached_matrix == Some(matrix) {
            return;
        }

        self.active_mode = match self.mode {
            TransformMode::DirectionLut => TransformMode::DirectionLut,
            TransformMode::Rotation => TransformMode::Rotation,
            TransformMode::Automatic => {
                if is_rotation_compatible(matrix) {
                    TransformMode::Rotation
                } else {
                    TransformMode::DirectionLut
                }
            }
        };

        match self.active_mode {
            TransformMode::DirectionLut => self.build_direction_lut(matrix),
            TransformMode::Rotation => self.rotation = build_rotation(matrix),
            TransformMode::Automatic => unreachable!(),
        }
        self.cached_matrix = Some(matrix);
        self.output_x_remainder = 0;
        self.output_y_remainder = 0;
    }

    fn build_direction_lut(&mut self, matrix: MatrixCoefficients) {
        for octant in 0..LUT_OCTANTS {
            for bucket in 0..LUT_BUCKETS {
                let min_component = representative_ratio(bucket);
                let (mut raw_x, mut raw_y) = if octant & 0b100 != 0 {
                    (min_component, RATIO_SCALE)
                } else {
                    (RATIO_SCALE, min_component)
                };

                if octant & 0b010 != 0 {
                    raw_x = -raw_x;
                }
                if octant & 0b001 != 0 {
                    raw_y = -raw_y;
                }

                self.direction_lut[octant * LUT_BUCKETS + bucket] =
                    normalized_per_max(raw_x, raw_y, matrix);
            }
        }
    }

    fn apply_direction_lut(&mut self, raw_x: i16, raw_y: i16) -> (i16, i16) {
        let raw_x = i64::from(raw_x);
        let raw_y = i64::from(raw_y);
        let abs_x = raw_x.unsigned_abs() as i64;
        let abs_y = raw_y.unsigned_abs() as i64;
        let max_component = abs_x.max(abs_y);
        let min_component = abs_x.min(abs_y);
        let mut bucket = (min_component * LUT_BUCKETS as i64 / max_component) as usize;
        if bucket >= LUT_BUCKETS {
            bucket = LUT_BUCKETS - 1;
        }

        let octant = (usize::from(raw_y < 0))
            | (usize::from(raw_x < 0) << 1)
            | (usize::from(abs_x < abs_y) << 2);
        let entry = self.direction_lut[octant * LUT_BUCKETS + bucket];

        let output_x_numerator = i64::from(entry.x) * max_component + self.output_x_remainder;
        let output_y_numerator = i64::from(entry.y) * max_component + self.output_y_remainder;
        let output_x = output_x_numerator / DIRECTION_SCALE;
        let output_y = output_y_numerator / DIRECTION_SCALE;

        self.output_x_remainder = output_x_numerator - output_x * DIRECTION_SCALE;
        self.output_y_remainder = output_y_numerator - output_y * DIRECTION_SCALE;

        (clamp_i16(output_x), clamp_i16(output_y))
    }

    fn apply_rotation(&mut self, raw_x: i16, raw_y: i16) -> (i16, i16) {
        let cos = i64::from(self.rotation.cos);
        let sin = i64::from(self.rotation.sin);
        let raw_x = i64::from(raw_x);
        let raw_y = i64::from(raw_y);
        let output_x_numerator = cos * raw_x - sin * raw_y + self.output_x_remainder;
        let output_y_numerator = sin * raw_x + cos * raw_y + self.output_y_remainder;
        let output_x = output_x_numerator / DIRECTION_SCALE;
        let output_y = output_y_numerator / DIRECTION_SCALE;

        self.output_x_remainder = output_x_numerator - output_x * DIRECTION_SCALE;
        self.output_y_remainder = output_y_numerator - output_y * DIRECTION_SCALE;

        (clamp_i16(output_x), clamp_i16(output_y))
    }
}

fn representative_ratio(bucket: usize) -> i64 {
    if bucket == 0 {
        return 0;
    }
    if bucket == LUT_BUCKETS - 1 {
        return RATIO_SCALE;
    }

    let low = bucket as i64 * RATIO_SCALE / LUT_BUCKETS as i64;
    let high = (bucket as i64 + 1) * RATIO_SCALE / LUT_BUCKETS as i64;
    (low + high) / 2
}

fn normalized_per_max(raw_x: i64, raw_y: i64, matrix: MatrixCoefficients) -> DirectionEntry {
    let output_x_numerator = raw_x * i64::from(matrix.m00) + raw_y * i64::from(matrix.m01);
    let output_y_numerator = raw_x * i64::from(matrix.m10) + raw_y * i64::from(matrix.m11);
    let raw_length =
        integer_sqrt((raw_x * raw_x + raw_y * raw_y) as u64 * MATRIX_SCALE_U64 * MATRIX_SCALE_U64);
    let transformed_length = integer_sqrt(
        output_x_numerator.unsigned_abs() * output_x_numerator.unsigned_abs()
            + output_y_numerator.unsigned_abs() * output_y_numerator.unsigned_abs(),
    );

    if raw_length == 0 || transformed_length == 0 {
        return DirectionEntry::ZERO;
    }

    let normalized_x =
        i128::from(output_x_numerator) * i128::from(raw_length) / i128::from(transformed_length);
    let normalized_y =
        i128::from(output_y_numerator) * i128::from(raw_length) / i128::from(transformed_length);
    let denominator = i128::from(MATRIX_SCALE) * i128::from(RATIO_SCALE);

    DirectionEntry {
        x: (normalized_x * i128::from(DIRECTION_SCALE) / denominator)
            .clamp(i128::from(i16::MIN), i128::from(i16::MAX)) as i16,
        y: (normalized_y * i128::from(DIRECTION_SCALE) / denominator)
            .clamp(i128::from(i16::MIN), i128::from(i16::MAX)) as i16,
    }
}

fn is_rotation_compatible(matrix: MatrixCoefficients) -> bool {
    const TOLERANCE_DENOMINATOR: i128 = 32;

    let m00 = i128::from(matrix.m00);
    let m01 = i128::from(matrix.m01);
    let m10 = i128::from(matrix.m10);
    let m11 = i128::from(matrix.m11);
    let first_column_squared = m00 * m00 + m10 * m10;
    let second_column_squared = m01 * m01 + m11 * m11;
    let maximum_column_squared = first_column_squared.max(second_column_squared);
    if maximum_column_squared == 0 {
        return false;
    }

    let columns_dot = (m00 * m01 + m10 * m11).abs();
    let columns_length_difference = (first_column_squared - second_column_squared).abs();
    columns_dot * TOLERANCE_DENOMINATOR <= maximum_column_squared
        && columns_length_difference * TOLERANCE_DENOMINATOR <= maximum_column_squared
}

fn build_rotation(matrix: MatrixCoefficients) -> RotationCoefficients {
    let sum = i64::from(matrix.m00) + i64::from(matrix.m11);
    let difference = i64::from(matrix.m10) - i64::from(matrix.m01);
    let length = integer_sqrt((sum * sum + difference * difference) as u64);

    if length == 0 {
        return RotationCoefficients::IDENTITY;
    }

    RotationCoefficients {
        cos: (sum * DIRECTION_SCALE / length as i64) as i32,
        sin: (difference * DIRECTION_SCALE / length as i64) as i32,
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

    fn default_transform() -> TrackballTransform {
        TrackballTransform::new()
    }

    #[test]
    fn integer_sqrt_rounds_down() {
        assert_eq!(integer_sqrt(0), 0);
        assert_eq!(integer_sqrt(1), 1);
        assert_eq!(integer_sqrt(2), 1);
        assert_eq!(integer_sqrt(9), 3);
        assert_eq!(integer_sqrt(15), 3);
    }

    #[test]
    fn applies_calibrated_matrix_with_near_unit_length() {
        let mut transform = default_transform();
        for (raw_x, raw_y) in [(1000, 0), (0, 1000), (707, 707), (-707, 707)] {
            let (x, y) = transform.apply(raw_x, raw_y, MatrixCoefficients::DEFAULT);
            let length_squared = i64::from(x) * i64::from(x) + i64::from(y) * i64::from(y);
            assert!((950_000..=1_050_000).contains(&length_squared));
        }
    }

    #[test]
    fn retains_fractional_motion() {
        let mut transform = TrackballTransform::with_mode(TransformMode::DirectionLut);
        let mut total_x = 0_i32;
        let mut total_y = 0_i32;
        for _ in 0..1000 {
            let (x, y) = transform.apply(1, 0, MatrixCoefficients::DEFAULT);
            total_x += i32::from(x);
            total_y += i32::from(y);
        }
        assert!((total_x + 303).abs() <= 2, "total_x={total_x}");
        assert!((total_y + 953).abs() <= 2, "total_y={total_y}");
    }

    #[test]
    fn rotation_mode_preserves_length() {
        let mut transform = TrackballTransform::with_mode(TransformMode::Rotation);
        let quarter_turn = MatrixCoefficients {
            m00: 0,
            m01: -1000,
            m10: 1000,
            m11: 0,
        };
        let (x, y) = transform.apply(1000, 0, quarter_turn);
        let length_squared = i64::from(x) * i64::from(x) + i64::from(y) * i64::from(y);
        assert!((990_000..=1_010_000).contains(&length_squared));
    }

    #[test]
    fn automatic_mode_preserves_calibrated_direction() {
        let mut raw_x_transform = default_transform();
        let (raw_x_output, raw_x_vertical) =
            raw_x_transform.apply(1000, 0, MatrixCoefficients::DEFAULT);
        assert_eq!(raw_x_transform.active_mode, TransformMode::DirectionLut);
        assert!(raw_x_output < 0, "raw X output={raw_x_output}");
        assert!(raw_x_vertical < 0, "raw X vertical output={raw_x_vertical}");

        let mut raw_y_transform = default_transform();
        let (raw_y_output, raw_y_vertical) =
            raw_y_transform.apply(0, 1000, MatrixCoefficients::DEFAULT);
        assert_eq!(raw_y_transform.active_mode, TransformMode::DirectionLut);
        assert!(raw_y_output > 0, "raw Y output={raw_y_output}");
        assert!(raw_y_vertical > 0, "raw Y vertical output={raw_y_vertical}");
    }

    #[test]
    fn automatic_mode_uses_rotation_for_orthogonal_matrix() {
        let identity = MatrixCoefficients {
            m00: 1000,
            m01: 0,
            m10: 0,
            m11: 1000,
        };
        let mut transform = default_transform();
        assert_eq!(transform.apply(1000, 0, identity), (1000, 0));
        assert_eq!(transform.active_mode, TransformMode::Rotation);
    }

    #[test]
    fn rebuilds_cached_transform_when_matrix_changes() {
        let identity = MatrixCoefficients {
            m00: 1000,
            m01: 0,
            m10: 0,
            m11: 1000,
        };
        let quarter_turn = MatrixCoefficients {
            m00: 0,
            m01: -1000,
            m10: 1000,
            m11: 0,
        };

        for mode in [
            TransformMode::DirectionLut,
            TransformMode::Rotation,
            TransformMode::Automatic,
        ] {
            let mut transform = TrackballTransform::with_mode(mode);
            assert_eq!(transform.apply(1000, 0, identity), (1000, 0));
            assert_eq!(transform.apply(1000, 0, quarter_turn), (0, 1000));
        }
    }

    #[test]
    fn clamps_to_i16() {
        let mut transform = default_transform();
        let (x, y) = transform.apply(i16::MIN, i16::MAX, MatrixCoefficients::DEFAULT);
        assert!(x >= i16::MIN);
        assert!(y >= i16::MIN);
    }
}
