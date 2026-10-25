//! Text transactions and privileged input methods.

use super::*;
use inox_protocols::standard::wp::{
    input_method::server::{
        zwp_input_method_keyboard_grab_v2::{self as grab, ZwpInputMethodKeyboardGrabV2},
        zwp_input_method_manager_v2::{self as imm, ZwpInputMethodManagerV2},
        zwp_input_method_v2::{self as im, ZwpInputMethodV2},
        zwp_input_popup_surface_v2::{self as popup, ZwpInputPopupSurfaceV2},
    },
    text_input::server::{
        zwp_text_input_manager_v3::{self as tim, ZwpTextInputManagerV3},
        zwp_text_input_v3::{self as ti, ZwpTextInputV3},
    },
    virtual_keyboard::server::zwp_virtual_keyboard_v1::{self as vk, ZwpVirtualKeyboardV1},
};
use smithay::{
    desktop::utils::{bbox_from_surface_tree, send_frames_surface_tree, under_from_surface_tree},
    input::keyboard::{
        GrabStartData, KeyboardGrab, KeyboardInnerHandle, KeyboardTarget, KeymapFile,
        ModifiersState, xkb,
    },
    reexports::wayland_server::{WEnum, protocol::wl_keyboard::KeymapFormat},
    wayland::{
        compositor::{TraversalAction, give_role, with_surface_tree_downward},
        virtual_keyboard::{VirtualKeyboardManagerState, VirtualKeyboardUserData},
    },
};

const POPUP_ROLE: &str = "zwp_input_popup_surface_v2";
const MAX_TEXT_BYTES: usize = 4000;

#[derive(Default)]
struct Context {
    enabled: Option<bool>,
    surrounding: Option<(String, i32, i32)>,
    cause: Option<ti::ChangeCause>,
    content: Option<(ti::ContentHint, ti::ContentPurpose)>,
    cursor: Option<Rectangle<i32, Logical>>,
}

struct TextInput {
    object: ZwpTextInputV3,
    serial: u32,
    pending: Context,
    current: Context,
}

#[derive(Default)]
struct Editing {
    preedit: Option<(String, i32, i32)>,
    commit: Option<String>,
    delete: Option<(u32, u32)>,
}

struct Candidate {
    role: ZwpInputPopupSurfaceV2,
    surface: WlSurface,
    last_rectangle: Option<Rectangle<i32, Logical>>,
    output: Option<Output>,
}

struct ForwardedKeys {
    surface: WlSurface,
    keys: HashSet<u32>,
}

impl Drop for Candidate {
    fn drop(&mut self) {
        if let Some(output) = self.output.take() {
            surface_output(&self.surface, &output, false);
        }
    }
}

fn surface_output(surface: &WlSurface, output: &Output, enter: bool) {
    with_surface_tree_downward(
        surface,
        (),
        |_, _, _| TraversalAction::DoChildren(()),
        |surface, _, _| {
            if enter {
                output.enter(surface)
            } else {
                output.leave(surface)
            }
        },
        |_, _, _| true,
    );
}

#[derive(Default)]
pub(super) struct ImeState {
    focus: Option<WlSurface>,
    texts: Vec<TextInput>,
    active: Option<ZwpTextInputV3>,
    method: Option<ZwpInputMethodV2>,
    serial: u32,
    editing: Editing,
    popups: Vec<Candidate>,
    grab: Option<ZwpInputMethodKeyboardGrabV2>,
    grab_installed: bool,
    virtual_keys: HashMap<ZwpVirtualKeyboardV1, ForwardedKeys>,
}

pub(super) fn trusted(client: &Client) -> bool {
    client
        .get_data::<ClientState>()
        .is_some_and(|data| data.input_method)
}

pub(super) fn init(display: &DisplayHandle) {
    display.create_global::<InoxState, ZwpTextInputManagerV3, ()>(1, ());
    display.create_global::<InoxState, ZwpInputMethodManagerV2, ()>(1, ());
    VirtualKeyboardManagerState::new::<InoxState, _>(display, trusted);
}

