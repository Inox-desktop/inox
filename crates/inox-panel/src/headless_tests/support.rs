//! The harness the panel's projection tests run through.
//!
//! There is no renderer here on purpose. The panel's QML tree is a pure function
//! of a Qt-free scene ([`crate::view`]), so what a surface draws is asserted as
//! values — a rectangle's colour, a label's text size, the position the clock is
//! laid out at — rather than as pixels. That is both stricter (no tolerance
//! windows, no anti-aliasing guesses) and independent of a compositor or a Qt
//! application, which is what lets these tests run anywhere.
//!
//! Interaction goes through [`crate::layer::Input`], the same pointer half the
//! `wl_pointer` handler drives, so a click asserted here is the click the panel
//! acts on.

#![allow(dead_code)]

use serde_json::Value;

use inox_common::i18n::{Locale, Localizer};
use inox_config::{PanelConfig, PanelSurface, Rgba, ShellConfig, Theme as Tokens};

use crate::app::{self, Message, State};
use crate::contract::ServiceFailure;
use crate::layer::{Button, Input, Point, Pointer};
use crate::tasks::{SessionGeneration, WindowId, WindowKey, WindowState};
use crate::tray::{TrayItem, TrayItemId, TrayItemUpdate};
use crate::view::Scene;

/// The width the acceptance scenes are projected at (Q02).
pub const WIDTH: f32 = 1280.0;

/// A panel: its state, its tokens, its configured surface and its size.
pub struct Panel {
    state: State,
    tokens: Tokens,
    shell: ShellConfig,
    input: Input,
    width: f32,
    height: f32,
}

impl Panel {
    /// A taskbar panel with the default configuration and the dark tokens.
    pub fn new() -> Self {
        Self::with(PanelConfig::default(), Tokens::builtin_dark())
    }

    /// A panel of `config`'s surface with `tokens`.
    pub fn with(config: PanelConfig, tokens: Tokens) -> Self {
        Self::with_shell(
            ShellConfig {
                panel: config,
                ..ShellConfig::default()
            },
            tokens,
        )
    }

    /// A panel of the surface `shell.panel.surface` names.
    pub fn with_shell(shell: ShellConfig, tokens: Tokens) -> Self {
        let height = app::window_height(&shell, &tokens);

        Self::at(shell, tokens, WIDTH, height)
    }

    /// A panel of `shell`'s surface projected at exactly `width` × `height`.
    pub fn at(shell: ShellConfig, tokens: Tokens, width: f32, height: f32) -> Self {
        let localizer = catalogs();
        let state = State::with_shell(shell.clone(), tokens.clone(), localizer);

        Self {
            state,
            tokens,
            shell,
            input: Input::new(width as u32, height as u32),
            width,
            height,
        }
    }

    /// The panel's state.
    pub fn state(&self) -> &State {
        &self.state
    }

    /// The panel's state, for a test that has to populate it directly.
    pub fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }

    /// The tokens this panel is drawn with.
    pub fn tokens(&self) -> &Tokens {
        &self.tokens
    }

    /// The configuration this panel runs under.
    pub fn shell(&self) -> &ShellConfig {
        &self.shell
    }

    /// The size the panel is projected at.
    pub fn size(&self) -> (f32, f32) {
        (self.width, self.height)
    }

    /// Applies one message the way the runtime would.
    pub fn update(&mut self, message: Message) {
        app::update(&mut self.state, message);
    }

    /// Applies several messages in order.
    pub fn update_all(&mut self, messages: impl IntoIterator<Item = Message>) {
        for message in messages {
            self.update(message);
        }
    }

    /// The scene this panel projects right now.
    pub fn scene(&self) -> Scene {
        self.input.project(&self.state)
    }

    /// Feeds one pointer event, applying whatever it produced.
    pub fn pointer(&mut self, event: Pointer) -> crate::layer::Produced {
        self.input.pointer(&mut self.state, event)
    }

    /// Clicks at `point`: enter, press, release.
    ///
    /// Returns the messages the click produced — which is the whole of what a
    /// click means to this panel (D01).
    pub fn click(&mut self, point: Point) -> Vec<Message> {
        self.pointer(Pointer::Enter(point));
        self.pointer(Pointer::Press(Button::Left));

        self.pointer(Pointer::Release(Button::Left)).messages
    }

    /// Hovers at `point`, which is what a hovered entry is drawn from.
    pub fn hover(&mut self, point: Point) {
        self.pointer(Pointer::Enter(point));
    }

    /// Presses and holds at `point`, which is what a pressed entry is drawn from.
    pub fn press(&mut self, point: Point) {
        self.pointer(Pointer::Enter(point));
        self.pointer(Pointer::Press(Button::Left));
    }
}

