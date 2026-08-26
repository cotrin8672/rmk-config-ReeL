use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};

use embassy_time::Timer;
use embedded_storage_async::nor_flash::NorFlash;
use rmk::core_traits::Runnable;
use rmk::embassy_futures::select::{Either, select};
use rmk::event::{EventSubscriber, SleepStateEvent, SubscribableEvent};
use rmk::host::KeyboardContext;
use rmk_types::constants::MACRO_SPACE_SIZE;

pub const CALIBRATION_BLOB_SIZE: usize = 28;
pub const CALIBRATION_MACRO_OFFSET: usize = MACRO_SPACE_SIZE - CALIBRATION_BLOB_SIZE;
pub const RMK_STORAGE_FLASH_START: u32 = 0xA0000;
pub const RMK_STORAGE_FLASH_SIZE: u32 = 0x6000;
pub const CALIBRATION_FLASH_START: u32 = 0xA6000;
pub const CALIBRATION_FLASH_SIZE: u32 = 0x1000;

const MAGIC: [u8; 4] = *b"RLC1";
const FORMAT_VERSION: u8 = 1;
const MAX_ABS_COEFFICIENT: i32 = 16_000;
const MIN_ABS_DETERMINANT: i64 = 10_000;
const CALIBRATION_SLOT_COUNT: usize = CALIBRATION_FLASH_SIZE as usize / CALIBRATION_BLOB_SIZE;
const CONFIG_REFRESH_INTERVAL_SECS: u64 = 1;

const _: () = assert!(MACRO_SPACE_SIZE >= CALIBRATION_BLOB_SIZE);

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
        let values = [self.m00, self.m01, self.m10, self.m11];
        if values
            .iter()
            .any(|value| value.unsigned_abs() > MAX_ABS_COEFFICIENT as u32)
        {
            return false;
        }

        let determinant =
            i64::from(self.m00) * i64::from(self.m11) - i64::from(self.m01) * i64::from(self.m10);
        determinant.unsigned_abs() >= MIN_ABS_DETERMINANT as u64
    }
}

static MATRIX_M00: AtomicI32 = AtomicI32::new(MatrixCoefficients::DEFAULT.m00);
static MATRIX_M01: AtomicI32 = AtomicI32::new(MatrixCoefficients::DEFAULT.m01);
static MATRIX_M10: AtomicI32 = AtomicI32::new(MatrixCoefficients::DEFAULT.m10);
static MATRIX_M11: AtomicI32 = AtomicI32::new(MatrixCoefficients::DEFAULT.m11);
static MATRIX_GENERATION: AtomicU32 = AtomicU32::new(0);

pub fn current_matrix() -> MatrixCoefficients {
    loop {
        let before = MATRIX_GENERATION.load(Ordering::Acquire);
        if before & 1 != 0 {
            core::hint::spin_loop();
            continue;
        }
        let matrix = MatrixCoefficients {
            m00: MATRIX_M00.load(Ordering::Relaxed),
            m01: MATRIX_M01.load(Ordering::Relaxed),
            m10: MATRIX_M10.load(Ordering::Relaxed),
            m11: MATRIX_M11.load(Ordering::Relaxed),
        };
        if MATRIX_GENERATION.load(Ordering::Acquire) == before {
            return matrix;
        }
    }
}

fn apply_matrix(matrix: MatrixCoefficients) {
    MATRIX_GENERATION.fetch_add(1, Ordering::AcqRel);
    MATRIX_M00.store(matrix.m00, Ordering::Relaxed);
    MATRIX_M01.store(matrix.m01, Ordering::Relaxed);
    MATRIX_M10.store(matrix.m10, Ordering::Relaxed);
    MATRIX_M11.store(matrix.m11, Ordering::Relaxed);
    MATRIX_GENERATION.fetch_add(1, Ordering::Release);
}

fn checksum(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
    })
}