impl InoxState {
    pub(super) fn ime_sync_focus(&mut self, focus: Option<WlSurface>) {
        let focus = focus.filter(|surface| surface.is_alive() && !self.lock.accepted());
        if self.ime.focus != focus {
            self.ime.virtual_keys.clear();
            self.ime_deactivate();
            if let Some(old) = self.ime.focus.as_ref() {
                for text in &mut self.ime.texts {
                    if text.object.id().same_client_as(&old.id()) {
                        text.object.leave(old);
                        text.pending = Context::default();
                        text.current = Context::default();
                    }
                }
            }
            self.ime.focus = focus;
            if let Some(focus) = self.ime.focus.as_ref() {
                for text in &self.ime.texts {
                    if text.object.id().same_client_as(&focus.id()) {
                        text.object.enter(focus);
                    }
                }
            }
        }
        self.ime_sync_grab();
    }

    fn ime_deactivate(&mut self) {
        if let Some(active) = self.ime.active.take() {
            if active.is_alive()
                && let Some(text) = self.ime.texts.iter().find(|text| text.object == active)
            {
                active.preedit_string(Some(String::new()), 0, 0);
                active.done(text.serial);
            }
            if let Some(method) = self.ime.method.as_ref() {
                method.deactivate();
                self.ime_done();
            }
            self.repaint = true;
        }
        self.ime.editing = Editing::default();
        self.ime_update_popups();
    }

    fn ime_done(&mut self) {
        if let Some(method) = self.ime.method.as_ref() {
            method.done();
            self.ime.serial = self.ime.serial.wrapping_add(1);
        }
        self.ime.editing = Editing::default();
    }

    fn ime_send_context(&mut self, activate: bool) {
        let Some(method) = self.ime.method.as_ref() else {
            return;
        };
        let Some(text) = self
            .ime
            .texts
            .iter()
            .find(|text| Some(&text.object) == self.ime.active.as_ref())
        else {
            return;
        };
        if activate {
            method.activate();
        }
        let current = &text.current;
        if let Some((text, cursor, anchor)) = &current.surrounding {
            method.surrounding_text(text.clone(), *cursor as u32, *anchor as u32);
        }
        method.text_change_cause(current.cause.unwrap_or(ti::ChangeCause::InputMethod));
        let (hint, purpose) = current
            .content
            .unwrap_or((ti::ContentHint::None, ti::ContentPurpose::Normal));
        method.content_type(hint, purpose);
        self.ime_update_popups();
        self.ime_done();
        self.repaint = true;
    }

    pub(super) fn ime_sync_grab(&mut self) {
        let allowed = !self.lock.accepted()
            && self.seat.get_keyboard().is_some()
            && self.ime.focus.is_some()
            && self.ime.method.as_ref().is_some_and(Resource::is_alive)
            && self.ime.grab.as_ref().is_some_and(Resource::is_alive);
        if !allowed && self.ime.grab_installed {
            self.ime.grab_installed = false;
            let keyboard = self.keyboard.clone();
            keyboard.unset_grab(self);
        } else if allowed
            && !self.ime.grab_installed
            && !self.keyboard.is_grabbed()
            && let Some(object) = self.ime.grab.clone()
        {
            if !self.ime_send_keymap(&object) {
                return;
            }
            self.ime.grab_installed = true;
            let keyboard = self.keyboard.clone();
            keyboard.set_grab(
                self,
                ImeGrab {
                    object,
                    start: GrabStartData { focus: None },
                },
                SERIAL_COUNTER.next_serial(),
            );
        }
    }

    fn ime_send_keymap(&mut self, object: &ZwpInputMethodKeyboardGrabV2) -> bool {
        let config = keyboard_config(&self.input_config);
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let Some(keymap) = xkb::Keymap::new_from_names(
            &context,
            config.rules,
            config.model,
            config.layout,
            config.variant,
            config.options,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        ) else {
            warn!("failed to compile input method keymap");
            return false;
        };
        let keymap = KeymapFile::new(&keymap);
        if let Err(error) = keymap.with_fd(false, |fd, size| {
            object.keymap(KeymapFormat::XkbV1, fd, size as u32)
        }) {
            warn!(%error, "failed to send input method keymap");
            return false;
        }
        object.repeat_info(
            self.input_config.keyboard_repeat_rate_hz as i32,
            self.input_config.keyboard_repeat_delay_ms as i32,
        );
        send_modifiers(
            object,
            self.keyboard.modifier_state(),
            SERIAL_COUNTER.next_serial(),
        );
        true
    }