/// The panel's catalogs, parsed once for a test.
pub fn catalogs() -> std::rc::Rc<Localizer> {
    std::rc::Rc::new(
        crate::i18n::load(&Locale::default_locale()).expect("the panel's catalogs parse"),
    )
}

/// A window key in session generation `generation()`.
pub fn key(window: u64) -> WindowKey {
    WindowKey::new(WindowId::new(window), SessionGeneration::new(1))
}

/// The generation the fixtures report.
pub fn generation() -> SessionGeneration {
    SessionGeneration::new(1)
}

/// A window state.
pub fn window(app: &str, title: &str) -> WindowState {
    WindowState {
        app_id: app.to_owned(),
        title: title.to_owned(),
        ..WindowState::default()
    }
}

/// A window the compositor reports as focused.
pub fn focused_window(app: &str, title: &str) -> WindowState {
    WindowState {
        app_id: app.to_owned(),
        title: title.to_owned(),
        focused: true,
        ..WindowState::default()
    }
}

/// A panel showing three applications, one of them focused.
pub fn populated(tokens: Tokens) -> Panel {
    populated_with(PanelConfig::default(), tokens)
}

/// A panel of `config` (surface and position included) showing three
/// applications, one of them focused.
pub fn populated_with(config: PanelConfig, tokens: Tokens) -> Panel {
    let mut panel = Panel::with(config, tokens);

    panel.update(Message::ServiceReady {
        generation: generation(),
    });
    panel.update(Message::Snapshot {
        generation: generation(),
        windows: vec![
            (key(1), window("firefox", "Inox — Documentation")),
            (
                key(2),
                focused_window("org.inkscape.Inkscape", "Drawing.svg"),
            ),
            (key(3), window("kitty", "~/Projects/Inox")),
        ],
    });

    panel
}

/// A panel of `mode` showing three applications, one of them focused.
pub fn populated_surface(mode: PanelSurface, tokens: Tokens) -> Panel {
    let mut panel = surface_panel(mode, tokens);

    panel.update(Message::ServiceReady {
        generation: generation(),
    });
    panel.update(Message::Snapshot {
        generation: generation(),
        windows: vec![
            (key(1), window("firefox", "Inox — Documentation")),
            (
                key(2),
                focused_window("org.inkscape.Inkscape", "Drawing.svg"),
            ),
            (key(3), window("kitty", "~/Projects/Inox")),
        ],
    });

    panel
}

/// A panel of `mode` with nothing running.
pub fn empty_surface(mode: PanelSurface, tokens: Tokens) -> Panel {
    let mut panel = surface_panel(mode, tokens);

    panel.update(Message::ServiceReady {
        generation: generation(),
    });
    panel.update(Message::Snapshot {
        generation: generation(),
        windows: Vec::new(),
    });

    panel
}

/// A panel of `mode` whose window service is down.
pub fn unavailable_surface(mode: PanelSurface, tokens: Tokens) -> Panel {
    let mut panel = surface_panel(mode, tokens);

    panel.update(Message::ServiceUnavailable {
        reason: ServiceFailure::Disconnected,
    });

    panel
}