fn decode_blob(blob: &[u8; CALIBRATION_BLOB_SIZE]) -> Option<MatrixCoefficients> {
    if blob[..4] != MAGIC || blob[4] != FORMAT_VERSION || blob[5] != 0 {
        return None;
    }
    if u16::from_le_bytes([blob[6], blob[7]]) != 1000 {
        return None;
    }
    if u32::from_le_bytes(blob[24..28].try_into().ok()?) != checksum(&blob[..24]) {
        return None;
    }

    let matrix = MatrixCoefficients {
        m00: i32::from_le_bytes(blob[8..12].try_into().ok()?),
        m01: i32::from_le_bytes(blob[12..16].try_into().ok()?),
        m10: i32::from_le_bytes(blob[16..20].try_into().ok()?),
        m11: i32::from_le_bytes(blob[20..24].try_into().ok()?),
    };
    matrix.is_safe().then_some(matrix)
}

fn encode_blob(matrix: MatrixCoefficients) -> [u8; CALIBRATION_BLOB_SIZE] {
    let mut blob = [0u8; CALIBRATION_BLOB_SIZE];
    blob[..4].copy_from_slice(&MAGIC);
    blob[4] = FORMAT_VERSION;
    blob[6..8].copy_from_slice(&1000u16.to_le_bytes());
    blob[8..12].copy_from_slice(&matrix.m00.to_le_bytes());
    blob[12..16].copy_from_slice(&matrix.m01.to_le_bytes());
    blob[16..20].copy_from_slice(&matrix.m10.to_le_bytes());
    blob[20..24].copy_from_slice(&matrix.m11.to_le_bytes());
    let checksum = checksum(&blob[..24]);
    blob[24..28].copy_from_slice(&checksum.to_le_bytes());
    blob
}

pub async fn recover_legacy_matrix<F: NorFlash>(flash: &mut F) -> Option<MatrixCoefficients> {
    const READ_SIZE: usize = 256;
    const OVERLAP: usize = CALIBRATION_BLOB_SIZE - 1;

    let mut buffer = [0xff; READ_SIZE + OVERLAP];
    let mut carry = 0;
    let mut offset = 0;
    let mut latest = None;

    while offset < flash.capacity() {
        let read_size = READ_SIZE.min(flash.capacity() - offset);
        if flash
            .read(offset as u32, &mut buffer[carry..carry + read_size])
            .await
            .is_err()
        {
            break;
        }

        let available = carry + read_size;
        if available >= CALIBRATION_BLOB_SIZE {
            for start in 0..=available - CALIBRATION_BLOB_SIZE {
                if buffer[start..start + MAGIC.len()] == MAGIC {
                    let mut blob = [0u8; CALIBRATION_BLOB_SIZE];
                    blob.copy_from_slice(&buffer[start..start + CALIBRATION_BLOB_SIZE]);
                    if let Some(matrix) = decode_blob(&blob) {
                        latest = Some(matrix);
                    }
                }
            }
        }

        carry = OVERLAP.min(available);
        buffer.copy_within(available - carry..available, 0);
        offset += read_size;
    }

    latest
}

pub struct CalibrationConfigWatcher<'a, 'keymap, F: NorFlash> {
    context: &'a KeyboardContext<'keymap>,
    flash: F,
    migration_matrix: Option<MatrixCoefficients>,
    last_applied: MatrixCoefficients,
    last_persisted: Option<MatrixCoefficients>,
    next_slot: usize,
}

impl<'a, 'keymap, F: NorFlash> CalibrationConfigWatcher<'a, 'keymap, F> {
    pub fn with_migration(
        context: &'a KeyboardContext<'keymap>,
        flash: F,
        migration_matrix: Option<MatrixCoefficients>,
    ) -> Self {
        Self {
            context,
            flash,
            migration_matrix,
            last_applied: MatrixCoefficients::DEFAULT,
            last_persisted: None,
            next_slot: 0,
        }
    }

    fn read_blob(&self) -> [u8; CALIBRATION_BLOB_SIZE] {
        let mut blob = [0u8; CALIBRATION_BLOB_SIZE];
        self.context
            .read_macro_buffer(CALIBRATION_MACRO_OFFSET, &mut blob);
        blob
    }

