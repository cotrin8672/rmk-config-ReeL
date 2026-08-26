use rmk::types::action::{EncoderAction, KeyAction};
use rmk::types::modifier::ModifierCombination;
use rmk::types::morse::{MorseMode, MorseProfile};
use rmk::{a, encoder, k, ltp, mt, mtp, shifted, td, user, wm};

pub const ROW: usize = 4;
pub const COL: usize = 11;
pub const NUM_LAYER: usize = 4;
pub const NUM_ENCODER: usize = 1;

const CTRL_SHIFT: ModifierCombination = ModifierCombination::new()
    .with_left_ctrl(true)
    .with_left_shift(true);
const GUI_SHIFT: ModifierCombination = ModifierCombination::new()
    .with_left_gui(true)
    .with_left_shift(true);

const HOLD_PREFERRED_PROFILE: MorseProfile =
    MorseProfile::new(None, Some(MorseMode::HoldOnOtherPress), None, None);
const BALANCED_LAYER_TAP_PROFILE: MorseProfile =
    MorseProfile::new(None, Some(MorseMode::PermissiveHold), None, None);

#[rustfmt::skip]
pub const fn get_default_keymap() -> [[[KeyAction; COL]; ROW]; NUM_LAYER] {
    [
        [
            [k!(T), k!(R), k!(E), k!(W), k!(Q), a!(No), k!(P), k!(O), k!(I), k!(U), k!(Y)],
            [k!(G), k!(F), k!(D), k!(S), k!(A), mt!(Escape, ModifierCombination::LCTRL), mt!(Semicolon, CTRL_SHIFT), k!(L), k!(K), k!(J), k!(H)],
            [k!(B), k!(V), k!(C), k!(X), k!(Z), k!(LGui), k!(Slash), k!(Dot), k!(Comma), k!(M), k!(N)],
            [k!(MouseBtn4), k!(MouseBtn5), mtp!(Language2, ModifierCombination::LSHIFT, HOLD_PREFERRED_PROFILE), ltp!(1, Space, BALANCED_LAYER_TAP_PROFILE), mt!(Language1, ModifierCombination::LALT), a!(No), k!(Minus), k!(Tab), ltp!(2, Enter, BALANCED_LAYER_TAP_PROFILE), td!(0), a!(No)],
        ],
        [
            [k!(Kc5), k!(Kc4), k!(Kc3), k!(Kc2), k!(Kc1), a!(No), k!(Kc0), k!(Kc9), k!(Kc8), k!(Kc7), k!(Kc6)],
            [shifted!(Comma), shifted!(Kc9), shifted!(LeftBracket), k!(LeftBracket), shifted!(Quote), a!(Transparent), k!(Quote), k!(RightBracket), shifted!(RightBracket), shifted!(Kc0), shifted!(Dot)],
            [shifted!(Kc5), k!(Slash), shifted!(Kc8), k!(Minus), shifted!(Equal), a!(Transparent), shifted!(Kc6), a!(Transparent), a!(Transparent), k!(Equal), shifted!(Semicolon)],
            [a!(Transparent), a!(Transparent), a!(Transparent), a!(Transparent), a!(Transparent), a!(No), a!(Transparent), a!(Transparent), a!(Transparent), a!(Transparent), a!(No)],
        ],
        [
            [k!(F5), k!(F4), k!(F3), k!(F2), k!(F1), a!(No), k!(F10), k!(F9), k!(F8), k!(F7), k!(F6)],
            [a!(Transparent), shifted!(Backslash), shifted!(Kc7), shifted!(Kc1), shifted!(Kc6), a!(Transparent), shifted!(Kc4), k!(Right), k!(Up), k!(Down), k!(Left)],
            [a!(Transparent), shifted!(Kc3), wm!(S, GUI_SHIFT), shifted!(Grave), wm!(S, GUI_SHIFT), user!(5), k!(Backslash), a!(Transparent), shifted!(Kc2), k!(Equal), a!(Transparent)],
            [a!(Transparent), a!(Transparent), a!(Transparent), a!(Transparent), a!(Transparent), a!(No), k!(F11), k!(F12), a!(Transparent), a!(Transparent), a!(No)],
        ],
        [
            [a!(Transparent), a!(Transparent), a!(Transparent), a!(Transparent), a!(Transparent), a!(No), a!(Transparent), a!(Transparent), a!(Transparent), a!(Transparent), a!(Transparent)],
            [a!(Transparent), a!(Transparent), a!(Transparent), a!(Transparent), a!(Transparent), a!(Transparent), a!(Transparent), k!(MouseBtn2), k!(MouseBtn3), k!(MouseBtn1), a!(Transparent)],
            [a!(Transparent), a!(Transparent), a!(Transparent), a!(Transparent), a!(Transparent), a!(No), a!(Transparent), a!(Transparent), a!(Transparent), a!(Transparent), a!(Transparent)],
            [a!(Transparent), a!(Transparent), a!(Transparent), a!(Transparent), a!(Transparent), a!(No), a!(Transparent), a!(Transparent), a!(Transparent), a!(Transparent), a!(No)],
        ],
    ]
}

pub const fn get_default_encoder_map() -> [[EncoderAction; NUM_ENCODER]; NUM_LAYER] {
    [
        [encoder!(k!(MouseWheelUp), k!(MouseWheelDown))],
        [encoder!(k!(MouseWheelUp), k!(MouseWheelDown))],
        [encoder!(a!(Transparent), a!(Transparent))],
        [encoder!(a!(Transparent), a!(Transparent))],
    ]
}
