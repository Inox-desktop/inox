//! Real application, input method and lock clients on one software-rendered host.

use calloop::EventLoop;
use inox_compositor::{ClientState, CompositorConfig, HeadlessOutput, InoxState};
use inox_protocols::{
    ipc::inox_ipc::client::inox_ipc as ipc,
    standard::{
        core::client::{
            wl_buffer, wl_callback, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry,
            wl_seat, wl_shm, wl_shm_pool, wl_surface,
        },
        ext::session_lock::client::{
            ext_session_lock_manager_v1 as lock_manager, ext_session_lock_v1 as lock,
        },
        wp::{
            input_method::client::{
                zwp_input_method_keyboard_grab_v2 as grab, zwp_input_method_manager_v2 as imm,
                zwp_input_method_v2 as im, zwp_input_popup_surface_v2 as popup,
            },
            text_input::client::{zwp_text_input_manager_v3 as tim, zwp_text_input_v3 as ti},
            virtual_keyboard::client::{
                zwp_virtual_keyboard_manager_v1 as vkm, zwp_virtual_keyboard_v1 as vk,
            },
        },
        xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base},
    },
};
use smithay::{
    backend::input::{ButtonState, KeyState},
    input::keyboard::{Keycode, xkb},
    reexports::wayland_server::Display,
    utils::Point,
};
use std::{
    collections::HashMap,
    io::{self, Write},
    os::{fd::AsFd, unix::net::UnixStream},
    sync::Arc,
    time::{Duration, Instant},
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};

#[derive(Default)]
struct Client {
    globals: HashMap<String, (u32, u32)>,
    synced: u32,
    text: Vec<(u32, ti::Event)>,
    method: Vec<(u32, im::Event)>,
    method_serial: u32,
    grab_keys: Vec<(u32, u32)>,
    key_events: usize,
    pointer_enters: usize,
    popup_rectangles: usize,
    last_popup_rectangle: Option<(i32, i32, i32, i32)>,
    frame_callbacks: usize,
    surface_enters: usize,
    surface_leaves: usize,
    locked: bool,
}

