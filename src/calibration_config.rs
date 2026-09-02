#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatrixCoefficients {
    pub m00: i32,
    pub m01: i32,
    pub m10: i32,
    pub m11: i32,
}

const FIXED_MATRIX: MatrixCoefficients = MatrixCoefficients {
    m00: 709,
    m01: 71,
    m10: 121,
    m11: -1117,
};

pub const fn current_matrix() -> MatrixCoefficients {
    FIXED_MATRIX
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_fixed_calibration_matrix() {
        assert_eq!(
            current_matrix(),
            MatrixCoefficients {
                m00: 709,
                m01: 71,
                m10: 121,
                m11: -1117,
            }
        );
    }
}
