//! Controller state translated to the original nine-button mapping.
use std::cell::Cell;

pub const DEFAULT_MAPPING: [u16; 9] = [
    0,
    1,
    0,
    u16::MAX,
    u16::MAX,
    u16::MAX,
    u16::MAX,
    u16::MAX,
    u16::MAX,
];
thread_local! { static MAPPING: Cell<[u16; 9]> = const { Cell::new(DEFAULT_MAPPING) }; }
pub fn controller_mapping() -> [u16; 9] {
    MAPPING.with(Cell::get)
}
pub fn set_controller_mapping(mapping: [u16; 9]) {
    MAPPING.with(|m| m.set(mapping));
}

#[derive(Default)]
pub struct State {
    pub buttons: u32,
    pub directions: u16,
}
pub fn map_state(state: &State) -> u16 {
    controller_mapping()
        .iter()
        .enumerate()
        .fold(state.directions, |bits, (i, &button)| {
            bits | if button < 32 && state.buttons & (1 << button) != 0 {
                1 << i
            } else {
                0
            }
        })
}

#[cfg(not(target_arch = "wasm32"))]
pub struct ControllerDevice(Option<gilrs::Gilrs>);
#[cfg(not(target_arch = "wasm32"))]
impl Default for ControllerDevice {
    fn default() -> Self {
        Self(gilrs::Gilrs::new().ok())
    }
}
#[cfg(not(target_arch = "wasm32"))]
impl ControllerDevice {
    pub fn sample(&mut self) -> State {
        use gilrs::{Axis, Button};
        let Some(gilrs) = &mut self.0 else {
            return State::default();
        };
        while gilrs.next_event().is_some() {}
        let Some((_, pad)) = gilrs.gamepads().next() else {
            return State::default();
        };
        // Web-standard numbering makes a saved mapping portable between builds.
        let buttons = [
            Button::South,
            Button::East,
            Button::West,
            Button::North,
            Button::LeftTrigger,
            Button::RightTrigger,
            Button::LeftTrigger2,
            Button::RightTrigger2,
            Button::Select,
            Button::Start,
            Button::LeftThumb,
            Button::RightThumb,
            Button::DPadUp,
            Button::DPadDown,
            Button::DPadLeft,
            Button::DPadRight,
            Button::Mode,
        ];
        let raw = buttons
            .iter()
            .enumerate()
            .fold(0, |b, (i, &key)| b | ((pad.is_pressed(key) as u32) << i));
        let x = pad.value(Axis::LeftStickX);
        let y = pad.value(Axis::LeftStickY);
        State {
            buttons: raw,
            directions: ((y > 0.5 || pad.is_pressed(Button::DPadUp)) as u16) << 4
                | ((y < -0.5 || pad.is_pressed(Button::DPadDown)) as u16) << 5
                | ((x < -0.5 || pad.is_pressed(Button::DPadLeft)) as u16) << 6
                | ((x > 0.5 || pad.is_pressed(Button::DPadRight)) as u16) << 7,
        }
    }
}

#[cfg(target_arch = "wasm32")]
#[derive(Default)]
pub struct ControllerDevice;
#[cfg(target_arch = "wasm32")]
impl ControllerDevice {
    pub fn sample(&mut self) -> State {
        use wasm_bindgen::JsCast;
        let Some(window) = web_sys::window() else {
            return State::default();
        };
        let Ok(pads) = window.navigator().get_gamepads() else {
            return State::default();
        };
        let Some(pad) = pads
            .iter()
            .find_map(|p| p.dyn_into::<web_sys::Gamepad>().ok())
        else {
            return State::default();
        };
        let raw = pad
            .buttons()
            .iter()
            .take(32)
            .enumerate()
            .fold(0, |bits, (i, v)| {
                bits | ((v
                    .dyn_into::<web_sys::GamepadButton>()
                    .is_ok_and(|b| b.pressed()) as u32)
                    << i)
            });
        let axes = pad.axes();
        let x = axes.get(0).as_f64().unwrap_or(0.0);
        let y = axes.get(1).as_f64().unwrap_or(0.0);
        State {
            buttons: raw,
            directions: ((y < -0.5 || raw & (1 << 12) != 0) as u16) << 4
                | ((y > 0.5 || raw & (1 << 13) != 0) as u16) << 5
                | ((x < -0.5 || raw & (1 << 14) != 0) as u16) << 6
                | ((x > 0.5 || raw & (1 << 15) != 0) as u16) << 7,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controller_mapping_keeps_unbound_buttons_and_axes_separate() {
        set_controller_mapping(DEFAULT_MAPPING);
        assert_eq!(
            map_state(&State {
                buttons: 1,
                directions: 1 << 4
            }),
            0x15
        );
        assert_eq!(
            map_state(&State {
                buttons: 1 << 31,
                directions: 0
            }),
            0
        );
    }
}