    fn ime_parent_origin(&self) -> Option<Point<i32, Logical>> {
        let focus = self.ime.focus.as_ref()?;
        if let Some(entry) = self
            .layers
            .iter()
            .find(|entry| entry.mapped && entry.surface.wl_surface() == focus)
        {
            return Some(entry.geometry.loc);
        }
        self.windows.values().find_map(|window| {
            (window_toplevel_surface(window) == Some(focus))
                .then(|| {
                    self.space
                        .element_location(window)
                        .map(|location| location - window.geometry().loc)
                })
                .flatten()
        })
    }

    fn ime_cursor(&self) -> Rectangle<i32, Logical> {
        self.ime
            .texts
            .iter()
            .find(|text| Some(&text.object) == self.ime.active.as_ref())
            .and_then(|text| text.current.cursor)
            .unwrap_or_default()
    }

    fn ime_popup_location(&self, surface: &WlSurface) -> Option<Point<i32, Logical>> {
        if self.lock.accepted() || self.ime.active.is_none() || !surface.is_alive() {
            return None;
        }
        let parent = self.ime_parent_origin()?;
        let focus = self.ime.focus.as_ref()?;
        let output = self
            .output_for_surface(focus)
            .and_then(|id| self.outputs.iter().find(|entry| entry.id == id))?;
        let bounds = bbox_from_surface_tree(surface, (0, 0));
        let cursor = self.ime_cursor();
        let cursor = Rectangle::new(
            (
                parent.x.saturating_add(cursor.loc.x),
                parent.y.saturating_add(cursor.loc.y),
            )
                .into(),
            cursor.size,
        );
        Some(candidate_location(cursor, bounds, output.area))
    }

    pub(super) fn ime_update_popups(&mut self) {
        let parent = self.ime_parent_origin();
        let cursor = self.ime_cursor();
        let rectangles: Vec<_> = self
            .ime
            .popups
            .iter()
            .map(|popup| {
                self.ime_popup_location(&popup.surface)
                    .zip(parent)
                    .map(|(location, parent)| {
                        Rectangle::new(
                            (
                                parent
                                    .x
                                    .saturating_add(cursor.loc.x)
                                    .saturating_sub(location.x),
                                parent
                                    .y
                                    .saturating_add(cursor.loc.y)
                                    .saturating_sub(location.y),
                            )
                                .into(),
                            cursor.size,
                        )
                    })
            })
            .collect();
        let output = self
            .ime
            .focus
            .as_ref()
            .and_then(|surface| self.output_for_surface(surface))
            .and_then(|id| self.output_of(id))
            .cloned();
        for (popup, rectangle) in self.ime.popups.iter_mut().zip(rectangles) {
            let target = output.clone().filter(|_| {
                rectangle.is_some()
                    && popup.role.is_alive()
                    && with_renderer_surface_state(&popup.surface, |state| state.buffer().is_some())
                        .unwrap_or(false)
            });
            if popup.output != target
                && let Some(old) = popup.output.take()
            {
                surface_output(&popup.surface, &old, false);
            }
            if let Some(output) = &target {
                surface_output(&popup.surface, output, true);
            }
            popup.output = target;
            if rectangle != popup.last_rectangle
                && let Some(rectangle) = rectangle
            {
                popup.role.text_input_rectangle(
                    rectangle.loc.x,
                    rectangle.loc.y,
                    rectangle.size.w,
                    rectangle.size.h,
                );
                popup.last_rectangle = Some(rectangle);
            }
        }
    }

    pub(super) fn ime_update_scales(&self) {
        for popup in &self.ime.popups {
            self.update_surface_scale(&popup.surface);
        }
    }

