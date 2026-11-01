use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::index::{Column, Point, Side};
use alacritty_terminal::index::Direction;
use alacritty_terminal::selection::Selection;
use alacritty_terminal::term::TermMode;
use arboard::{Clipboard, GetExtLinux, LinuxClipboardKind, SetExtLinux};
use koral::blocks::Activity;
use koral::config::{self, Config};
use koral::blink::Blink;
use koral::frame::{self, Chrome, CursorView, Frame, Layout, TabView};
use koral::links::{self, Link};
use koral::matugen;
use koral::input::{self, Action, Clicks, Mouse, binding};
use koral::keyboard::{self, KeyPress, Stage};
use koral::notify::{self, Limiter};
use koral::pty::{Geometry, Terminal, UserEvent};
use koral::render::{Renderer, rgba};
use koral::search::Search;
use koral::settings::{self, Command, FIELDS, Kind, Outcome, Settings, Zone};
use koral::tabs::{self, Bar, Hit, Ring, Tabs};
use koral::theme::Theme;
use koral::view;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::platform::wayland::WindowAttributesExtWayland;
use winit::window::{CursorIcon, UserAttentionType, Window, WindowId};

const CONFIRM_WINDOW: Duration = Duration::from_secs(3);

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("koral {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let event_loop =EventLoop::<UserEvent>::with_user_event().build().expect("create event loop");
    let proxy = event_loop.create_proxy();
    let mut app = App::new(proxy);
    event_loop.run_app(&mut app).expect("run event loop");
}

fn default_shell() -> String {
    let on_path = std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| dir.join("sharky").is_file())
    });
    if on_path {
        return "sharky".to_string();
    }
    if let Some(home) = std::env::var_os("HOME") {
        let local = Path::new(&home).join(".local/bin/sharky");
        if local.is_file() {
            return local.display().to_string();
        }
    }
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    None,
    Selecting,
    Tab(usize),
}

struct App {
    proxy: EventLoopProxy<UserEvent>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    tabs: Option<Tabs<Terminal>>,
    next_id: u64,
    config: Config,
    source: String,
    settings: Option<Settings>,
    settings_error: Option<String>,
    theme: Theme,
    home: Option<String>,
    clipboard: Option<Clipboard>,
    mods: ModifiersState,
    cursor: (f32, f32),
    pressed: Option<u8>,
    drag: Drag,
    focused: bool,
    confirming: Option<(u64, Instant)>,
    clicks: Clicks,
    preedit: Option<String>,
    ime_cursor: Option<(f32, f32)>,
    zoom: f32,
    blink: Blink,
    blink_shown: bool,
    window_title: String,
    notices: Limiter,
    search: Option<Search>,
    link: Option<Link>,
}