impl Dispatch<wl_registry::WlRegistry, ()> for Client {
    fn event(
        state: &mut Self,
        _: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            state.globals.insert(interface, (name, version));
        }
    }
}
impl Dispatch<wl_callback::WlCallback, u32> for Client {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        cookie: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if *cookie == 0 {
            state.frame_callbacks += 1;
        } else {
            state.synced = *cookie;
        }
    }
}
impl Dispatch<ti::ZwpTextInputV3, u32> for Client {
    fn event(
        state: &mut Self,
        _: &ti::ZwpTextInputV3,
        event: ti::Event,
        id: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.text.push((*id, event));
    }
}
impl Dispatch<im::ZwpInputMethodV2, u32> for Client {
    fn event(
        state: &mut Self,
        _: &im::ZwpInputMethodV2,
        event: im::Event,
        id: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if *id == 1 && matches!(event, im::Event::Done) {
            state.method_serial = state.method_serial.wrapping_add(1);
        }
        state.method.push((*id, event));
    }
}
impl Dispatch<grab::ZwpInputMethodKeyboardGrabV2, ()> for Client {
    fn event(
        state: &mut Self,
        _: &grab::ZwpInputMethodKeyboardGrabV2,
        event: grab::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let grab::Event::Key {
            key,
            state: pressed,
            ..
        } = event
        {
            state.grab_keys.push((key, pressed.into()));
        }
    }
}
impl Dispatch<wl_keyboard::WlKeyboard, ()> for Client {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_keyboard::Event::Key { .. }) {
            state.key_events += 1;
        }
    }
}
impl Dispatch<wl_pointer::WlPointer, ()> for Client {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_pointer::Event::Enter { .. }) {
            state.pointer_enters += 1;
        }
    }
}
impl Dispatch<popup::ZwpInputPopupSurfaceV2, ()> for Client {
    fn event(
        state: &mut Self,
        _: &popup::ZwpInputPopupSurfaceV2,
        event: popup::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let popup::Event::TextInputRectangle {
            x,
            y,
            width,
            height,
        } = event
        {
            state.popup_rectangles += 1;
            state.last_popup_rectangle = Some((x, y, width, height));
        }
    }
}
impl Dispatch<lock::ExtSessionLockV1, ()> for Client {
    fn event(
        state: &mut Self,
        _: &lock::ExtSessionLockV1,
        event: lock::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, lock::Event::Locked) {
            state.locked = true;
        }
    }
}
impl Dispatch<xdg_surface::XdgSurface, ()> for Client {
    fn event(
        _: &mut Self,
        surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            surface.ack_configure(serial);
        }
    }
}
wayland_client::delegate_noop!(Client: ignore wl_compositor::WlCompositor);
impl Dispatch<wl_surface::WlSurface, ()> for Client {
    fn event(
        state: &mut Self,
        _: &wl_surface::WlSurface,
        event: wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_surface::Event::Enter { .. } => state.surface_enters += 1,
            wl_surface::Event::Leave { .. } => state.surface_leaves += 1,
            _ => {}
        }
    }
}
wayland_client::delegate_noop!(Client: ignore wl_output::WlOutput);
wayland_client::delegate_noop!(Client: ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(Client: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(Client: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(Client: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(Client: ignore tim::ZwpTextInputManagerV3);
wayland_client::delegate_noop!(Client: ignore imm::ZwpInputMethodManagerV2);
wayland_client::delegate_noop!(Client: ignore vkm::ZwpVirtualKeyboardManagerV1);
wayland_client::delegate_noop!(Client: ignore vk::ZwpVirtualKeyboardV1);
wayland_client::delegate_noop!(Client: ignore xdg_wm_base::XdgWmBase);
wayland_client::delegate_noop!(Client: ignore xdg_toplevel::XdgToplevel);
wayland_client::delegate_noop!(Client: ignore lock_manager::ExtSessionLockManagerV1);
wayland_client::delegate_noop!(Client: ignore ipc::InoxIpc);

struct Peer {
    connection: Connection,
    queue: EventQueue<Client>,
    registry: wl_registry::WlRegistry,
    state: Client,
    cookie: u32,
    server_socket: UnixStream,
}

impl Peer {
    fn new(display: &mut Display<InoxState>, compositor: &mut InoxState, trusted: bool) -> Self {
        let (server, client) = UnixStream::pair().unwrap();
        let server_socket = server.try_clone().unwrap();
        client.set_nonblocking(true).unwrap();
        display
            .handle()
            .insert_client(
                server,
                Arc::new(if trusted {
                    ClientState::input_method()
                } else {
                    ClientState::default()
                }),
            )
            .unwrap();
        let connection = Connection::from_socket(client).unwrap();
        let queue = connection.new_event_queue();
        let registry = connection.display().get_registry(&queue.handle(), ());
        let mut peer = Self {
            connection,
            queue,
            registry,
            state: Client::default(),
            cookie: 0,
            server_socket,
        };
        peer.sync(display, compositor);
        peer
    }
    fn pump(&mut self) {
        self.connection.flush().unwrap();
        self.queue.dispatch_pending(&mut self.state).unwrap();
        if let Some(guard) = self.connection.prepare_read() {
            match guard.read() {
                Ok(_) => {}
                Err(wayland_client::backend::WaylandError::Io(error))
                    if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("client read failed: {error}"),
            }
        }
        self.queue.dispatch_pending(&mut self.state).unwrap();
    }
    fn sync(&mut self, display: &mut Display<InoxState>, compositor: &mut InoxState) {
        self.cookie += 1;
        self.connection
            .display()
            .sync(&self.queue.handle(), self.cookie);
        let deadline = Instant::now() + Duration::from_secs(20);
        while self.state.synced != self.cookie {
            assert!(Instant::now() < deadline, "Wayland sync timed out");
            self.pump();
            display.dispatch_clients(compositor).unwrap();
            display.flush_clients().unwrap();
            std::thread::yield_now();
        }
    }
    fn bind<I: Proxy + 'static>(&self) -> I
    where
        Client: Dispatch<I, ()>,
    {
        let (name, version) = self.state.globals[I::interface().name];
        self.registry.bind(name, version, &self.queue.handle(), ())
    }
    fn map(
        &mut self,
        display: &mut Display<InoxState>,
        compositor: &mut InoxState,
    ) -> wl_surface::WlSurface {
        let surface = self
            .bind::<wl_compositor::WlCompositor>()
            .create_surface(&self.queue.handle(), ());
        let xdg = self.bind::<xdg_wm_base::XdgWmBase>();
        let xdg_surface = xdg.get_xdg_surface(&surface, &self.queue.handle(), ());
        let _toplevel = xdg_surface.get_toplevel(&self.queue.handle(), ());
        surface.commit();
        self.sync(display, compositor);
        surface
    }
    fn text_input(&self, id: u32) -> ti::ZwpTextInputV3 {
        self.bind::<tim::ZwpTextInputManagerV3>().get_text_input(
            &self.bind::<wl_seat::WlSeat>(),
            &self.queue.handle(),
            id,
        )
    }
    fn method(&self, id: u32) -> im::ZwpInputMethodV2 {
        self.bind::<imm::ZwpInputMethodManagerV2>()
            .get_input_method(&self.bind::<wl_seat::WlSeat>(), &self.queue.handle(), id)
    }
    fn buffer(&self, width: i32, height: i32) -> wl_buffer::WlBuffer {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(&[0, 0, 255, 0].repeat((width * height) as usize))
            .unwrap();
        let pool = self.bind::<wl_shm::WlShm>().create_pool(
            file.as_fd(),
            width * height * 4,
            &self.queue.handle(),
            (),
        );
        pool.create_buffer(
            0,
            width,
            height,
            width * 4,
            wl_shm::Format::Xrgb8888,
            &self.queue.handle(),
            (),
        )
    }
    fn keyboard(&self) -> wl_keyboard::WlKeyboard {
        self.bind::<wl_seat::WlSeat>()
            .get_keyboard(&self.queue.handle(), ())
    }

    fn expect_rejection(&mut self, display: &mut Display<InoxState>, compositor: &mut InoxState) {
        self.cookie += 1;
        self.connection
            .display()
            .sync(&self.queue.handle(), self.cookie);
        self.connection.flush().unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            assert!(Instant::now() < deadline, "invalid client was not rejected");
            display.dispatch_clients(compositor).unwrap();
            display.flush_clients().unwrap();
            if let Some(guard) = self.connection.prepare_read() {
                match guard.read() {
                    Ok(_) => {}
                    Err(wayland_client::backend::WaylandError::Io(error))
                        if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(_) => return,
                }
            }
            if self.queue.dispatch_pending(&mut self.state).is_err() {
                return;
            }
            assert_ne!(
                self.state.synced, self.cookie,
                "invalid request was accepted"
            );
            std::thread::yield_now();
        }
    }
}

fn enable(text: &ti::ZwpTextInputV3) {
    text.enable();
    text.set_surrounding_text("你好 Inox".into(), 6, 3);
    text.set_content_type(ti::ContentHint::Completion, ti::ContentPurpose::Normal);
    text.set_cursor_rectangle(6, 10, 1, 12);
    text.commit();
}

fn has_commit(peer: &Peer, value: &str) -> bool {
    peer.state.text.iter().any(
        |(_, event)| matches!(event, ti::Event::CommitString { text: Some(text) } if text == value),
    )
}

fn red_at(frame: &[u8], x: i32, y: i32) -> bool {
    let offset = ((y * 480 + x) * 4) as usize;
    frame[offset] > 200 && frame[offset + 1] < 60 && frame[offset + 2] < 60
}

#[test]
fn text_input_transactions_candidates_and_security_lifecycle() {
    let event_loop: EventLoop<InoxState> = EventLoop::try_new().unwrap();
    let mut display = Display::<InoxState>::new().unwrap();
    let config = CompositorConfig {
        outputs: vec![HeadlessOutput {
            width: 480,
            height: 360,
            scale: 1.5,
            ..Default::default()
        }],
        lock_token: Some("ime-test-lock".into()),
        ..Default::default()
    };
    let mut state = InoxState::new(&display.handle(), &config, event_loop.get_signal()).unwrap();
    let mut app = Peer::new(&mut display, &mut state, false);
    assert!(app.state.globals.contains_key("zwp_text_input_manager_v3"));
    assert!(
        !app.state
            .globals
            .contains_key("zwp_input_method_manager_v2")
    );
    assert!(
        !app.state
            .globals
            .contains_key("zwp_virtual_keyboard_manager_v1")
    );
    let _surface = app.map(&mut display, &mut state);
    let text = app.text_input(1);
    let other_text = app.text_input(2);
    let keyboard = app.keyboard();
    app.sync(&mut display, &mut state);
    for id in [1, 2] {
        assert!(
            app.state
                .text
                .iter()
                .any(|(actual, event)| *actual == id && matches!(event, ti::Event::Enter { .. }))
        );
    }

    let mut engine = Peer::new(&mut display, &mut state, true);
    let mut attacker = Peer::new(&mut display, &mut state, false);
    let (name, _) = engine.state.globals["zwp_input_method_manager_v2"];
    let _: imm::ZwpInputMethodManagerV2 =
        attacker
            .registry
            .bind(name, 1, &attacker.queue.handle(), ());
    attacker.expect_rejection(&mut display, &mut state);
    app.sync(&mut display, &mut state);
    let method = engine.method(1);
    engine.sync(&mut display, &mut state);
    enable(&text);
    app.sync(&mut display, &mut state);
    engine.sync(&mut display, &mut state);
    assert!(
        engine
            .state
            .method
            .iter()
            .any(|(_, event)| matches!(event, im::Event::Activate))
    );
    assert!(engine.state.method.iter().any(|(_, event)| matches!(event, im::Event::SurroundingText { text, cursor: 6, anchor: 3 } if text == "你好 Inox")));
    assert!(
        engine
            .state
            .method
            .iter()
            .any(|(_, event)| matches!(event, im::Event::ContentType { .. }))
    );
    assert_eq!(engine.state.method_serial, 1);
    method.set_preedit_string("nihao".into(), 5, 5);
    engine.sync(&mut display, &mut state);
    app.sync(&mut display, &mut state);
    assert!(
        !app.state
            .text
            .iter()
            .any(|(_, event)| matches!(event, ti::Event::PreeditString { .. })),
        "preedit must wait for commit"
    );
    method.commit(engine.state.method_serial);
    engine.sync(&mut display, &mut state);
    app.sync(&mut display, &mut state);
    assert!(app.state.text.iter().any(|(id, event)| *id == 1 && matches!(event, ti::Event::PreeditString { text: Some(value), cursor_begin: 5, cursor_end: 5 } if value == "nihao")));
    method.commit_string("你好".into());
    method.delete_surrounding_text(3, 0);
    method.commit(engine.state.method_serial);
    engine.sync(&mut display, &mut state);
    app.sync(&mut display, &mut state);
    assert!(has_commit(&app, "你好"));
    assert!(app.state.text.iter().any(|(_, event)| matches!(
        event,
        ti::Event::DeleteSurroundingText {
            before_length: 3,
            after_length: 0
        }
    )));
    assert!(
        app.state
            .text
            .iter()
            .any(|(_, event)| matches!(event, ti::Event::Done { serial: 1 }))
    );
    method.commit_string("stale".into());
    method.commit(engine.state.method_serial.wrapping_sub(1));
    method.commit(engine.state.method_serial);
    engine.sync(&mut display, &mut state);
    app.sync(&mut display, &mut state);
    assert!(!has_commit(&app, "stale"));

    let duplicate = engine.method(2);
    engine.sync(&mut display, &mut state);
    assert!(
        engine
            .state
            .method
            .iter()
            .any(|(id, event)| *id == 2 && matches!(event, im::Event::Unavailable))
    );
    duplicate.destroy();
    engine.sync(&mut display, &mut state);
    method.commit_string("still alive".into());
    method.commit(engine.state.method_serial);
    engine.sync(&mut display, &mut state);
    app.sync(&mut display, &mut state);
    assert!(has_commit(&app, "still alive"));

    enable(&other_text);
    app.sync(&mut display, &mut state);
    engine.sync(&mut display, &mut state);
    method.commit_string("另一个输入框".into());
    method.commit(engine.state.method_serial);
    engine.sync(&mut display, &mut state);
    app.sync(&mut display, &mut state);
    assert!(app.state.text.iter().any(|(id, event)| *id == 2 && matches!(event, ti::Event::CommitString { text: Some(value) } if value == "另一个输入框")));
    other_text.destroy();
    app.sync(&mut display, &mut state);
    enable(&text);
    app.sync(&mut display, &mut state);
    engine.sync(&mut display, &mut state);

    let popup_surface = engine
        .bind::<wl_compositor::WlCompositor>()
        .create_surface(&engine.queue.handle(), ());
    let _output = engine.bind::<wl_output::WlOutput>();
    let popup = method.get_input_popup_surface(&popup_surface, &engine.queue.handle(), ());
    let buffer = engine.buffer(20, 10);
    let _pointer = engine
        .bind::<wl_seat::WlSeat>()
        .get_pointer(&engine.queue.handle(), ());
    popup_surface.attach(Some(&buffer), 0, 0);
    popup_surface.frame(&engine.queue.handle(), 0);
    popup_surface.commit();
    engine.sync(&mut display, &mut state);
    assert!(engine.state.popup_rectangles > 0);
    assert_eq!(engine.state.surface_enters, 1);
    assert_eq!(engine.state.last_popup_rectangle, Some((0, -12, 1, 12)));
    state.render_frame().unwrap();
    engine.sync(&mut display, &mut state);
    assert_eq!(engine.state.frame_callbacks, 1);
    let origin = state
        .window_location(state.focused_window().unwrap())
        .unwrap();
    let popup_x = origin.x + 6;
    let popup_y = origin.y + 22;
    let frame = state.read_frame_rgba().unwrap();
    let physical_x = (f64::from(popup_x) * 1.5).round() as i32;
    let physical_y = (f64::from(popup_y) * 1.5).round() as i32;
    assert!(red_at(&frame, physical_x + 1, physical_y + 1));
    assert!(red_at(&frame, physical_x + 28, physical_y + 13));
    assert!(!red_at(&frame, physical_x + 31, physical_y + 1));
    let keyboard_focus = state.keyboard_focus();
    state.inject_pointer_motion(Point::from((
        f64::from(popup_x + 2),
        f64::from(popup_y + 2),
    )));
    state.inject_pointer_button(0x110, ButtonState::Pressed);
    state.inject_pointer_button(0x110, ButtonState::Released);
    engine.sync(&mut display, &mut state);
    assert_eq!(engine.state.pointer_enters, 1);
    assert_eq!(state.keyboard_focus(), keyboard_focus);

    let grabbed = method.grab_keyboard(&engine.queue.handle(), ());
    engine.sync(&mut display, &mut state);
    state.inject_key(Keycode::new(38), KeyState::Pressed);
    state.inject_key(Keycode::new(38), KeyState::Released);
    engine.sync(&mut display, &mut state);
    app.sync(&mut display, &mut state);
    assert_eq!(engine.state.grab_keys, [(30, 1), (30, 0)]);
    assert_eq!(app.state.key_events, 0);

    let virtual_keyboard = engine
        .bind::<vkm::ZwpVirtualKeyboardManagerV1>()
        .create_virtual_keyboard(
            &engine.bind::<wl_seat::WlSeat>(),
            &engine.queue.handle(),
            (),
        );
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap = xkb::Keymap::new_from_names(
        &context,
        "evdev",
        "pc105",
        "us",
        "",
        None,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .unwrap();
    let mut file = tempfile::tempfile().unwrap();
    let mut bytes = keymap
        .get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1)
        .into_bytes();
    bytes.push(0);
    file.write_all(&bytes).unwrap();
    virtual_keyboard.keymap(1, file.as_fd(), bytes.len() as u32);
    virtual_keyboard.key(1, 30, 1);
    virtual_keyboard.key(2, 30, 0);
    engine.sync(&mut display, &mut state);
    app.sync(&mut display, &mut state);
    assert_eq!(
        app.state.key_events, 2,
        "unconsumed keys must reach the application without looping back to the IME"
    );
    assert_eq!(engine.state.grab_keys.len(), 2);

    let mut second = Peer::new(&mut display, &mut state, false);
    let second_surface = second.map(&mut display, &mut state);
    let second_text = second.text_input(3);
    second.sync(&mut display, &mut state);
    app.sync(&mut display, &mut state);
    assert!(
        app.state
            .text
            .iter()
            .any(|(id, event)| *id == 1 && matches!(event, ti::Event::Leave { .. }))
    );
    enable(&text);
    app.sync(&mut display, &mut state);
    engine.sync(&mut display, &mut state);
    method.commit_string("unfocused".into());
    method.commit(engine.state.method_serial);
    engine.sync(&mut display, &mut state);
    app.sync(&mut display, &mut state);
    assert!(!has_commit(&app, "unfocused"));
    enable(&second_text);
    second.sync(&mut display, &mut state);
    engine.sync(&mut display, &mut state);
    method.commit_string("第二个窗口".into());
    method.commit(engine.state.method_serial);
    engine.sync(&mut display, &mut state);
    second.sync(&mut display, &mut state);
    assert!(has_commit(&second, "第二个窗口"));
    second_text.disable();
    second_text.commit();
    second.sync(&mut display, &mut state);
    engine.sync(&mut display, &mut state);
    method.commit_string("disabled".into());
    method.commit(engine.state.method_serial);
    engine.sync(&mut display, &mut state);
    second.sync(&mut display, &mut state);
    assert!(!has_commit(&second, "disabled"));
    enable(&second_text);
    second.sync(&mut display, &mut state);

    let mut locker = Peer::new(&mut display, &mut state, false);
    locker
        .bind::<ipc::InoxIpc>()
        .claim_lock_role("ime-test-lock".into());
    let lock = locker
        .bind::<lock_manager::ExtSessionLockManagerV1>()
        .lock(&locker.queue.handle(), ());
    locker.sync(&mut display, &mut state);
    assert!(state.session_locked());
    engine.sync(&mut display, &mut state);
    let grab_count = engine.state.grab_keys.len();
    state.inject_key(Keycode::new(38), KeyState::Pressed);
    state.inject_key(Keycode::new(38), KeyState::Released);
    grabbed.release();
    let extra_grab = method.grab_keyboard(&engine.queue.handle(), ());
    extra_grab.release();
    virtual_keyboard.key(3, 30, 1);
    virtual_keyboard.key(4, 30, 0);
    method.commit_string("locked".into());
    method.commit(engine.state.method_serial);
    engine.sync(&mut display, &mut state);
    second.sync(&mut display, &mut state);
    assert_eq!(engine.state.grab_keys.len(), grab_count);
    assert!(!has_commit(&second, "locked"));
    state.render_frame().unwrap();
    locker.sync(&mut display, &mut state);
    assert!(locker.state.locked);
    assert!(
        !state
            .read_frame_rgba()
            .unwrap()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[0] > 200 && pixel[1] < 60)
    );
    lock.unlock_and_destroy();
    locker.sync(&mut display, &mut state);
    assert!(!state.session_locked());
    enable(&second_text);
    second.sync(&mut display, &mut state);
    engine.sync(&mut display, &mut state);
    let _grab = method.grab_keyboard(&engine.queue.handle(), ());
    engine.sync(&mut display, &mut state);
    method.set_preedit_string("待恢复".into(), 9, 9);
    method.commit(engine.state.method_serial);
    engine.sync(&mut display, &mut state);
    second.sync(&mut display, &mut state);
    let second_keyboard = second.keyboard();
    second.sync(&mut display, &mut state);
    virtual_keyboard.key(5, 31, 1);
    engine.sync(&mut display, &mut state);
    second.sync(&mut display, &mut state);
    let before_crash = second.state.key_events;
    engine
        .server_socket
        .shutdown(std::net::Shutdown::Both)
        .unwrap();
    display.dispatch_clients(&mut state).unwrap();
    second.sync(&mut display, &mut state);
    assert_eq!(
        second.state.key_events,
        before_crash + 1,
        "a crashed virtual keyboard must release its forwarded key"
    );
    assert!(second.state.text.iter().any(|(_, event)| matches!(event, ti::Event::PreeditString { text: Some(value), .. } if value.is_empty())));
    let before = second.state.key_events;
    state.inject_key(Keycode::new(38), KeyState::Pressed);
    state.inject_key(Keycode::new(38), KeyState::Released);
    second.sync(&mut display, &mut state);
    assert_eq!(
        second.state.key_events,
        before + 2,
        "input method crash must restore direct keyboard input"
    );
    let mut restarted = Peer::new(&mut display, &mut state, true);
    let new_method = restarted.method(1);
    restarted.sync(&mut display, &mut state);
    assert!(
        restarted
            .state
            .method
            .iter()
            .any(|(_, event)| matches!(event, im::Event::Activate))
    );
    new_method.commit_string("恢复输入".into());
    new_method.commit(restarted.state.method_serial);
    restarted.sync(&mut display, &mut state);
    second.sync(&mut display, &mut state);
    assert!(has_commit(&second, "恢复输入"));
    second_text.destroy();
    second.sync(&mut display, &mut state);
    restarted.sync(&mut display, &mut state);
    assert!(
        restarted
            .state
            .method
            .iter()
            .any(|(_, event)| matches!(event, im::Event::Deactivate))
    );
    drop((other_text, keyboard, second_keyboard, second_surface, popup));
    new_method.destroy();
    restarted.sync(&mut display, &mut state);
    let mut malformed = Peer::new(&mut display, &mut state, false);
    let _surface = malformed.map(&mut display, &mut state);
    let invalid_text = malformed.text_input(1);
    invalid_text.enable();
    invalid_text.set_surrounding_text("中文".into(), 1, 0);
    malformed.expect_rejection(&mut display, &mut state);
    app.sync(&mut display, &mut state);
    if std::env::var_os("INOX_FCITX5_E2E").is_some_and(|value| value == "1") {
        fcitx5_roundtrip(&mut display, &mut state);
    }
}

fn fcitx5_roundtrip(display: &mut Display<InoxState>, compositor: &mut InoxState) {
    use std::{
        fs,
        os::fd::OwnedFd,
        process::{Child, Command, Stdio},
    };
    struct Engine(Child);
    impl Drop for Engine {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join("config/fcitx5");
    fs::create_dir_all(config.join("conf")).unwrap();
    fs::write(config.join("profile"), "[Groups/0]\nName=Default\nDefault Layout=us\nDefaultIM=pinyin\n\n[Groups/0/Items/0]\nName=keyboard-us\nLayout=\n\n[Groups/0/Items/1]\nName=pinyin\nLayout=\n\n[GroupOrder]\n0=Default\n").unwrap();
    fs::write(config.join("config"), "[Behavior]\nActiveByDefault=True\n").unwrap();
    // Disable the network-based optional engine even if the distro enables it.
    fs::write(
        config.join("conf/cloudpinyin.conf"),
        "[GlobalSection]\nEnabled=False\n",
    )
    .unwrap();
    let (server, child) = UnixStream::pair().unwrap();
    display
        .handle()
        .insert_client(server, Arc::new(ClientState::input_method()))
        .unwrap();
    let stdin: OwnedFd = child.into();
    let mut engine = Engine(
        Command::new("fcitx5")
            .arg("--disable=cloudpinyin,notification,updatechecker")
            .env("WAYLAND_SOCKET", "0")
            .env("XDG_SESSION_TYPE", "wayland")
            .env("XDG_CURRENT_DESKTOP", "Inox")
            .env("HOME", home.path())
            .env("XDG_CONFIG_HOME", home.path().join("config"))
            .env("XDG_DATA_HOME", home.path().join("data"))
            .env("XDG_CACHE_HOME", home.path().join("cache"))
            .env("XDG_RUNTIME_DIR", home.path())
            .env_remove("DISPLAY")
            .stdin(Stdio::from(stdin))
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("CI must install fcitx5 and fcitx5-pinyin"),
    );
    let mut app = Peer::new(display, compositor, false);
    let _surface = app.map(display, compositor);
    let text = app.text_input(1);
    text.enable();
    text.set_surrounding_text(String::new(), 0, 0);
    text.set_content_type(ti::ContentHint::None, ti::ContentPurpose::Normal);
    text.set_cursor_rectangle(6, 10, 1, 12);
    text.commit();
    app.sync(display, compositor);
    let deadline = Instant::now() + Duration::from_secs(30);
    while !compositor.seat().get_keyboard().unwrap().is_grabbed() {
        assert!(
            Instant::now() < deadline,
            "Fcitx5 did not activate its keyboard grab"
        );
        assert!(
            engine.0.try_wait().unwrap().is_none(),
            "Fcitx5 exited during startup"
        );
        display.dispatch_clients(compositor).unwrap();
        display.flush_clients().unwrap();
        app.pump();
        std::thread::sleep(Duration::from_millis(2));
    }
    for key in [57, 31, 43, 38, 32] {
        compositor.inject_key(Keycode::new(key), KeyState::Pressed);
        compositor.inject_key(Keycode::new(key), KeyState::Released);
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    while !app.state.text.iter().any(|(_, event)| matches!(event, ti::Event::PreeditString { text: Some(value), .. } if !value.is_empty())) {
        assert!(Instant::now() < deadline, "Fcitx5 did not produce Chinese preedit");
        display.dispatch_clients(compositor).unwrap();
        display.flush_clients().unwrap();
        app.pump();
        compositor.render_frame().unwrap();
        std::thread::sleep(Duration::from_millis(2));
    }
    compositor.inject_key(Keycode::new(65), KeyState::Pressed);
    compositor.inject_key(Keycode::new(65), KeyState::Released);
    let deadline = Instant::now() + Duration::from_secs(20);
    while !has_commit(&app, "你好") {
        assert!(
            Instant::now() < deadline,
            "Fcitx5 did not commit 你好: {:?}",
            app.state.text
        );
        assert!(engine.0.try_wait().unwrap().is_none());
        display.dispatch_clients(compositor).unwrap();
        display.flush_clients().unwrap();
        app.pump();
        compositor.render_frame().unwrap();
        std::thread::sleep(Duration::from_millis(2));
    }
}