    async fn load_persistent_matrix(&mut self) -> Option<MatrixCoefficients> {
        let mut latest = None;

        for slot in 0..CALIBRATION_SLOT_COUNT {
            let mut blob = [0u8; CALIBRATION_BLOB_SIZE];
            if self
                .flash
                .read((slot * CALIBRATION_BLOB_SIZE) as u32, &mut blob)
                .await
                .is_err()
            {
                self.next_slot = slot;
                return latest;
            }

            if blob.iter().all(|byte| *byte == 0xff) {
                self.next_slot = slot;
                return latest;
            }

            if let Some(matrix) = decode_blob(&blob) {
                latest = Some(matrix);
            }
        }

        self.next_slot = CALIBRATION_SLOT_COUNT;
        latest
    }

    async fn persist_matrix(&mut self, matrix: MatrixCoefficients) -> bool {
        if self.next_slot >= CALIBRATION_SLOT_COUNT {
            if self.flash.erase(0, CALIBRATION_FLASH_SIZE).await.is_err() {
                return false;
            }
            self.next_slot = 0;
        }

        let offset = (self.next_slot * CALIBRATION_BLOB_SIZE) as u32;
        if self
            .flash
            .write(offset, &encode_blob(matrix))
            .await
            .is_err()
        {
            return false;
        }

        self.next_slot += 1;
        true
    }

    pub async fn initialize(&mut self) {
        let persistent_matrix = self.load_persistent_matrix().await;
        let macro_matrix = decode_blob(&self.read_blob());
        let matrix = persistent_matrix
            .or(self.migration_matrix)
            .or(macro_matrix)
            .unwrap_or(MatrixCoefficients::DEFAULT);

        if macro_matrix != Some(matrix) {
            self.context
                .write_macro_buffer(CALIBRATION_MACRO_OFFSET, &encode_blob(matrix))
                .await;
        }

        if persistent_matrix.is_none() && self.persist_matrix(matrix).await {
            self.last_persisted = Some(matrix);
        } else {
            self.last_persisted = persistent_matrix;
        }

        apply_matrix(matrix);
        self.last_applied = matrix;
    }

    async fn refresh(&mut self) {
        if let Some(matrix) = decode_blob(&self.read_blob()) {
            if matrix != self.last_applied {
                apply_matrix(matrix);
                self.last_applied = matrix;
            }

            if Some(matrix) != self.last_persisted && self.persist_matrix(matrix).await {
                self.last_persisted = Some(matrix);
            }
        }
    }
}

impl<F: NorFlash> Runnable for CalibrationConfigWatcher<'_, '_, F> {
    async fn run(&mut self) -> ! {
        let mut sleep_subscriber = SleepStateEvent::subscriber();

        loop {
            match select(
                sleep_subscriber.next_event(),
                Timer::after_secs(CONFIG_REFRESH_INTERVAL_SECS),
            )
            .await
            {
                Either::First(sleep) if sleep.0 => {
                    while sleep_subscriber.next_event().await.0 {}
                    self.refresh().await;
                }
                Either::First(_) => {}
                Either::Second(_) => self.refresh().await,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_round_trip() {
        let matrix = MatrixCoefficients::DEFAULT;
        assert_eq!(decode_blob(&encode_blob(matrix)), Some(matrix));
    }

    #[test]
    fn default_blob_matches_browser_wire_format() {
        assert_eq!(
            encode_blob(MatrixCoefficients::DEFAULT),
            [
                0x52, 0x4c, 0x43, 0x31, 0x01, 0x00, 0xe8, 0x03, 0xf7, 0xfe, 0xff, 0xff, 0x76, 0x04,
                0x00, 0x00, 0xc1, 0xfc, 0xff, 0xff, 0x32, 0x02, 0x00, 0x00, 0x8d, 0x69, 0x15, 0x6a,
            ]
        );
    }

    #[test]
    fn rejects_corruption() {
        let mut blob = encode_blob(MatrixCoefficients::DEFAULT);
        blob[10] ^= 0x80;
        assert_eq!(decode_blob(&blob), None);
    }

    #[test]
    fn rejects_dangerous_matrix() {
        assert!(
            !MatrixCoefficients {
                m00: 0,
                m01: 0,
                m10: 0,
                m11: 0,
            }
            .is_safe()
        );
    }
}
