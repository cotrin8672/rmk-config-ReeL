pub fn should_write_line(
    framebuffer: &[u8],
    previous: Option<&[u8]>,
    width_bytes: usize,
    y: usize,
) -> bool {
    let start = y * width_bytes;
    let end = start + width_bytes;
    previous.is_none_or(|previous| framebuffer[start..end] != previous[start..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH_BYTES: usize = 20;
    const HEIGHT: usize = 68;
    const FRAMEBUFFER_SIZE: usize = WIDTH_BYTES * HEIGHT;

    #[test]
    fn first_flush_writes_every_physical_line() {
        let framebuffer = [0xff; FRAMEBUFFER_SIZE];

        for y in 0..HEIGHT {
            assert!(should_write_line(&framebuffer, None, WIDTH_BYTES, y));
        }
    }

    #[test]
    fn later_flush_writes_only_changed_physical_lines() {
        let previous = [0xff; FRAMEBUFFER_SIZE];
        let mut framebuffer = previous;
        framebuffer[7 * WIDTH_BYTES + 3] = 0x7f;
        framebuffer[42 * WIDTH_BYTES + 19] = 0xfe;

        for y in 0..HEIGHT {
            assert_eq!(
                should_write_line(&framebuffer, Some(&previous), WIDTH_BYTES, y),
                y == 7 || y == 42
            );
        }
    }

    #[test]
    fn applying_selected_lines_reconstructs_identical_framebuffer() {
        let mut previous = [0xff; FRAMEBUFFER_SIZE];
        previous[12 * WIDTH_BYTES + 4] = 0x0f;

        let mut framebuffer = previous;
        framebuffer[12 * WIDTH_BYTES + 4] = 0xf0;
        framebuffer[31 * WIDTH_BYTES + 8] = 0x55;

        let mut reconstructed = previous;
        for y in 0..HEIGHT {
            if should_write_line(&framebuffer, Some(&previous), WIDTH_BYTES, y) {
                let start = y * WIDTH_BYTES;
                let end = start + WIDTH_BYTES;
                reconstructed[start..end].copy_from_slice(&framebuffer[start..end]);
            }
        }

        assert_eq!(reconstructed, framebuffer);
    }
}
