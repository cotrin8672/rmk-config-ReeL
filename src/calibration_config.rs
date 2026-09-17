use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};

use embassy_time::Timer;
use embedded_storage_async::nor_flash::NorFlash;
use rmk::core_traits::Runnable;
use rmk::embassy_futures::select::{Either, select};
use rmk::event::{EventSubscriber, SleepStateEvent, SubscribableEvent};
use rmk::host::KeyboardContext;
use rmk_types::constants::MACRO_SPACE_SIZE;

pub use crate::calibration_data::MatrixCoefficients;
use crate::calibration_data::{Calibration, decode_blob, encode_blob};
pub const CALIBRATION_BLOB_SIZE: usize = crate::calibration_data::BLOB_SIZE;
pub const CALIBRATION_MACRO_OFFSET: usize = MACRO_SPACE_SIZE - CALIBRATION_BLOB_SIZE;
pub const RMK_STORAGE_FLASH_START: u32 = 0xA0000;
pub const RMK_STORAGE_FLASH_SIZE: u32 = 0x6000;
pub const CALIBRATION_FLASH_START: u32 = 0xA6000;
pub const CALIBRATION_FLASH_SIZE: u32 = 0x1000;

const MAGIC: [u8; 4] = *b"RLC1";
const CALIBRATION_SLOT_COUNT: usize = CALIBRATION_FLASH_SIZE as usize / CALIBRATION_BLOB_SIZE;
const CONFIG_REFRESH_INTERVAL_SECS: u64 = 1;

const _: () = assert!(MACRO_SPACE_SIZE >= CALIBRATION_BLOB_SIZE);

static MATRIX_M00: AtomicI32 = AtomicI32::new(MatrixCoefficients::DEFAULT.m00);
static MATRIX_M01: AtomicI32 = AtomicI32::new(MatrixCoefficients::DEFAULT.m01);
static MATRIX_M10: AtomicI32 = AtomicI32::new(MatrixCoefficients::DEFAULT.m10);
static MATRIX_M11: AtomicI32 = AtomicI32::new(MatrixCoefficients::DEFAULT.m11);
static DIRECTION_GAINS: [AtomicU32; 4] = [const { AtomicU32::new(1000) }; 4];
static SENSITIVITY: AtomicU32 = AtomicU32::new(100);
static MATRIX_GENERATION: AtomicU32 = AtomicU32::new(0);

pub fn current_calibration() -> Calibration {
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
        let gains = core::array::from_fn(|i| DIRECTION_GAINS[i].load(Ordering::Relaxed) as u16);
        let sensitivity = SENSITIVITY.load(Ordering::Relaxed) as u8;
        if MATRIX_GENERATION.load(Ordering::Acquire) == before {
            return Calibration {
                matrix,
                gains,
                sensitivity,
            };
        }
    }
}

fn apply_matrix(config: Calibration) {
    let matrix = config.matrix;
    MATRIX_GENERATION.fetch_add(1, Ordering::AcqRel);
    MATRIX_M00.store(matrix.m00, Ordering::Relaxed);
    MATRIX_M01.store(matrix.m01, Ordering::Relaxed);
    MATRIX_M10.store(matrix.m10, Ordering::Relaxed);
    MATRIX_M11.store(matrix.m11, Ordering::Relaxed);
    for (value, atomic) in config.gains.iter().zip(&DIRECTION_GAINS) {
        atomic.store(u32::from(*value), Ordering::Relaxed);
    }
    SENSITIVITY.store(u32::from(config.sensitivity), Ordering::Relaxed);
    MATRIX_GENERATION.fetch_add(1, Ordering::Release);
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
                        latest = Some(matrix.matrix);
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
    last_applied: Calibration,
    last_persisted: Option<Calibration>,
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
            last_applied: Calibration::DEFAULT,
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

    async fn load_persistent_matrix(&mut self) -> Option<Calibration> {
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

    async fn persist_matrix(&mut self, matrix: Calibration) -> bool {
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
            .or(self.migration_matrix.map(Calibration::from_matrix))
            .or(macro_matrix)
            .unwrap_or(Calibration::DEFAULT);

        if macro_matrix != Some(matrix) || self.read_blob()[4] != 2 {
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
        let blob = self.read_blob();
        if let Some(mut matrix) = decode_blob(&blob) {
            // Older web clients only update the angle matrix; retain amount settings.
            if blob[4] == 1 {
                matrix.gains = self.last_applied.gains;
                matrix.sensitivity = self.last_applied.sensitivity;
                self.context
                    .write_macro_buffer(CALIBRATION_MACRO_OFFSET, &encode_blob(matrix))
                    .await;
            }
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