    pub(super) fn ime_render_elements(
        &self,
        renderer: &mut GlesRenderer,
        region: Rectangle<i32, Logical>,
        scale: f64,
        elements: &mut Vec<SceneElement>,
    ) {
        for popup in self
            .ime
            .popups
            .iter()
            .rev()
            .filter(|popup| popup.role.is_alive())
        {
            if let Some(location) = self.ime_popup_location(&popup.surface) {
                elements.extend(render_elements_from_surface_tree(
                    renderer,
                    &popup.surface,
                    (location - region.loc).to_physical_precise_round(scale),
                    scale,
                    1.0,
                    Kind::Unspecified,
                ));
            }
        }
    }

    pub(super) fn ime_surface_under(
        &self,
        point: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.ime
            .popups
            .iter()
            .rev()
            .filter(|popup| popup.role.is_alive())
            .find_map(|popup| {
                let location = self.ime_popup_location(&popup.surface)?;
                under_from_surface_tree(&popup.surface, point, location, WindowSurfaceType::ALL)
                    .map(|(surface, offset)| (surface, offset.to_f64()))
            })
    }

    pub(super) fn ime_send_frames(&self, output: &Output, time: Duration) {
        if self.lock.accepted() {
            return;
        }
        for popup in &self.ime.popups {
            if popup.role.is_alive()
                && self.ime_popup_location(&popup.surface).is_some()
                && self
                    .ime
                    .focus
                    .as_ref()
                    .and_then(|surface| self.output_for_surface(surface))
                    == self.find_output(output).map(|entry| entry.id)
            {
                send_frames_surface_tree(
                    &popup.surface,
                    output,
                    time,
                    Some(Duration::ZERO),
                    |_, _| Some(output.clone()),
                );
            }
        }
    }

    pub(super) fn ime_surface_destroyed(&mut self, surface: &WlSurface) {
        self.ime.popups.retain(|popup| popup.surface != *surface);
        if self.ime.focus.as_ref() == Some(surface) {
            self.ime_sync_focus(None);
        }
    }

    pub(super) fn ime_popup_parent(&self, surface: &WlSurface) -> Option<&WlSurface> {
        self.ime
            .popups
            .iter()
            .any(|popup| popup.surface == *surface)
            .then_some(self.ime.focus.as_ref())
            .flatten()
    }

    fn ime_release_virtual_keys(&mut self, object: &ZwpVirtualKeyboardV1) {
        let Some(forwarded) = self.ime.virtual_keys.remove(object) else {
            return;
        };
        if self.lock.accepted() || self.keyboard_focus().as_ref() != Some(&forwarded.surface) {
            return;
        }
        let keyboard = self.keyboard.clone();
        let seat = self.seat.clone();
        let held = keyboard.pressed_keys();
        for key in forwarded.keys {
            if self
                .ime
                .virtual_keys
                .values()
                .any(|other| other.surface == forwarded.surface && other.keys.contains(&key))
            {
                continue;
            }
            let Some(key) = key.checked_add(8).map(Keycode::new) else {
                continue;
            };
            if held.contains(&key) {
                continue;
            }
            // Virtual keys bypass the physical grab. Their teardown must do the
            // same so a disappearing IME cannot leave an application key held.
            keyboard.input_intercept(self, key, KeyState::Released, |state, _, keysym| {
                forwarded.surface.key(
                    &seat,
                    state,
                    keysym,
                    KeyState::Released,
                    SERIAL_COUNTER.next_serial(),
                    0,
                );
            });
        }
        forwarded.surface.modifiers(
            &seat,
            self,
            keyboard.modifier_state(),
            SERIAL_COUNTER.next_serial(),
        );
    }
}

fn candidate_location(
    cursor: Rectangle<i32, Logical>,
    popup: Rectangle<i32, Logical>,
    output: Rectangle<i32, Logical>,
) -> Point<i32, Logical> {
    let right = output.loc.x.saturating_add(output.size.w);
    let bottom = output.loc.y.saturating_add(output.size.h);
    let below = cursor.loc.y.saturating_add(cursor.size.h.max(0));
    let y = if below.saturating_add(popup.size.h) <= bottom {
        below
    } else {
        cursor.loc.y.saturating_sub(popup.size.h)
    };
    let x = cursor.loc.x.clamp(
        output.loc.x,
        right.saturating_sub(popup.size.w).max(output.loc.x),
    );
    let y = y.clamp(
        output.loc.y,
        bottom.saturating_sub(popup.size.h).max(output.loc.y),
    );
    (x.saturating_sub(popup.loc.x), y.saturating_sub(popup.loc.y)).into()
}

