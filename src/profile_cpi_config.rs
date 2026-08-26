use embassy_time::{Duration, Timer};
use embedded_storage_async::nor_flash::NorFlash;
use rmk::core_traits::Runnable;
use rmk::embassy_futures::select::{Either3, select3};
use rmk::event::{
    ConnectionStatusChangeEvent, EventSubscriber, PointingSetCpiEvent, SleepStateEvent,
    SubscribableEvent, publish_event_async,
};
use rmk::host::KeyboardContext;
use rmk_types::constants::MACRO_SPACE_SIZE;

use crate::calibration_config::{CALIBRATION_BLOB_SIZE, CALIBRATION_MACRO_OFFSET};

pub const PROFILE_COUNT: usize = 5;
pub const PROFILE_CPI_BLOB_SIZE: usize = 24;
pub const PROFILE_CPI_MACRO_OFFSET: usize = CALIBRATION_MACRO_OFFSET - PROFILE_CPI_BLOB_SIZE;
pub const PROFILE_CPI_FLASH_START: u32 = 0xA7000;
pub const PROFILE_CPI_FLASH_SIZE: u32 = 0x1000;

pub const CPI_MIN: u16 = 200;
pub const CPI_MAX: u16 = 3200;
pub const CPI_STEP: u16 = 200;
pub const DEFAULT_CPI: u16 = 1600;

const MAGIC: [u8; 4] = *b"RCP1";
const FORMAT_VERSION: u8 = 1;
const PROFILE_CPI_SLOT_COUNT: usize = PROFILE_CPI_FLASH_SIZE as usize / PROFILE_CPI_BLOB_SIZE;
const CONFIG_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const INITIAL_APPLY_DELAY: Duration = Duration::from_millis(50);

const _: () = assert!(MACRO_SPACE_SIZE >= CALIBRATION_BLOB_SIZE + PROFILE_CPI_BLOB_SIZE);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProfileCpiConfig {
    pub cpi: [u16; PROFILE_COUNT],
}

impl ProfileCpiConfig {
    pub const DEFAULT: Self = Self {
        cpi: [DEFAULT_CPI; PROFILE_COUNT],
    };

    fn is_valid(self) -> bool {
        self.cpi
            .iter()
            .all(|cpi| (CPI_MIN..=CPI_MAX).contains(cpi) && cpi % CPI_STEP == 0)
    }

    fn for_profile(self, profile: u8) -> u16 {
        self.cpi[usize::from(profile.min((PROFILE_COUNT - 1) as u8))]
    }
}

fn checksum(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
    })
}

fn decode_blob(blob: &[u8; PROFILE_CPI_BLOB_SIZE]) -> Option<ProfileCpiConfig> {
    if blob[..4] != MAGIC
        || blob[4] != FORMAT_VERSION
        || blob[5] != PROFILE_COUNT as u8
        || u16::from_le_bytes([blob[6], blob[7]]) != CPI_STEP
        || blob[18] != 0
        || blob[19] != 0
    {
        return None;
    }
    if u32::from_le_bytes(blob[20..24].try_into().ok()?) != checksum(&blob[..20]) {
        return None;
    }

    let mut cpi = [0; PROFILE_COUNT];
    for (index, value) in cpi.iter_mut().enumerate() {
        let offset = 8 + index * 2;
        *value = u16::from_le_bytes([blob[offset], blob[offset + 1]]);
    }
    let config = ProfileCpiConfig { cpi };
    config.is_valid().then_some(config)
}

fn encode_blob(config: ProfileCpiConfig) -> [u8; PROFILE_CPI_BLOB_SIZE] {
    let mut blob = [0u8; PROFILE_CPI_BLOB_SIZE];
    blob[..4].copy_from_slice(&MAGIC);
    blob[4] = FORMAT_VERSION;
    blob[5] = PROFILE_COUNT as u8;
    blob[6..8].copy_from_slice(&CPI_STEP.to_le_bytes());
    for (index, cpi) in config.cpi.iter().enumerate() {
        let offset = 8 + index * 2;
        blob[offset..offset + 2].copy_from_slice(&cpi.to_le_bytes());
    }
    let checksum = checksum(&blob[..20]);
    blob[20..24].copy_from_slice(&checksum.to_le_bytes());
    blob
}

pub struct ProfileCpiConfigWatcher<'a, 'keymap, F: NorFlash> {
    context: &'a KeyboardContext<'keymap>,
    flash: F,
    config: ProfileCpiConfig,
    last_persisted: Option<ProfileCpiConfig>,
    next_slot: usize,
    active_profile: u8,
    pointing_device_id: u8,
}

impl<'a, 'keymap, F: NorFlash> ProfileCpiConfigWatcher<'a, 'keymap, F> {
    pub fn new(context: &'a KeyboardContext<'keymap>, flash: F, pointing_device_id: u8) -> Self {
        Self {
            context,
            flash,
            config: ProfileCpiConfig::DEFAULT,
            last_persisted: None,
            next_slot: 0,
            active_profile: 0,
            pointing_device_id,
        }
    }

    fn read_blob(&self) -> [u8; PROFILE_CPI_BLOB_SIZE] {
        let mut blob = [0u8; PROFILE_CPI_BLOB_SIZE];
        self.context
            .read_macro_buffer(PROFILE_CPI_MACRO_OFFSET, &mut blob);
        blob
    }

