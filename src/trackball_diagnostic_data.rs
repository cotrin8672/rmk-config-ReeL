//! PMW3610 smart burst layout, shared by the trace and its host-side tests.
#[derive(Debug, PartialEq)]
pub struct SensorSample {
    pub dx: i16,
    pub dy: i16,
    pub squal: u8,
    pub shutter: u16,
    pub motion: u8,
}

pub fn decode_burst(bytes: &[u8]) -> Option<SensorSample> {
    if bytes.len() != 7 {
        return None;
    }
    let signed = |value: u16| ((value << 4) as i16) >> 4;
    // No-motion bursts can contain stale deltas; match RMK's motion-valid gate.
    let moving = bytes[0] & 0x80 != 0;
    Some(SensorSample {
        dx: if moving {
            signed((u16::from(bytes[3] & 0xf0) << 4) | u16::from(bytes[1]))
        } else {
            0
        },
        dy: if moving {
            signed((u16::from(bytes[3] & 0x0f) << 8) | u16::from(bytes[2]))
        } else {
            0
        },
        squal: bytes[4],
        shutter: u16::from_be_bytes([bytes[5], bytes[6]]),
        motion: bytes[0],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_signed_motion_and_optical_quality_without_stale_deltas() {
        assert_eq!(
            decode_burst(&[0x80, 0xff, 0x00, 0x78, 42, 0x12, 0x34]),
            Some(SensorSample {
                dx: 2047,
                dy: -2048,
                squal: 42,
                shutter: 0x1234,
                motion: 0x80,
            })
        );
        let negative = decode_burst(&[0x80, 0xff, 0xfe, 0xff, 9, 0, 45]).unwrap();
        assert_eq!((negative.dx, negative.dy), (-1, -2));
        let idle = decode_burst(&[0, 0xff, 0xfe, 0xff, 9, 0, 45]).unwrap();
        assert_eq!((idle.dx, idle.dy, idle.squal, idle.shutter), (0, 0, 9, 45));
        assert!(decode_burst(&[0; 4]).is_none());
        assert!(decode_burst(&[0; 8]).is_none());
    }
}