fn send_modifiers(
    object: &ZwpInputMethodKeyboardGrabV2,
    modifiers: ModifiersState,
    serial: Serial,
) {
    let mods = modifiers.serialized;
    object.modifiers(
        serial.into(),
        mods.depressed,
        mods.latched,
        mods.locked,
        mods.layout_effective,
    );
}

struct ImeGrab {
    object: ZwpInputMethodKeyboardGrabV2,
    start: GrabStartData<InoxState>,
}

impl KeyboardGrab<InoxState> for ImeGrab {
    fn input(
        &mut self,
        data: &mut InoxState,
        handle: &mut KeyboardInnerHandle<'_, InoxState>,
        keycode: Keycode,
        state: KeyState,
        modifiers: Option<ModifiersState>,
        serial: Serial,
        time: u32,
    ) {
        if data.lock.accepted() || !self.object.is_alive() {
            handle.input(data, keycode, state, modifiers, serial, time);
            return;
        }
        if let Some(key) = keycode.raw().checked_sub(8) {
            self.object.key(serial.into(), time, key, state.into());
        }
        if let Some(modifiers) = modifiers {
            send_modifiers(&self.object, modifiers, serial);
        }
    }
    fn set_focus(
        &mut self,
        data: &mut InoxState,
        handle: &mut KeyboardInnerHandle<'_, InoxState>,
        focus: Option<WlSurface>,
        serial: Serial,
    ) {
        handle.set_focus(data, focus, serial);
    }
    fn start_data(&self) -> &GrabStartData<InoxState> {
        &self.start
    }
    fn unset(&mut self, data: &mut InoxState) {
        data.ime.grab_installed = false;
    }
}

impl GlobalDispatch<ZwpTextInputManagerV3, ()> for InoxState {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<ZwpTextInputManagerV3>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
    }
}

impl Dispatch<ZwpTextInputManagerV3, ()> for InoxState {
    fn request(
        state: &mut Self,
        _: &Client,
        manager: &ZwpTextInputManagerV3,
        request: tim::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let tim::Request::GetTextInput { id, seat } = request {
            if Seat::<Self>::from_resource(&seat).as_ref() != Some(&state.seat) {
                init.post_error(id, 0u32, "Unknown seat");
                return;
            }
            let object = init.init(id, ());
            if let Some(focus) = state
                .ime
                .focus
                .as_ref()
                .filter(|focus| focus.id().same_client_as(&object.id()))
            {
                object.enter(focus);
            }
            state.ime.texts.push(TextInput {
                object,
                serial: 0,
                pending: Context::default(),
                current: Context::default(),
            });
        } else if !matches!(request, tim::Request::Destroy) {
            manager.post_error(0u32, "Unknown request");
        }
    }
}

