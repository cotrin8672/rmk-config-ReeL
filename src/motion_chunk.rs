const MAX_RELATIVE_AXIS: u32 = i8::MAX as u32;

pub fn take_proportional_i8_chunk(remaining_x: &mut i32, remaining_y: &mut i32) -> (i8, i8) {
    let maximum_component = remaining_x.unsigned_abs().max(remaining_y.unsigned_abs());
    if maximum_component == 0 {
        return (0, 0);
    }

    let chunk_count = maximum_component.div_ceil(MAX_RELATIVE_AXIS);
    let chunk_x = rounded_div(*remaining_x, chunk_count);
    let chunk_y = rounded_div(*remaining_y, chunk_count);

    debug_assert!((i32::from(i8::MIN)..=i32::from(i8::MAX)).contains(&chunk_x));
    debug_assert!((i32::from(i8::MIN)..=i32::from(i8::MAX)).contains(&chunk_y));

    *remaining_x -= chunk_x;
    *remaining_y -= chunk_y;

    (chunk_x as i8, chunk_y as i8)
}

fn rounded_div(value: i32, divisor: u32) -> i32 {
    let value = i64::from(value);
    let divisor = i64::from(divisor);
    let half = divisor / 2;

    if value >= 0 {
        ((value + half) / divisor) as i32
    } else {
        ((value - half) / divisor) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(x: i32, y: i32) -> Vec<(i8, i8)> {
        let mut remaining_x = x;
        let mut remaining_y = y;
        let mut chunks = Vec::new();

        while remaining_x != 0 || remaining_y != 0 {
            chunks.push(take_proportional_i8_chunk(
                &mut remaining_x,
                &mut remaining_y,
            ));
        }

        chunks
    }

    #[test]
    fn leaves_small_motion_in_one_report() {
        assert_eq!(split(-15, -47), vec![(-15, -47)]);
    }

    #[test]
    fn preserves_fast_vector_direction() {
        assert_eq!(split(-58, -182), vec![(-29, -91), (-29, -91)]);
    }

    #[test]
    fn preserves_total_and_axis_bounds() {
        for (x, y) in [
            (-151, -476),
            (476, -151),
            (-476, 151),
            (151, 476),
            (i32::from(i16::MIN), i32::from(i16::MAX)),
        ] {
            let chunks = split(x, y);
            let total_x: i32 = chunks.iter().map(|chunk| i32::from(chunk.0)).sum();
            let total_y: i32 = chunks.iter().map(|chunk| i32::from(chunk.1)).sum();

            assert_eq!((total_x, total_y), (x, y));
            assert!(chunks.iter().all(|(chunk_x, chunk_y)| {
                chunk_x.unsigned_abs() <= i8::MAX as u8 && chunk_y.unsigned_abs() <= i8::MAX as u8
            }));
        }
    }
}