    async fn load_persistent_config(&mut self) -> Option<ProfileCpiConfig> {
        let mut latest = None;

        for slot in 0..PROFILE_CPI_SLOT_COUNT {
            let mut blob = [0u8; PROFILE_CPI_BLOB_SIZE];
            if self
                .flash
                .read((slot * PROFILE_CPI_BLOB_SIZE) as u32, &mut blob)
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

            if let Some(config) = decode_blob(&blob) {
                latest = Some(config);
            }
        }

        self.next_slot = PROFILE_CPI_SLOT_COUNT;
        latest
    }

    async fn persist_config(&mut self, config: ProfileCpiConfig) -> bool {
        if self.next_slot >= PROFILE_CPI_SLOT_COUNT {
            if self.flash.erase(0, PROFILE_CPI_FLASH_SIZE).await.is_err() {
                return false;
            }
            self.next_slot = 0;
        }

        let offset = (self.next_slot * PROFILE_CPI_BLOB_SIZE) as u32;
        if self
            .flash
            .write(offset, &encode_blob(config))
            .await
            .is_err()
        {
            return false;
        }

        self.next_slot += 1;
        true
    }

    pub async fn initialize(&mut self) {
        let persistent_config = self.load_persistent_config().await;
        let macro_config = decode_blob(&self.read_blob());
        let config = persistent_config
            .or(macro_config)
            .unwrap_or(ProfileCpiConfig::DEFAULT);

        if macro_config != Some(config) {
            self.context
                .write_macro_buffer(PROFILE_CPI_MACRO_OFFSET, &encode_blob(config))
                .await;
        }

        if persistent_config.is_none() && self.persist_config(config).await {
            self.last_persisted = Some(config);
        } else {
            self.last_persisted = persistent_config;
        }

        self.config = config;
    }

    async fn apply_active_profile(&self) {
        publish_event_async(PointingSetCpiEvent {
            device_id: self.pointing_device_id,
            cpi: self.config.for_profile(self.active_profile),
        })
        .await;
    }

    async fn refresh(&mut self) {
        let Some(config) = decode_blob(&self.read_blob()) else {
            return;
        };

        if config != self.config {
            let active_cpi_changed = config.for_profile(self.active_profile)
                != self.config.for_profile(self.active_profile);
            self.config = config;
            if active_cpi_changed {
                self.apply_active_profile().await;
            }
        }

        if Some(config) != self.last_persisted && self.persist_config(config).await {
            self.last_persisted = Some(config);
        }
    }

    async fn handle_connection_change(&mut self, event: ConnectionStatusChangeEvent) {
        let profile = event.0.ble.profile.min((PROFILE_COUNT - 1) as u8);
        if profile != self.active_profile {
            self.active_profile = profile;
            self.apply_active_profile().await;
        }
    }
}

impl<F: NorFlash> Runnable for ProfileCpiConfigWatcher<'_, '_, F> {
    async fn run(&mut self) -> ! {
        let mut connection_subscriber = ConnectionStatusChangeEvent::subscriber();
        let mut sleep_subscriber = SleepStateEvent::subscriber();
        Timer::after(INITIAL_APPLY_DELAY).await;
        self.active_profile = self
            .context
            .connection_status()
            .ble
            .profile
            .min((PROFILE_COUNT - 1) as u8);
        self.apply_active_profile().await;

        loop {
            match select3(
                sleep_subscriber.next_event(),
                connection_subscriber.next_event(),
                Timer::after(CONFIG_REFRESH_INTERVAL),
            )
            .await
            {
                Either3::First(sleep) if sleep.0 => {
                    while sleep_subscriber.next_event().await.0 {}
                    self.active_profile = self
                        .context
                        .connection_status()
                        .ble
                        .profile
                        .min((PROFILE_COUNT - 1) as u8);
                    self.refresh().await;
                    self.apply_active_profile().await;
                }
                Either3::First(_) => {}
                Either3::Second(event) => self.handle_connection_change(event).await,
                Either3::Third(_) => self.refresh().await,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_round_trip() {
        let config = ProfileCpiConfig {
            cpi: [200, 800, 1600, 2400, 3200],
        };
        assert_eq!(decode_blob(&encode_blob(config)), Some(config));
    }

    #[test]
    fn default_blob_matches_browser_wire_format() {
        assert_eq!(
            encode_blob(ProfileCpiConfig::DEFAULT),
            [
                0x52, 0x43, 0x50, 0x31, 0x01, 0x05, 0xc8, 0x00, 0x40, 0x06, 0x40, 0x06, 0x40, 0x06,
                0x40, 0x06, 0x40, 0x06, 0x00, 0x00, 0x2d, 0xfb, 0x76, 0x89,
            ]
        );
    }

    #[test]
    fn rejects_out_of_range_or_unaligned_cpi() {
        for invalid in [0, 201, 3400] {
            let mut config = ProfileCpiConfig::DEFAULT;
            config.cpi[2] = invalid;
            assert_eq!(decode_blob(&encode_blob(config)), None);
        }
    }

    #[test]
    fn rejects_corruption() {
        let mut blob = encode_blob(ProfileCpiConfig::DEFAULT);
        blob[10] ^= 0x80;
        assert_eq!(decode_blob(&blob), None);
    }
}