impl Dispatch<ZwpTextInputV3, ()> for InoxState {
    fn request(
        state: &mut Self,
        _: &Client,
        object: &ZwpTextInputV3,
        request: ti::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        let Some(index) = state
            .ime
            .texts
            .iter()
            .position(|text| text.object == *object)
        else {
            return;
        };
        if matches!(request, ti::Request::Commit) {
            state.ime.texts[index].serial = state.ime.texts[index].serial.wrapping_add(1);
        }
        if !state
            .ime
            .focus
            .as_ref()
            .is_some_and(|focus| focus.is_alive() && focus.id().same_client_as(&object.id()))
            || state.lock.accepted()
        {
            return;
        }
        let pending = &mut state.ime.texts[index].pending;
        match request {
            ti::Request::Enable => {
                *pending = Context::default();
                pending.enabled = Some(true);
            }
            ti::Request::Disable => {
                *pending = Context::default();
                pending.enabled = Some(false);
            }
            ti::Request::SetSurroundingText {
                text,
                cursor,
                anchor,
            } => {
                if !valid_surrounding(&text, cursor, anchor) {
                    object.post_error(0u32, "Invalid UTF-8 surrounding text offsets or length");
                    return;
                }
                pending.surrounding = Some((text, cursor, anchor));
            }
            ti::Request::SetTextChangeCause {
                cause: WEnum::Value(cause),
            } => pending.cause = Some(cause),
            ti::Request::SetContentType {
                hint: WEnum::Value(hint),
                purpose: WEnum::Value(purpose),
            } => pending.content = Some((hint, purpose)),
            ti::Request::SetCursorRectangle {
                x,
                y,
                width,
                height,
            } if width >= 0 && height >= 0 => {
                pending.cursor = Some(Rectangle::new((x, y).into(), (width, height).into()))
            }
            ti::Request::Commit => {
                let pending = std::mem::take(pending);
                let activate = pending.enabled == Some(true);
                if activate {
                    state.ime_deactivate();
                    state.ime.active = Some(object.clone());
                    state.ime.texts[index].current = Context::default();
                } else if state.ime.active.as_ref() != Some(object) {
                    return;
                }
                if pending.enabled == Some(false) {
                    state.ime_deactivate();
                    return;
                }
                let current = &mut state.ime.texts[index].current;
                if pending.surrounding.is_some() {
                    current.surrounding = pending.surrounding;
                }
                current.cause = pending.cause;
                if pending.content.is_some() {
                    current.content = pending.content;
                }
                if pending.cursor.is_some() {
                    current.cursor = pending.cursor;
                }
                state.ime_send_context(activate);
            }
            ti::Request::Destroy => {}
            _ => object.post_error(0u32, "Invalid text input request"),
        }
    }
    fn destroyed(state: &mut Self, _: ClientId, object: &ZwpTextInputV3, _: &()) {
        if state.ime.active.as_ref() == Some(object) {
            state.ime_deactivate();
        }
        state.ime.texts.retain(|text| text.object != *object);
    }
}

fn valid_surrounding(text: &str, cursor: i32, anchor: i32) -> bool {
    text.len() <= MAX_TEXT_BYTES
        && [cursor, anchor]
            .into_iter()
            .all(|offset| usize::try_from(offset).is_ok_and(|offset| text.is_char_boundary(offset)))
}

fn valid_preedit(text: &str, begin: i32, end: i32) -> bool {
    [begin, end].into_iter().all(|offset| {
        offset == -1 || usize::try_from(offset).is_ok_and(|offset| text.is_char_boundary(offset))
    })
}

impl GlobalDispatch<ZwpInputMethodManagerV2, ()> for InoxState {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<ZwpInputMethodManagerV2>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
    }
    fn can_view(client: Client, _: &()) -> bool {
        trusted(&client)
    }
}

impl Dispatch<ZwpInputMethodManagerV2, ()> for InoxState {
    fn request(
        state: &mut Self,
        client: &Client,
        _: &ZwpInputMethodManagerV2,
        request: imm::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let imm::Request::GetInputMethod { seat, input_method } = request {
            let object = init.init(input_method, ());
            if !trusted(client)
                || Seat::<Self>::from_resource(&seat).as_ref() != Some(&state.seat)
                || state.ime.method.as_ref().is_some_and(Resource::is_alive)
            {
                object.unavailable();
                return;
            }
            state.ime.method = Some(object);
            state.ime.serial = 0;
            state.ime.editing = Editing::default();
            if state.ime.active.is_some() && !state.lock.accepted() {
                state.ime_send_context(true);
            }
        }
    }
}