impl App {
    fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        if let Some(Err(error)) = config::home().map(|home| config::migrate(&home.join("koral"))) {
            eprintln!("koral: cannot rename config.koral: {error}");
        }
        let source = config::path().and_then(|path| std::fs::read_to_string(path).ok()).unwrap_or_default();
        let (config, errors) = config::parse(&source);
        for error in errors {
            eprintln!("koral: config.sharky {error}");
        }
        matugen::watch(proxy.clone());
        Self {
            proxy,
            window: None,
            renderer: None,
            tabs: None,
            next_id: 0,
            theme: matugen::theme(config.matugen),
            config,
            source,
            settings: None,
            settings_error: None,
            home: std::env::var("HOME").ok(),
            clipboard: Clipboard::new().ok(),
            mods: ModifiersState::empty(),
            cursor: (0.0, 0.0),
            pressed: None,
            drag: Drag::None,
            focused: true,
            confirming: None,
            clicks: Clicks::default(),
            preedit: None,
            ime_cursor: None,
            zoom: 0.0,
            blink: Blink::new(Instant::now()),
            blink_shown: true,
            window_title: String::new(),
            notices: Limiter::default(),
            search: None,
            link: None,
        }
    }

    fn layout(&self) -> Option<Layout> {
        let window = self.window.as_ref()?;
        let size = window.inner_size();
        Some(Layout {
            width: size.width as f32,
            height: size.height as f32,
            scale: window.scale_factor() as f32,
            padding: self.config.padding,
            cell: self.renderer.as_ref()?.cell(),
        })
    }

    fn geometry(&self) -> Option<Geometry> {
        let layout = self.layout()?;
        Some(Geometry {
            columns: layout.columns(),
            lines: layout.lines(),
            cell_width: layout.cell.width as u16,
            cell_height: layout.cell.height as u16,
        })
    }

    fn bar(&self) -> Option<Bar> {
        let layout = self.layout()?;
        Some(tabs::layout(self.tabs.as_ref()?.len(), layout.width, layout.strip()))
    }

    fn redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn active(&self) -> Option<&Terminal> {
        self.tabs.as_ref()?.active()
    }

    fn active_mut(&mut self) -> Option<&mut Terminal> {
        self.tabs.as_mut()?.active_mut()
    }

    fn open_tab(&mut self, event_loop: &ActiveEventLoop) {
        let Some(geometry) = self.geometry() else {
            return;
        };
        let cwd = self.active().and_then(|tab| {
            let session = tab.session.lock();
            session.blocks.latest()?.cwd.clone().map(PathBuf::from)
        });
        let id = self.next_id;
        self.next_id += 1;

        let shell = std::env::var("KORAL_SHELL")
            .ok()
            .or_else(|| self.config.shell.clone())
            .unwrap_or_else(default_shell);
        match Terminal::spawn(id, self.proxy.clone(), &shell, geometry, cwd) {
            Ok(terminal) => match &mut self.tabs {
                Some(tabs) => tabs.push(terminal),
                None => self.tabs = Some(Tabs::new(terminal)),
            },
            Err(error) => {
                eprintln!("koral: cannot start {shell}: {error}");
                if self.tabs.is_none() {
                    event_loop.exit();
                }
            }
        }
        self.redraw();
    }

    /// A running command gets a second chance: the tab asks instead of a dialog.
    fn request_close(&mut self, index: usize, event_loop: &ActiveEventLoop) {
        let Some(tab) = self.tabs.as_ref().and_then(|tabs| tabs.iter().nth(index)) else {
            return;
        };
        let id = tab.id;
        let running = tab.session.lock().blocks.activity() == Activity::Running;
        let confirmed = self.confirming.is_some_and(|(pending, at)| pending == id && at.elapsed() < CONFIRM_WINDOW);
        if running && !confirmed {
            self.confirming = Some((id, Instant::now()));
            event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + CONFIRM_WINDOW));
            self.redraw();
            return;
        }
        self.confirming = None;
        self.close_tab(index, event_loop);
    }

    fn close_tab(&mut self, index: usize, event_loop: &ActiveEventLoop) {
        if let Some(tabs) = &mut self.tabs {
            tabs.close(index);
            if tabs.is_empty() {
                event_loop.exit();
            }
        }
        self.redraw();
    }

    fn write_active(&mut self, bytes: Vec<u8>) {
        if let Some(tab) = self.active_mut() {
            tab.scroll = 0;
            tab.write(bytes);
        }
    }

    fn mode(&self) -> TermMode {
        self.active().map_or(TermMode::empty(), |tab| *tab.session.lock().term.mode())
    }

    fn ime(&mut self, ime: Ime) {
        match ime {
            Ime::Preedit(text, _) => self.preedit = Some(text).filter(|text| !text.is_empty()),
            Ime::Commit(text) if self.settings.is_some() => {
                self.preedit = None;
                text.chars().for_each(|ch| self.settings_command(Command::Type(ch)));
            }
            Ime::Commit(text) if self.search.is_some() => {
                self.preedit = None;
                self.search_type(&text);
            }
            Ime::Commit(text) => {
                self.preedit = None;
                self.write_active(text.into_bytes());
            }
            Ime::Enabled => {}
            Ime::Disabled => self.preedit = None,
        }
        self.redraw();
    }

    fn kitty_key(&self, event: &KeyEvent, stage: Stage) -> Option<Vec<u8>> {
        let unshifted = event.key_without_modifiers();
        let text = event.text.as_deref();
        let press = KeyPress { key: &event.logical_key, unshifted: &unshifted, text, mods: self.mods, stage };
        keyboard::encode(&press, self.mode())
    }

    fn run(&mut self, action: Action, event_loop: &ActiveEventLoop) {
        match action {
            Action::NewTab => self.open_tab(event_loop),
            Action::CloseTab => {
                if let Some(index) = self.tabs.as_ref().map(Tabs::active_index) {
                    self.request_close(index, event_loop);
                }
            }
            Action::NextTab => self.with_tabs(Tabs::next),
            Action::PreviousTab => self.with_tabs(Tabs::previous),
            Action::SelectTab(index) => self.with_tabs(|tabs| tabs.select(index)),
            Action::Copy => self.copy_selection(LinuxClipboardKind::Clipboard),
            Action::Paste => self.paste(LinuxClipboardKind::Clipboard),
            Action::PreviousPrompt => self.jump(view::previous_prompt),
            Action::NextPrompt => self.jump(view::next_prompt),
            Action::CopyOutput => self.copy_output(),
            Action::ToggleFold => self.toggle_fold(),
            Action::Search => {
                self.search.get_or_insert_with(Search::default);
            }
            Action::PageUp => self.scroll_lines(self.page()),
            Action::PageDown => self.scroll_lines(-self.page()),
            Action::OpenSettings => {
                self.search = None;
                self.toggle_settings();
            }
            Action::ZoomIn => self.zoom_to(self.zoom + 1.0),
            Action::ZoomOut => self.zoom_to(self.zoom - 1.0),
            Action::ZoomReset => self.zoom_to(0.0),
        }
        self.redraw();
    }

    /// For this run only: the font size in the config stays what the user chose.
    fn zoom_to(&mut self, zoom: f32) {
        let size = input::zoomed(self.config.font_size, zoom);
        self.zoom = size - self.config.font_size;
        if let Some(renderer) = &mut self.renderer {
            renderer.set_font(&self.config.face(), size);
        }
        self.resize_all();
    }

    fn toggle_settings(&mut self) {
        self.settings = match self.settings {
            Some(_) => None,
            None => Some(Settings::default()),
        };
        self.settings_error = None;
        self.redraw();
    }

    fn settings_command(&mut self, command: Command) {
        let Some(settings) = &mut self.settings else {
            return;
        };
        self.settings_error = None;
        match settings.handle(command, &self.config) {
            Outcome::Nothing => {}
            Outcome::Close => self.settings = None,
            Outcome::Change(key, value) => self.change_setting(key, &value),
        }
        self.redraw();
    }

    /// Live first, then saved: the file stays the source of truth for the next start.
    fn change_setting(&mut self, key: &str, value: &str) {
        if key == "matugen" && value == "on" && let Err(message) = matugen::install() {
            self.settings_error = Some(message);
            return;
        }
        let (source, config) = match config::update(&self.source, key, value) {
            Ok(updated) => updated,
            Err(message) => {
                self.settings_error = Some(message);
                return;
            }
        };
        let font_changed = config.face() != self.config.face() || config.font_size != self.config.font_size;
        self.source = source;
        self.config = config;
        self.theme = matugen::theme(self.config.matugen);

        if font_changed && let Some(renderer) = &mut self.renderer {
            renderer.set_font(&self.config.face(), input::zoomed(self.config.font_size, self.zoom));
        }
        self.resize_all();

        match config::path().map(|path| config::save(&path, &self.source)) {
            Some(Ok(())) => {}
            Some(Err(error)) => self.settings_error = Some(format!("Not saved: {error}")),
            None => self.settings_error = Some("Not saved: no config folder".to_string()),
        }
    }

    fn search_key(&mut self, key: &Key, text: Option<&str>) {
        let direction = if self.mods.shift_key() { Direction::Right } else { Direction::Left };
        match key {
            Key::Named(NamedKey::Escape) => self.search = None,
            Key::Named(NamedKey::Enter) => self.search_step(direction),
            Key::Named(NamedKey::Backspace) => self.search_edit(|query| {
                query.pop();
            }),
            _ if !self.mods.control_key() => self.search_type(text.unwrap_or("")),
            _ => {}
        }
        self.redraw();
    }

    fn search_type(&mut self, text: &str) {
        let typed: String = text.chars().filter(|ch| !ch.is_control()).collect();
        if !typed.is_empty() {
            self.search_edit(|query| query.push_str(&typed));
        }
    }

    fn search_edit(&mut self, change: impl FnOnce(&mut String)) {
        let Some(search) = &mut self.search else {
            return;
        };
        let mut query = search.query.clone();
        change(&mut query);
        search.set_query(query);
        self.search_step(Direction::Left);
    }

    fn search_step(&mut self, direction: Direction) {
        let Some(lines) = self.layout().map(|layout| layout.lines()) else {
            return;
        };
        let Some(mut search) = self.search.take() else {
            return;
        };
        if let Some(tab) = self.active_mut() {
            let mut session = tab.session.lock();
            if let Some(found) = search.step(&session.term, direction) {
                let line = found.start().line;
                let rows = view::rows(&session.term, &session.blocks);
                if let Some(id) = view::block_of(&rows, line) {
                    session.blocks.toggle_fold(id);
                }
                let rows = view::rows(&session.term, &session.blocks);
                if let Some(index) = view::row_of(&rows, line) {
                    tab.scroll = rows.len().saturating_sub(lines + index.saturating_sub(lines / 2));
                }
            }
        }
        self.search = Some(search);
        self.redraw();
    }

    fn settings_key(&mut self, key: &Key, text: Option<&str>) {
        let command = match key {
            Key::Named(NamedKey::ArrowUp) => Some(Command::Up),
            Key::Named(NamedKey::ArrowDown) => Some(Command::Down),
            Key::Named(NamedKey::ArrowLeft) => Some(Command::Less),
            Key::Named(NamedKey::ArrowRight) => Some(Command::More),
            Key::Named(NamedKey::Enter) => Some(Command::Confirm),
            Key::Named(NamedKey::Escape) => Some(Command::Cancel),
            Key::Named(NamedKey::Backspace) => Some(Command::Erase),
            _ => None,
        };
        if let Some(command) = command {
            self.settings_command(command);
        } else if let Some(text) = text {
            for ch in text.chars().filter(|ch| !ch.is_control()) {
                self.settings_command(Command::Type(ch));
            }
        }
    }

    /// True when the click belonged to the open panel.
    fn press_settings(&mut self) -> bool {
        let Some(layout) = self.layout() else {
            return false;
        };
        let Some(open) = &mut self.settings else {
            return false;
        };
        let (x, y) = self.cursor;
        match settings::zone(frame::panel(layout), x, y) {
            Zone::Outside => return false,
            Zone::Inside => {}
            Zone::Row(row) => {
                let chosen = open.selected == row;
                if !chosen {
                    open.editing = None;
                    open.selected = row;
                }
                if chosen || FIELDS[row].kind == Kind::Toggle {
                    self.settings_command(Command::Confirm);
                }
            }
            Zone::Less(row) => {
                open.editing = None;
                open.selected = row;
                self.settings_command(Command::Less);
            }
            Zone::More(row) => {
                open.editing = None;
                open.selected = row;
                self.settings_command(Command::More);
            }
        }
        self.redraw();
        true
    }

    fn tab_mut(&mut self, id: u64) -> Option<&mut Terminal> {
        self.tabs.as_mut().and_then(|tabs| tabs.iter_mut().find(|tab| tab.id == id))
    }

    fn bell(&mut self, id: u64) {
        let active = self.active().is_some_and(|tab| tab.id == id);
        let Ring { mark, urgent } = tabs::ring(active, self.focused);
        if let Some(tab) = self.tab_mut(id) {
            tab.rang |= mark;
        }
        if let (true, Some(window)) = (urgent, &self.window) {
            window.request_user_attention(Some(UserAttentionType::Informational));
        }
        self.redraw();
    }

    fn with_tabs(&mut self, change: impl FnOnce(&mut Tabs<Terminal>)) {
        self.search = None;
        if let Some(tabs) = &mut self.tabs {
            change(tabs);
        }
    }

    fn page(&self) -> i64 {
        self.layout().map_or(1, |layout| layout.lines() as i64 - 1)
    }

    fn jump(&mut self, target: fn(&[view::Row], usize) -> Option<usize>) {
        let Some(lines) = self.layout().map(|layout| layout.lines()) else {
            return;
        };
        let Some(tab) = self.active_mut() else {
            return;
        };
        let rows = {
            let session = tab.session.lock();
            view::rows(&session.term, &session.blocks)
        };
        let top = frame::visible_top(&rows, lines, tab.scroll);
        if let Some(index) = target(&rows, top) {
            tab.scroll = rows.len().saturating_sub(lines + index);
        }
    }

    fn copy_output(&mut self) {
        let Some(tab) = self.active() else {
            return;
        };
        let text = {
            let session = tab.session.lock();
            let rows = view::rows(&session.term, &session.blocks);
            view::fold_target(&rows, rows.len(), &session.blocks)
                .and_then(|id| view::output_text(&session.term, &session.blocks, id))
        };
        if let Some(text) = text {
            self.store(text, LinuxClipboardKind::Clipboard);
        }
    }

    fn toggle_fold(&mut self) {
        let Some(lines) = self.layout().map(|layout| layout.lines()) else {
            return;
        };
        let Some(tab) = self.active() else {
            return;
        };
        let scroll = tab.scroll;
        let mut session = tab.session.lock();
        let rows = view::rows(&session.term, &session.blocks);
        let end = frame::visible_top(&rows, lines, scroll) + lines;
        if let Some(id) = view::fold_target(&rows, end, &session.blocks) {
            session.blocks.toggle_fold(id);
        }
    }

    fn store(&mut self, text: String, kind: LinuxClipboardKind) {
        if let Some(clipboard) = &mut self.clipboard {
            let _ = clipboard.set().clipboard(kind).text(text);
        }
    }

    fn copy_selection(&mut self, kind: LinuxClipboardKind) {
        let text = self.active().and_then(|tab| tab.session.lock().term.selection_to_string());
        if let Some(text) = text {
            self.store(view::without_anchors(&text), kind);
        }
    }

    fn paste(&mut self, kind: LinuxClipboardKind) {
        let text = self.clipboard.as_mut().and_then(|clipboard| clipboard.get().clipboard(kind).text().ok());
        if let (Some(text), true) = (&text, self.search.is_some()) {
            let line = text.lines().next().unwrap_or("").to_string();
            self.search_edit(|query| query.push_str(&line));
            return;
        }
        if let Some(text) = text {
            let bytes = input::paste(&text, self.mode());
            self.write_active(bytes);
        }
    }

    fn scroll_lines(&mut self, delta: i64) {
        let Some(lines) = self.layout().map(|layout| layout.lines()) else {
            return;
        };
        let Some(tab) = self.active_mut() else {
            return;
        };
        let total = {
            let session = tab.session.lock();
            view::rows(&session.term, &session.blocks).len()
        };
        let most = total.saturating_sub(lines) as i64;
        tab.scroll = (tab.scroll as i64 + delta).clamp(0, most) as usize;
    }

    fn wheel(&mut self, lines: i64) {
        let mode = self.mode();
        if lines == 0 {
            return;
        }
        if mode.intersects(TermMode::MOUSE_MODE) && !self.mods.shift_key() {
            let button = if lines > 0 { 64 } else { 65 };
            for _ in 0..lines.abs() {
                self.report_mouse(button, true);
            }
        } else if mode.contains(TermMode::ALT_SCREEN) {
            if mode.contains(TermMode::ALTERNATE_SCROLL) {
                let key = match (lines > 0, mode.contains(TermMode::APP_CURSOR)) {
                    (true, true) => "\x1bOA",
                    (true, false) => "\x1b[A",
                    (false, true) => "\x1bOB",
                    (false, false) => "\x1b[B",
                };
                self.write_active(key.repeat(lines.unsigned_abs() as usize).into_bytes());
            }
        } else {
            self.scroll_lines(lines);
        }
        self.redraw();
    }

    fn report_mouse(&mut self, button: u8, pressed: bool) -> bool {
        let Some(layout) = self.layout() else {
            return false;
        };
        let (line, column) = layout.cell_at(self.cursor.0, self.cursor.1);
        let mouse = Mouse { button, column, line, pressed, mods: self.mods };
        match input::mouse_report(mouse, self.mode()) {
            Some(bytes) => {
                if let Some(tab) = self.active() {
                    tab.write(bytes);
                }
                true
            }
            None => false,
        }
    }

    fn point_under_cursor(&self) -> Option<(Point, Option<u32>)> {
        let layout = self.layout()?;
        let (row, column) = layout.cell_at(self.cursor.0, self.cursor.1);
        let tab = self.active()?;
        let session = tab.session.lock();
        let rows = view::rows(&session.term, &session.blocks);
        let top = frame::visible_top(&rows, layout.lines(), tab.scroll);
        let target = rows.get(top + row)?;
        Some((Point::new(target.line, Column(column)), target.block))
    }

    fn press_strip(&mut self, button: MouseButton, event_loop: &ActiveEventLoop) {
        let Some(bar) = self.bar() else {
            return;
        };
        match (hit_at(&bar, self.cursor), button) {
            (Hit::New, MouseButton::Left) => self.open_tab(event_loop),
            (Hit::Settings, MouseButton::Left) => self.toggle_settings(),
            (Hit::Close(index), MouseButton::Left) => self.request_close(index, event_loop),
            (Hit::Tab(index) | Hit::Close(index), MouseButton::Middle) => self.request_close(index, event_loop),
            (Hit::Tab(index), MouseButton::Left) => {
                self.with_tabs(|tabs| tabs.select(index));
                self.drag = Drag::Tab(index);
            }
            (Hit::Empty, MouseButton::Left) => {
                if let Some(window) = &self.window {
                    let _ = window.drag_window();
                }
            }
            _ => {}
        }
        self.redraw();
    }

    fn press_terminal(&mut self, button: MouseButton) {
        let code = match button {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            _ => return,
        };
        let mode = self.mode();
        let app_mouse = mode.intersects(TermMode::MOUSE_MODE) && !self.mods.shift_key();
        if button == MouseButton::Left && !app_mouse {
            self.update_link();
            if let Some(link) = &self.link {
                links::open(&link.uri);
                return;
            }
        }
        if app_mouse {
            self.pressed = Some(code);
            self.report_mouse(code, true);
            return;
        }
        if button == MouseButton::Middle {
            self.paste(LinuxClipboardKind::Primary);
            return;
        }
        if button != MouseButton::Left {
            return;
        }

        let Some(layout) = self.layout() else {
            return;
        };
        let Some((point, block)) = self.point_under_cursor() else {
            return;
        };
        if layout.in_gutter(self.cursor.0) {
            if let (Some(id), Some(tab)) = (block, self.active()) {
                tab.session.lock().blocks.toggle_fold(id);
            }
            self.redraw();
            return;
        }
        let clicks = self.clicks.press((point.line.0, point.column.0), Instant::now());
        let kind = input::selection_kind(clicks, self.mods);
        if let Some(tab) = self.active() {
            tab.session.lock().term.selection = Some(Selection::new(kind, point, Side::Left));
        }
        self.drag = Drag::Selecting;
        self.redraw();
    }

    fn release(&mut self) {
        if let Some(code) = self.pressed.take() {
            self.report_mouse(code, false);
        }
        if self.drag == Drag::Selecting {
            self.copy_selection(LinuxClipboardKind::Primary);
        }
        self.drag = Drag::None;
    }

    fn update_link(&mut self) {
        let in_grid = self.drag == Drag::None
            && self.layout().is_some_and(|layout| self.cursor.1 >= layout.strip() && !layout.in_gutter(self.cursor.0));
        let point = self.point_under_cursor().filter(|_| self.mods.control_key() && in_grid);
        let link = point.and_then(|(point, _)| links::at(&self.active()?.session.lock().term, point));
        if link != self.link {
            if let Some(window) = &self.window {
                window.set_cursor(if link.is_some() { CursorIcon::Pointer } else { CursorIcon::Default });
            }
            self.link = link;
            self.redraw();
        }
    }

    fn cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        self.cursor = (position.x as f32, position.y as f32);
        match self.drag {
            Drag::Selecting => {
                if let Some((point, _)) = self.point_under_cursor()
                    && let Some(tab) = self.active()
                    && let Some(selection) = &mut tab.session.lock().term.selection
                {
                    selection.update(point, Side::Right);
                }
            }
            Drag::Tab(from) => {
                if let Some(bar) = self.bar()
                    && let Hit::Tab(to) | Hit::Close(to) = hit_at(&bar, self.cursor)
                    && to != from
                {
                    self.with_tabs(|tabs| tabs.move_tab(from, to));
                    self.drag = Drag::Tab(to);
                }
            }
            Drag::None => {}
        }
        if self.layout().is_some_and(|layout| self.cursor.1 < layout.strip()) || self.drag != Drag::None {
            self.redraw();
        }
        self.update_link();
    }

    fn resize_all(&mut self) {
        let Some(geometry) = self.geometry() else {
            return;
        };
        if let Some(tabs) = &mut self.tabs {
            for tab in tabs.iter_mut() {
                tab.resize(geometry);
            }
        }
    }

    fn blinking(&self) -> bool {
        self.focused && self.active().is_some_and(|tab| tab.scroll == 0 && tab.session.lock().term.cursor_style().blinking)
    }

    fn draw(&mut self) {
        let (Some(layout), Some(bar)) = (self.layout(), self.bar()) else {
            return;
        };
        self.blink_shown = !self.blinking() || self.blink.visible(Instant::now());
        if let Some(tab) = self.active_mut() {
            tab.rang = false;
        }
        let Some(tabs) = &self.tabs else {
            return;
        };

        let confirming = self.confirming.filter(|(_, at)| at.elapsed() < CONFIRM_WINDOW).map(|(id, _)| id);
        let views: Vec<TabView> = tabs
            .iter()
            .map(|tab| {
                let session = tab.session.lock();
                TabView {
                    title: tabs::title(&session.blocks, tab.title.as_deref(), self.home.as_deref()),
                    activity: session.blocks.activity(),
                    confirming: confirming == Some(tab.id),
                    rang: tab.rang,
                }
            })
            .collect();

        let mut frame = Frame::default();
        let mut cursor = None;
        if let Some(tab) = tabs.active() {
            let session = tab.session.lock();
            let rows = view::rows(&session.term, &session.blocks);
            frame::draw_terminal(
                &mut frame,
                &session.term,
                &session.blocks,
                &rows,
                tab.scroll,
                CursorView { focused: self.focused, visible: self.blink_shown },
                layout,
                &self.theme,
            );
            frame::draw_pictures(&mut frame, &session.term, &session.graphics, &rows, tab.scroll, layout);
            cursor = frame::cursor_position(&session.term, &rows, tab.scroll, layout);
            let top = frame::visible_top(&rows, layout.lines(), tab.scroll);
            let shown = &rows[top..rows.len().min(top + layout.lines())];
            if let (Some(search), Some(first), Some(last)) = (&mut self.search, shown.first(), shown.last()) {
                let matches = search.visible(&session.term, first.line, last.line);
                frame::draw_search(&mut frame, &rows, top, &matches, search.current.as_ref(), layout, &self.theme);
            }
            if let Some(link) = &self.link {
                frame::draw_link(&mut frame, &rows, top, link, layout, &self.theme);
            }
        }
        if let (Some(text), Some((x, y))) = (&self.preedit, cursor) {
            frame::draw_preedit(&mut frame, text, x, y, layout, &self.theme);
        }

        let hit = hit_at(&bar, self.cursor);
        let chrome = Chrome {
            tabs: &views,
            active: tabs.active_index(),
            hovered: match hit {
                Hit::Tab(index) | Hit::Close(index) => Some(index),
                _ => None,
            },
            hovered_close: matches!(hit, Hit::Close(_)),
            hovered_new: hit == Hit::New,
            hovered_settings: hit == Hit::Settings,
            settings_open: self.settings.is_some(),
            bar: &bar,
        };
        frame::draw_chrome(&mut frame, &chrome, layout, &self.theme);
        if let Some(search) = &self.search {
            frame::draw_search_bar(&mut frame, &search.query, search.current.is_some(), layout, &self.theme);
        } else if let Some(link) = &self.link {
            frame::draw_link_bar(&mut frame, &link.uri, layout, &self.theme);
        }
        if let Some(settings) = &self.settings {
            frame::draw_settings(
                &mut frame,
                settings,
                &self.config,
                self.settings_error.as_deref(),
                layout,
                &self.theme,
            );
        }

        if let (Some(window), Some(view)) = (&self.window, views.get(tabs.active_index()))
            && view.title != self.window_title
        {
            window.set_title(&view.title);
            self.window_title = view.title.clone();
        }
        if let (Some(window), Some((x, y)), true) = (&self.window, cursor, cursor != self.ime_cursor) {
            let size = PhysicalSize::new(layout.cell.width, layout.cell.height);
            window.set_ime_cursor_area(PhysicalPosition::new(x, y), size);
            self.ime_cursor = cursor;
        }
        if let Some(renderer) = &mut self.renderer {
            renderer.draw(&frame.quads, &frame.pictures, &frame.lines, frame.clip, rgba(self.theme.background, self.config.opacity));
        }
    }
}

