//! Versioned 28-byte calibration record; macro offsets and flash slots stay unchanged.
pub const BLOB_SIZE: usize = 28;
const MAGIC: [u8; 4] = *b"RLC1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatrixCoefficients {
    pub m00: i32,
    pub m01: i32,
    pub m10: i32,
    pub m11: i32,
}

impl MatrixCoefficients {
    pub const DEFAULT: Self = Self {
        m00: -265,
        m01: 1142,
        m10: -831,
        m11: 562,
    };
    fn is_safe(self) -> bool {
        [self.m00, self.m01, self.m10, self.m11]
            .iter()
            .all(|v| v.unsigned_abs() <= 16_000)
            && (i64::from(self.m00) * i64::from(self.m11)
                - i64::from(self.m01) * i64::from(self.m10))
            .unsigned_abs()
                >= 10_000
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Calibration {
    pub matrix: MatrixCoefficients,
    /// Right, left, down, up, in thousandths.
    pub gains: [u16; 4],
    pub sensitivity: u8,
}

impl Calibration {
    pub const DEFAULT: Self = Self {
        matrix: MatrixCoefficients::DEFAULT,
        gains: [1000; 4],
        sensitivity: 100,
    };
    pub fn from_matrix(matrix: MatrixCoefficients) -> Self {
        Self {
            matrix,
            ..Self::DEFAULT
        }
    }
}

pub fn checksum(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
    })
}

pub fn decode_blob(blob: &[u8; BLOB_SIZE]) -> Option<Calibration> {
    if blob[..4] != MAGIC
        || u16::from_le_bytes([blob[6], blob[7]]) != 1000
        || u32::from_le_bytes(blob[24..28].try_into().ok()?) != checksum(&blob[..24])
    {
        return None;
    }
    let mut config = Calibration::DEFAULT;
    let values: [i32; 4] = match blob[4] {
        1 if blob[5] == 0 => core::array::from_fn(|i| {
            i32::from_le_bytes(blob[8 + i * 4..12 + i * 4].try_into().unwrap())
        }),
        2 if (50..=150).contains(&blob[5]) => {
            config.sensitivity = blob[5];
            config.gains = core::array::from_fn(|i| {
                u16::from_le_bytes(blob[16 + i * 2..18 + i * 2].try_into().unwrap())
            });
            if config.gains.iter().any(|g| !(500..=2000).contains(g)) {
                return None;
            }
            core::array::from_fn(|i| {
                i32::from(i16::from_le_bytes(
                    blob[8 + i * 2..10 + i * 2].try_into().unwrap(),
                ))
            })
        }
        _ => return None,
    };
    config.matrix = MatrixCoefficients {
        m00: values[0],
        m01: values[1],
        m10: values[2],
        m11: values[3],
    };
    config.matrix.is_safe().then_some(config)
}

pub fn encode_blob(config: Calibration) -> [u8; BLOB_SIZE] {
    let mut blob = [0; BLOB_SIZE];
    blob[..4].copy_from_slice(&MAGIC);
    blob[4] = 2;
    blob[5] = config.sensitivity;
    blob[6..8].copy_from_slice(&1000u16.to_le_bytes());
    for (i, value) in [
        config.matrix.m00,
        config.matrix.m01,
        config.matrix.m10,
        config.matrix.m11,
    ]
    .iter()
    .enumerate()
    {
        blob[8 + i * 2..10 + i * 2].copy_from_slice(&(*value as i16).to_le_bytes());
    }
    for (i, value) in config.gains.iter().enumerate() {
        blob[16 + i * 2..18 + i * 2].copy_from_slice(&value.to_le_bytes());
    }
    let hash = checksum(&blob[..24]);
    blob[24..28].copy_from_slice(&hash.to_le_bytes());
    blob
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_migrates_without_changing_motion() {
        let blob = [
            0x52, 0x4c, 0x43, 0x31, 1, 0, 0xe8, 3, 0xf7, 0xfe, 0xff, 0xff, 0x76, 4, 0, 0, 0xc1,
            0xfc, 0xff, 0xff, 0x32, 2, 0, 0, 0x8d, 0x69, 0x15, 0x6a,
        ];
        assert_eq!(decode_blob(&blob), Some(Calibration::DEFAULT));
    }
    #[test]
    fn asymmetric_round_trip_and_corruption() {
        let config = Calibration {
            gains: [500, 2000, 1237, 825],
            sensitivity: 135,
            ..Calibration::DEFAULT
        };
        let mut blob = encode_blob(config);
        assert_eq!(
            blob,
            [
                82, 76, 67, 49, 2, 135, 232, 3, 247, 254, 118, 4, 193, 252, 50, 2, 244, 1, 208, 7,
                213, 4, 57, 3, 188, 104, 215, 146
            ]
        );
        assert_eq!(decode_blob(&blob), Some(config));
        blob[16] ^= 1;
        assert_eq!(decode_blob(&blob), None);
    }
    #[test]
    fn rejects_valid_checksum_but_unsafe_values() {
        for config in [
            Calibration {
                gains: [0; 4],
                ..Calibration::DEFAULT
            },
            Calibration {
                sensitivity: 0,
                ..Calibration::DEFAULT
            },
            Calibration {
                matrix: MatrixCoefficients {
                    m00: 0,
                    m01: 0,
                    m10: 0,
                    m11: 0,
                },
                ..Calibration::DEFAULT
            },
        ] {
            assert_eq!(decode_blob(&encode_blob(config)), None);
        }
    }
}