impl Dispatch<ZwpInputMethodV2, ()> for InoxState {
    fn request(
        state: &mut Self,
        _: &Client,
        object: &ZwpInputMethodV2,
        request: im::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        let current = state.ime.method.as_ref() == Some(object);
        match request {
            im::Request::GetInputPopupSurface { id, surface } => {
                let role = init.init(id, ());
                if !current {
                    return;
                }
                if give_role(&surface, POPUP_ROLE).is_err() {
                    object.post_error(0u32, "Surface already has a role");
                    return;
                }
                state.ime.popups.push(Candidate {
                    role,
                    surface: surface.clone(),
                    last_rectangle: None,
                    output: None,
                });
                state.ime_update_popups();
                state.update_surface_scale(&surface);
                state.repaint = true;
            }
            im::Request::GrabKeyboard { keyboard } => {
                let grab = init.init(keyboard, ());
                if !current {
                    return;
                }
                if state.ime.grab.as_ref().is_some_and(Resource::is_alive) {
                    object.post_error(0u32, "Keyboard is already grabbed");
                    return;
                }
                state.ime.grab = Some(grab);
                state.ime_sync_grab();
            }
            im::Request::Destroy => {}
            _ if !current || state.lock.accepted() || state.ime.active.is_none() => {}
            im::Request::CommitString { text } => state.ime.editing.commit = Some(text),
            im::Request::SetPreeditString {
                text,
                cursor_begin,
                cursor_end,
            } => {
                if !valid_preedit(&text, cursor_begin, cursor_end) {
                    object.post_error(0u32, "Invalid UTF-8 preedit offsets");
                    return;
                }
                state.ime.editing.preedit = Some((text, cursor_begin, cursor_end));
            }
            im::Request::DeleteSurroundingText {
                before_length,
                after_length,
            } => state.ime.editing.delete = Some((before_length, after_length)),
            im::Request::Commit { serial } => {
                let editing = std::mem::take(&mut state.ime.editing);
                if serial != state.ime.serial {
                    return;
                }
                let Some(text) = state
                    .ime
                    .texts
                    .iter()
                    .find(|text| Some(&text.object) == state.ime.active.as_ref())
                else {
                    return;
                };
                if let Some((value, begin, end)) = editing.preedit {
                    text.object.preedit_string(Some(value), begin, end);
                }
                if let Some(value) = editing.commit {
                    text.object.commit_string(Some(value));
                }
                if let Some((before, after)) = editing.delete {
                    text.object.delete_surrounding_text(before, after);
                }
                text.object.done(text.serial);
            }
            _ => {}
        }
    }
    fn destroyed(state: &mut Self, _: ClientId, object: &ZwpInputMethodV2, _: &()) {
        if state.ime.method.as_ref() != Some(object) {
            return;
        }
        state.ime.method = None;
        state.ime.editing = Editing::default();
        state.ime.popups.clear();
        state.ime.grab = None;
        state.ime_sync_grab();
        let keyboards: Vec<_> = state.ime.virtual_keys.keys().cloned().collect();
        for keyboard in keyboards {
            state.ime_release_virtual_keys(&keyboard);
        }
        if let Some(text) = state.ime.active.as_ref() {
            text.preedit_string(Some(String::new()), 0, 0);
            if let Some(instance) = state
                .ime
                .texts
                .iter()
                .find(|instance| instance.object == *text)
            {
                text.done(instance.serial);
            }
        }
        state.repaint = true;
    }
}

impl Dispatch<ZwpInputPopupSurfaceV2, ()> for InoxState {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ZwpInputPopupSurfaceV2,
        _: popup::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
    fn destroyed(state: &mut Self, _: ClientId, object: &ZwpInputPopupSurfaceV2, _: &()) {
        state.ime.popups.retain(|popup| popup.role != *object);
        state.repaint = true;
    }
}

impl Dispatch<ZwpInputMethodKeyboardGrabV2, ()> for InoxState {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ZwpInputMethodKeyboardGrabV2,
        _: grab::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
    fn destroyed(state: &mut Self, _: ClientId, object: &ZwpInputMethodKeyboardGrabV2, _: &()) {
        if state.ime.grab.as_ref() == Some(object) {
            state.ime.grab = None;
            state.ime_sync_grab();
        }
    }
}