fn hit_at(bar: &Bar, cursor: (f32, f32)) -> Hit {
    tabs::hit(bar, cursor.0, cursor.1)
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("koral")
            .with_name("koral", "koral")
            .with_inner_size(LogicalSize::new(960.0, 600.0))
            .with_transparent(true)
            .with_decorations(false);
        let window = Arc::new(event_loop.create_window(attributes).expect("create window"));
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(Box::new(
            event_loop.owned_display_handle(),
        )));
        let (face, points) = (self.config.face(), self.config.font_size);
        self.renderer = Some(pollster::block_on(Renderer::new(window.clone(), instance, &face, points)));
        window.set_ime_allowed(true);
        self.window = Some(window);
        self.open_tab(event_loop);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Redraw => self.redraw(),
            UserEvent::Exited(id) => {
                let index = self.tabs.as_ref().and_then(|tabs| tabs.iter().position(|tab| tab.id == id));
                if let Some(index) = index {
                    self.close_tab(index, event_loop);
                }
            }
            UserEvent::Reply(id, text) => {
                if let Some(tab) = self.tabs.as_ref().and_then(|tabs| tabs.iter().find(|tab| tab.id == id)) {
                    tab.write(text.into_bytes());
                }
            }
            UserEvent::Copy(text) => self.store(text, LinuxClipboardKind::Clipboard),
            UserEvent::Title(id, title) => {
                if let Some(tab) = self.tab_mut(id) {
                    tab.title = title;
                }
                self.redraw();
            }
            UserEvent::Bell(id) => self.bell(id),
            UserEvent::Notify(id, notice) => {
                let in_view = self.focused && self.active().is_some_and(|tab| tab.id == id);
                if !in_view && self.notices.allow(Instant::now()) {
                    notify::show(&notice);
                }
            }
            UserEvent::Blink => {
                self.blink.reset(Instant::now());
                self.redraw();
            }
            UserEvent::Colors => {
                self.theme = matugen::theme(self.config.matugen);
                self.redraw();
            }
            UserEvent::Size(id, reply) => {
                if let (Some(geometry), Some(tab)) = (self.geometry(), self.tab_mut(id)) {
                    tab.write(reply(geometry.window_size()).into_bytes());
                }
            }
            UserEvent::Color(id, index, reply) => {
                let color = self.theme.palette(index);
                if let Some(tab) = self.tabs.as_ref().and_then(|tabs| tabs.iter().find(|tab| tab.id == id)) {
                    tab.write(reply(color).into_bytes());
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
                self.resize_all();
                self.redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.set_scale(scale_factor as f32);
                }
                self.resize_all();
                self.redraw();
            }
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::ModifiersChanged(modifiers) => {
                self.mods = modifiers.state();
                self.update_link();
            }
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                self.blink.reset(Instant::now());
                if self.mode().contains(TermMode::FOCUS_IN_OUT)
                    && let Some(tab) = self.active()
                {
                    tab.write(if focused { b"\x1b[I".to_vec() } else { b"\x1b[O".to_vec() });
                }
                self.redraw();
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.blink.reset(Instant::now());
                let action = binding(&event.logical_key, self.mods);
                if self.settings.is_some() && action != Some(Action::OpenSettings) {
                    self.settings_key(&event.logical_key, event.text.as_deref());
                    return;
                }
                if let Some(action) = action {
                    self.run(action, event_loop);
                    return;
                }
                if self.search.is_some() {
                    self.search_key(&event.logical_key, event.text.as_deref());
                    return;
                }
                let text = if self.mods.control_key() { None } else { event.text.as_deref() };
                let stage = if event.repeat { Stage::Repeat } else { Stage::Press };
                let bytes = self.kitty_key(&event, stage).or_else(|| input::encode(&event.logical_key, text, self.mods, self.mode()));
                if let Some(bytes) = bytes {
                    if let Some(tab) = self.active() {
                        tab.session.lock().term.selection = None;
                    }
                    self.write_active(bytes);
                    self.redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } if self.settings.is_none() && self.search.is_none() => {
                if let (Some(bytes), Some(tab)) = (self.kitty_key(&event, Stage::Release), self.active()) {
                    tab.write(bytes);
                }
            }
            WindowEvent::Ime(ime) => self.ime(ime),
            WindowEvent::CursorMoved { position, .. } => self.cursor_moved(position),
            WindowEvent::MouseInput { state: ElementState::Pressed, button, .. } => {
                if button == MouseButton::Left && self.press_settings() {
                    return;
                }
                if self.layout().is_some_and(|layout| self.cursor.1 < layout.strip()) {
                    self.press_strip(button, event_loop);
                } else {
                    if self.settings.take().is_some() {
                        self.redraw();
                        return;
                    }
                    self.press_terminal(button);
                }
            }
            WindowEvent::MouseInput { state: ElementState::Released, .. } => self.release(),
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => (y * 3.0) as i64,
                    MouseScrollDelta::PixelDelta(position) => {
                        let height = self.layout().map_or(20.0, |layout| layout.cell.height);
                        (position.y as f32 / height) as i64
                    }
                };
                self.wheel(lines);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.confirming.is_some_and(|(_, at)| at.elapsed() >= CONFIRM_WINDOW) {
            self.confirming = None;
            self.redraw();
        }
        let now = Instant::now();
        let mut blink = None;
        if self.blinking() {
            blink = self.blink.next(now);
            if self.blink.visible(now) != self.blink_shown {
                self.redraw();
            }
        }
        let confirm = self.confirming.map(|(_, at)| at + CONFIRM_WINDOW);
        event_loop.set_control_flow(match blink.into_iter().chain(confirm).min() {
            Some(deadline) => ControlFlow::WaitUntil(deadline),
            None => ControlFlow::Wait,
        });
    }
}