/// A panel of `mode` with the default configuration.
pub fn surface_panel(mode: PanelSurface, tokens: Tokens) -> Panel {
    let mut shell = ShellConfig::default();
    shell.panel.surface = mode;

    Panel::with_shell(shell, tokens)
}

/// Three windows, one per application.
pub fn three_tasks() -> Vec<(WindowKey, WindowState)> {
    vec![
        (key(1), window("firefox", "Inox — Documentation")),
        (
            key(2),
            focused_window("org.inkscape.Inkscape", "Drawing.svg"),
        ),
        (key(3), window("kitty", "~/Projects/Inox")),
    ]
}

/// Registers one status notifier item with `title` on the panel.
pub fn add_tray_item(panel: &mut Panel, title: &str) {
    let id = TrayItemId {
        bus_name: ":1.42".to_owned(),
        object_path: "/StatusNotifierItem".to_owned(),
    };

    panel.update_all([
        Message::TraySnapshot {
            items: vec![TrayItem::new(id.clone())],
        },
        Message::TrayUpdated {
            id,
            update: TrayItemUpdate::Title(title.to_owned()),
        },
    ]);
}

/// A token colour as the projection spells it.
pub fn token(colour: Rgba) -> String {
    crate::theme::qml_color(colour)
}

/// Every drawing item of a scene.
pub fn items(scene: &Scene) -> &[Value] {
    scene.json["items"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// The scene's filled rectangles.
pub fn rects(scene: &Scene) -> Vec<&Value> {
    items(scene)
        .iter()
        .filter(|item| is_kind(item, "rect"))
        .collect()
}

/// The scene's lines of text.
pub fn texts(scene: &Scene) -> Vec<&Value> {
    items(scene)
        .iter()
        .filter(|item| is_kind(item, "text"))
        .collect()
}

fn is_kind(item: &Value, kind: &str) -> bool {
    item["kind"].as_str() == Some(kind)
}

/// A number field of an item.
pub fn number(item: &Value, field: &str) -> f32 {
    item[field].as_f64().unwrap_or_default() as f32
}

/// A string field of an item.
pub fn string<'a>(item: &'a Value, field: &str) -> &'a str {
    item[field].as_str().unwrap_or_default()
}

/// The colour of an item.
pub fn color(item: &Value) -> &str {
    string(item, "color")
}

/// Whether the item's box contains `(x, y)`.
pub fn covers(item: &Value, x: f32, y: f32) -> bool {
    let (left, top) = (number(item, "x"), number(item, "y"));

    x >= left && x < left + number(item, "w") && y >= top && y < top + number(item, "h")
}

/// The topmost rectangle drawn at `(x, y)`.
pub fn rect_at(scene: &Scene, x: f32, y: f32) -> Option<&Value> {
    rects(scene)
        .into_iter()
        .rev()
        .find(|item| covers(item, x, y))
}

/// The line of text whose `text` field is exactly `wanted`.
pub fn text_item<'a>(scene: &'a Scene, wanted: &str) -> Option<&'a Value> {
    texts(scene)
        .into_iter()
        .find(|item| string(item, "text") == wanted)
}

/// Every sentence a scene draws, in the order it draws them.
pub fn sentences(scene: &Scene) -> Vec<String> {
    texts(scene)
        .into_iter()
        .map(|item| string(item, "text").to_owned())
        .collect()
}

/// Whether any line of text contains `needle`.
pub fn has_text(scene: &Scene, needle: &str) -> bool {
    sentences(scene)
        .iter()
        .any(|sentence| sentence.contains(needle))
}

/// The colours the scene draws with, distinct and in the order they first appear.
pub fn colours(scene: &Scene) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();

    for item in items(scene) {
        let colour = color(item).to_owned();

        if !seen.contains(&colour) {
            seen.push(colour);
        }
    }

    seen
}

/// A `PanelConfig` that draws the menu bar or the dock instead of the taskbar.
pub fn shell_for(mode: PanelSurface) -> ShellConfig {
    let mut shell = ShellConfig::default();
    shell.panel.surface = mode;

    shell
}
