//! Libinput's calloop source, session-owned file descriptors and seat bookkeeping.

use std::{
    collections::{HashMap, HashSet},
    os::fd::OwnedFd,
    path::Path,
};

use inox_config::InputConfig;
use smithay::{
    backend::{
        input::{
            self, AbsolutePositionEvent, Device, Event, KeyboardKeyEvent, PointerAxisEvent,
            PointerButtonEvent, PointerMotionEvent,
        },
        libinput::LibinputInputBackend,
        session::{AsErrno, Session, libseat::LibSeatSession},
    },
    input::pointer::AxisFrame,
    reexports::{
        input::{Libinput, LibinputInterface},
        rustix::fs::OFlags,
    },
};
use tracing::{debug, warn};

use crate::{Action, InoxState, StateError};

struct SessionInterface(LibSeatSession);

impl LibinputInterface for SessionInterface {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32> {
        self.0
            .open(path, OFlags::from_bits_retain(flags as u32))
            .map_err(|error| error.as_errno().unwrap_or(5))
    }

    fn close_restricted(&mut self, fd: OwnedFd) {
        if let Err(error) = self.0.close(fd) {
            warn!(%error, "failed to release libinput device through libseat");
        }
    }
}

#[derive(Default)]
struct DeviceKeys {
    handle: Option<smithay::reexports::input::Device>,
    keyboard: bool,
    pointer: bool,
    keys: HashSet<u32>,
    buttons: HashSet<u32>,
}

#[derive(Debug, PartialEq, Eq)]
enum Release {
    Key(u32),
    Button(u32),
}

/// Physical keys are counted across devices so removing one keyboard cannot
/// release a key still held on another one.
#[derive(Default)]
struct Devices(HashMap<String, DeviceKeys>);

impl Devices {
    fn capabilities(&self) -> (bool, bool) {
        (
            self.0.values().any(|d| d.keyboard),
            self.0.values().any(|d| d.pointer),
        )
    }

    fn key(&mut self, id: &str, key: u32, pressed: bool, button: bool) -> bool {
        let held_before = self.0.values().any(|device| {
            if button {
                device.buttons.contains(&key)
            } else {
                device.keys.contains(&key)
            }
        });
        let Some(device) = self.0.get_mut(id) else {
            return false;
        };
        let keys = if button {
            &mut device.buttons
        } else {
            &mut device.keys
        };
        if pressed {
            keys.insert(key);
        } else {
            keys.remove(&key);
        }
        let held_after = self.0.values().any(|device| {
            if button {
                device.buttons.contains(&key)
            } else {
                device.keys.contains(&key)
            }
        });
        held_before != held_after
    }

    fn remove(&mut self, id: &str) -> Vec<Release> {
        let Some(device) = self.0.remove(id) else {
            return Vec::new();
        };
        let mut releases = Vec::new();
        for key in device.keys {
            if !self.0.values().any(|device| device.keys.contains(&key)) {
                releases.push(Release::Key(key));
            }
        }
        for button in device.buttons {
            if !self
                .0
                .values()
                .any(|device| device.buttons.contains(&button))
            {
                releases.push(Release::Button(button));
            }
        }
        releases
    }

    fn clear(&mut self) -> Vec<Release> {
        let ids: Vec<_> = self.0.keys().cloned().collect();
        ids.into_iter().flat_map(|id| self.remove(&id)).collect()
    }
}

pub(crate) struct InputDevices {
    context: Libinput,
    devices: Devices,
    config: InputConfig,
    active: bool,
}

impl InputDevices {
    pub(crate) fn reconfigure(&mut self, config: InputConfig) -> Result<(), String> {
        if self.config.tap_to_click == config.tap_to_click
            && self.config.natural_scroll == config.natural_scroll
        {
            self.config = config;
            return Ok(());
        }
        let mut devices: Vec<_> = self
            .devices
            .0
            .values()
            .filter_map(|d| d.handle.clone())
            .collect();
        for device in &mut devices {
            if let Err(error) = configure_device(device, &config) {
                let mut failures = vec![error];
                for device in &mut devices {
                    if let Err(error) = configure_device(device, &self.config) {
                        failures.push(format!("rollback: {error}"));
                    }
                }
                return Err(failures.join("; "));
            }
        }
        self.config = config;
        Ok(())
    }

    pub(crate) fn new(
        session: LibSeatSession,
        config: InputConfig,
    ) -> Result<(Self, LibinputInputBackend), String> {
        let seat = session.seat();
        let mut context = Libinput::new_with_udev(SessionInterface(session));
        context
            .udev_assign_seat(&seat)
            .map_err(|()| format!("cannot assign libinput to seat {seat}"))?;
        let source = LibinputInputBackend::new(context.clone());
        Ok((
            Self {
                context,
                devices: Devices::default(),
                config,
                active: true,
            },
            source,
        ))
    }

    pub(crate) fn pause(&mut self, state: &mut InoxState) -> Result<(), StateError> {
        self.active = false;
        for release in self.devices.clear() {
            apply_release(state, release);
        }
        state.reset_input();
        self.context.suspend();
        state.set_input_capabilities(false, false)
    }

    pub(crate) fn resume(&mut self) -> Result<(), String> {
        if self.active {
            return Ok(());
        }
        self.context
            .resume()
            .map_err(|()| "cannot resume libinput".to_owned())?;
        self.active = true;
        Ok(())
    }