smithay::reexports::wayland_server::delegate_global_dispatch!(InoxState: [
    inox_protocols::standard::wp::virtual_keyboard::server::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1:
    smithay::wayland::virtual_keyboard::VirtualKeyboardManagerGlobalData
] => VirtualKeyboardManagerState);
smithay::reexports::wayland_server::delegate_dispatch!(InoxState: [
    inox_protocols::standard::wp::virtual_keyboard::server::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1: ()
] => VirtualKeyboardManagerState);

impl Dispatch<ZwpVirtualKeyboardV1, VirtualKeyboardUserData<Self>> for InoxState {
    fn request(
        state: &mut Self,
        client: &Client,
        object: &ZwpVirtualKeyboardV1,
        request: vk::Request,
        data: &VirtualKeyboardUserData<Self>,
        display: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if !trusted(client)
            || (!matches!(request, vk::Request::Keymap { .. })
                && (state.lock.accepted() || state.seat.get_keyboard().is_none()))
        {
            return;
        }
        if matches!(request, vk::Request::Key { state, .. } if state > 1) {
            object.post_error(0u32, "Invalid key state");
            return;
        }
        if matches!(request, vk::Request::Key { key, .. } if key.checked_add(8).is_none()) {
            object.post_error(0u32, "Keycode exceeds the XKB range");
            return;
        }
        if let vk::Request::Key {
            key,
            state: pressed,
            ..
        } = &request
            && let Some(surface) = state.keyboard_focus()
        {
            let keys = state
                .ime
                .virtual_keys
                .entry(object.clone())
                .or_insert_with(|| ForwardedKeys {
                    surface: surface.clone(),
                    keys: HashSet::new(),
                });
            if keys.surface != surface {
                keys.surface = surface;
                keys.keys.clear();
            }
            if *pressed == 1 {
                keys.keys.insert(*key);
            } else {
                keys.keys.remove(key);
            }
        }
        <VirtualKeyboardManagerState as Dispatch<
            ZwpVirtualKeyboardV1,
            VirtualKeyboardUserData<Self>,
            Self,
        >>::request(state, client, object, request, data, display, init);
    }

    fn destroyed(
        state: &mut Self,
        _: ClientId,
        object: &ZwpVirtualKeyboardV1,
        _: &VirtualKeyboardUserData<Self>,
    ) {
        state.ime_release_virtual_keys(object);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]
        #[test]
        fn candidate_placement_stays_on_its_output(
            x in -4_194_304i32..4_194_304, y in -4_194_304i32..4_194_304,
            width in 1i32..16_384, height in 1i32..16_384,
            cursor_x in any::<i32>(), cursor_y in any::<i32>(),
            popup_width in 1i32..16_384, popup_height in 1i32..16_384,
        ) {
            let output = Rectangle::new((x, y).into(), (width, height).into());
            let popup = Rectangle::new((0, 0).into(), (popup_width, popup_height).into());
            let cursor = Rectangle::new((cursor_x, cursor_y).into(), (1, 20).into());
            let location = candidate_location(cursor, popup, output);
            prop_assert!(location.x >= x && location.y >= y);
            prop_assert!(location.x + popup_width.min(width) <= x + width);
            prop_assert!(location.y + popup_height.min(height) <= y + height);
        }
    }
    #[test]
    fn utf8_offsets_are_byte_offsets() {
        assert!(valid_surrounding("你好", 6, 3));
        assert!(!valid_surrounding("你好", 1, 3));
        assert!(!valid_surrounding("你好", -1, 3));
        assert!(!valid_surrounding(&"a".repeat(4001), 0, 0));
        assert!(valid_preedit("中文", -1, -1));
        assert!(!valid_preedit("中文", 1, 3));
    }
    #[test]
    fn candidates_flip_and_clamp_on_negative_outputs() {
        let output = Rectangle::new((-800, -600).into(), (800, 600).into());
        let popup = Rectangle::new((0, 0).into(), (200, 100).into());
        let cursor = Rectangle::new((-10, -20).into(), (1, 15).into());
        assert_eq!(
            candidate_location(cursor, popup, output),
            (-200, -120).into()
        );
        let cursor = Rectangle::new((-900, -700).into(), (1, 15).into());
        assert_eq!(
            candidate_location(cursor, popup, output),
            (-800, -600).into()
        );
    }
}
