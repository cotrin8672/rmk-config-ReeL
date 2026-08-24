use core::sync::atomic::{AtomicU8, Ordering};

use rmk::core_traits::Runnable;
use rmk::event::{ActionEvent, EventSubscriber, SubscribableEvent, publish_event_async};
use rmk::types::action::Action;
use rmk::types::keycode::{HidKeyCode, KeyCode};

pub const AUTO_MOUSE_LAYER: u8 = 3;

static MANUAL_LAYER_MASK: AtomicU8 = AtomicU8::new(0);

/// Whether a manually held layer currently needs priority over the auto mouse layer.
pub fn manual_layer_is_active() -> bool {
    MANUAL_LAYER_MASK.load(Ordering::Relaxed) != 0
}

fn manual_layer_bit(layer: u8) -> Option<u8> {
    if layer == AUTO_MOUSE_LAYER || layer >= u8::BITS as u8 {
        None
    } else {
        Some(1 << layer)
    }
}

/// Gives a manually held layer priority over the auto mouse layer.
///
/// RMK's built-in `deactivate_on_key` classifier intentionally leaves layer
/// actions unclassified. Emit one synthetic, non-mouse action event for the
/// existing auto-mouse runner so it can release layer 3 through its normal
/// ownership path, while the atomic mask prevents the trigger from rearming
/// layer 3 until the LayerTap is released.
pub struct MouseLayerPriority;

impl MouseLayerPriority {
    pub const fn new() -> Self {
        Self
    }

    async fn handle_event(&mut self, event: ActionEvent) {
        let Some(bit) = manual_layer_bit_from_action(event.action) else {
            return;
        };

        if event.keyboard_event.pressed {
            MANUAL_LAYER_MASK.fetch_or(bit, Ordering::Relaxed);
            publish_event_async(ActionEvent {
                action: Action::Key(KeyCode::Hid(HidKeyCode::No)),
                keyboard_event: event.keyboard_event,
            })
            .await;
        } else {
            MANUAL_LAYER_MASK.fetch_and(!bit, Ordering::Relaxed);
        }
    }
}

impl Runnable for MouseLayerPriority {
    async fn run(&mut self) -> ! {
        let mut subscriber = ActionEvent::subscriber();
        loop {
            self.handle_event(subscriber.next_event().await).await;
        }
    }
}

fn manual_layer_bit_from_action(action: Action) -> Option<u8> {
    match action {
        Action::LayerOn(layer) | Action::LayerOnWithModifier(layer, _) => manual_layer_bit(layer),
        _ => None,
    }
}