    pub(crate) fn event(
        &mut self,
        event: input::InputEvent<LibinputInputBackend>,
        state: &mut InoxState,
        session: &mut LibSeatSession,
    ) -> Result<(), StateError> {
        if !self.active {
            return Ok(());
        }
        match event {
            input::InputEvent::DeviceAdded { mut device } => {
                let id = device.id();
                for release in self.devices.remove(&id) {
                    apply_release(state, release);
                }
                if device.config_tap_finger_count() > 0
                    && let Err(error) = device.config_tap_set_enabled(self.config.tap_to_click)
                {
                    debug!(?error, %id, "tap-to-click setting unavailable");
                }
                if device.config_scroll_has_natural_scroll()
                    && let Err(error) =
                        device.config_scroll_set_natural_scroll_enabled(self.config.natural_scroll)
                {
                    debug!(?error, %id, "natural-scroll setting unavailable");
                }
                self.devices.0.insert(
                    id,
                    DeviceKeys {
                        handle: Some(device.clone()),
                        keyboard: Device::has_capability(
                            &device,
                            input::DeviceCapability::Keyboard,
                        ),
                        pointer: Device::has_capability(&device, input::DeviceCapability::Pointer),
                        ..Default::default()
                    },
                );
                let (keyboard, pointer) = self.devices.capabilities();
                state.set_input_capabilities(keyboard, pointer)?;
            }
            input::InputEvent::DeviceRemoved { device } => {
                for release in self.devices.remove(&device.id()) {
                    apply_release(state, release);
                }
                let (keyboard, pointer) = self.devices.capabilities();
                state.set_input_capabilities(keyboard, pointer)?;
            }
            input::InputEvent::Keyboard { event } => {
                let key = event.key_code(); // Smithay has already converted evdev to XKB.
                if self.devices.key(
                    &event.device().id(),
                    key.raw(),
                    event.state() == input::KeyState::Pressed,
                    false,
                ) && let Some(Action::SwitchVt(vt)) = state.inject_key(key, event.state())
                    && let Err(error) = session.change_vt(i32::from(vt))
                {
                    warn!(%error, vt, "failed to switch VT");
                }
            }
            input::InputEvent::PointerMotion { event } => {
                let location = state.pointer_location() + event.delta();
                state.inject_pointer_motion(state.clamp_pointer(location));
            }
            input::InputEvent::PointerMotionAbsolute { event } => {
                let size = state.output_size();
                if size.0 > 0 && size.1 > 0 {
                    state.inject_pointer_motion(event.position_transformed(size.into()));
                }
            }
            input::InputEvent::PointerButton { event } => {
                if self.devices.key(
                    &event.device().id(),
                    event.button_code(),
                    event.state() == input::ButtonState::Pressed,
                    true,
                ) {
                    state.inject_pointer_button(event.button_code(), event.state());
                }
            }
            input::InputEvent::PointerAxis { event } => {
                let mut frame = AxisFrame::new(event.time_msec()).source(event.source());
                for axis in [input::Axis::Horizontal, input::Axis::Vertical] {
                    if let Some(value) = event.amount(axis) {
                        frame = frame.value(axis, value);
                    }
                    if let Some(value) = event.amount_v120(axis) {
                        frame = frame.v120(axis, value as i32);
                    }
                    if event.source() == input::AxisSource::Finger
                        && event.amount(axis) == Some(0.0)
                    {
                        frame = frame.stop(axis);
                    }
                }
                state.inject_pointer_axis(frame);
            }
            _ => {}
        }
        Ok(())
    }
}

fn apply_release(state: &mut InoxState, release: Release) {
    match release {
        Release::Key(key) => {
            state.inject_key(key.into(), input::KeyState::Released);
        }
        Release::Button(button) => {
            state.inject_pointer_button(button, input::ButtonState::Released)
        }
    }
}

fn configure_device(
    device: &mut smithay::reexports::input::Device,
    config: &InputConfig,
) -> Result<(), String> {
    if device.config_tap_finger_count() > 0 {
        device
            .config_tap_set_enabled(config.tap_to_click)
            .map_err(|e| format!("input.tap_to_click on {}: {e:?}", device.id()))?;
    }
    if device.config_scroll_has_natural_scroll() {
        device
            .config_scroll_set_natural_scroll_enabled(config.natural_scroll)
            .map_err(|e| format!("input.natural_scroll on {}: {e:?}", device.id()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_devices_and_hot_removal_leave_no_stuck_keys() {
        let mut devices = Devices::default();
        for id in ["a", "b"] {
            devices.0.insert(
                id.into(),
                DeviceKeys {
                    keyboard: true,
                    pointer: true,
                    ..Default::default()
                },
            );
        }
        assert!(devices.key("a", 38, true, false));
        assert!(!devices.key("b", 38, true, false));
        assert!(!devices.key("a", 38, false, false));
        assert!(devices.key("a", 272, true, true));
        assert_eq!(devices.remove("a"), vec![Release::Button(272)]);
        assert_eq!(devices.capabilities(), (true, true));
        assert_eq!(devices.remove("b"), vec![Release::Key(38)]);
        assert_eq!(devices.capabilities(), (false, false));
        assert!(!devices.key("missing", 38, true, false));
    }

    #[test]
    fn pause_clears_each_physical_press_exactly_once() {
        let mut devices = Devices::default();
        for id in ["a", "b"] {
            devices.0.insert(id.into(), DeviceKeys::default());
            devices.key(id, 50, true, false);
        }
        assert_eq!(devices.clear(), vec![Release::Key(50)]);
        assert!(devices.clear().is_empty());
    }
}
