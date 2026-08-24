use rmk::types::action::Action;
use rmk::types::morse::{Morse, MorseMode, MorseProfile};

const HOLD_TIMEOUT_MS: u16 = 220;
const DANCE_TIMEOUT_MS: u16 = 200;

/// A modifier-tap action with an independent action for tap-then-hold.
pub struct QuickModTap {
    tap: Action,
    hold: Action,
    dance: Action,
}

impl QuickModTap {
    pub const fn new(tap: Action, hold: Action, dance: Action) -> Self {
        Self { tap, hold, dance }
    }

    pub fn into_morse(self) -> Morse {
        Morse::new_from_vial(
            self.tap,
            self.hold,
            self.dance,
            Action::No,
            MorseProfile::new(
                Some(false),
                Some(MorseMode::HoldOnOtherPress),
                Some(HOLD_TIMEOUT_MS),
                Some(DANCE_TIMEOUT_MS),
            ),
        )
    }
}
