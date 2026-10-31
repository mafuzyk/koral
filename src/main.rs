use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::index::{Column, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use arboard::{Clipboard, GetExtLinux, LinuxClipboardKind, SetExtLinux};
use koral::blocks::Activity;
use koral::frame::{self, Chrome, Frame, Layout, OPACITY, TabView};
use koral::input::{self, Action, Mouse, binding};
use koral::pty::{Geometry, Terminal, UserEvent};
use koral::render::{Renderer, rgba};
use koral::tabs::{self, Bar, Hit, Tabs};
use koral::theme::Theme;
use koral::view;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::platform::wayland::WindowAttributesExtWayland;
use winit::window::{Window, WindowId};

const CONFIRM_WINDOW: Duration = Duration::from_secs(3);

fn main() {
    let event_loop = EventLoop::<UserEvent>::with_user_event().build().expect("create event loop");
    let proxy = event_loop.create_proxy();
    let mut app = App::new(proxy);
    event_loop.run_app(&mut app).expect("run event loop");
}

fn default_shell() -> String {
    if let Ok(shell) = std::env::var("KORAL_SHELL") {
        return shell;
    }
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
    shell: String,
    theme: Theme,
    home: Option<String>,
    clipboard: Option<Clipboard>,
    mods: ModifiersState,
    cursor: (f32, f32),
    pressed: Option<u8>,
    drag: Drag,
    focused: bool,
    confirming: Option<(u64, Instant)>,
}

impl App {
    fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        Self {
            proxy,
            window: None,
            renderer: None,
            tabs: None,
            next_id: 0,
            shell: default_shell(),
            theme: Theme::default(),
            home: std::env::var("HOME").ok(),
            clipboard: Clipboard::new().ok(),
            mods: ModifiersState::empty(),
            cursor: (0.0, 0.0),
            pressed: None,
            drag: Drag::None,
            focused: true,
            confirming: None,
        }
    }

    fn layout(&self) -> Option<Layout> {
        let window = self.window.as_ref()?;
        let size = window.inner_size();
        Some(Layout {
            width: size.width as f32,
            height: size.height as f32,
            scale: window.scale_factor() as f32,
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

        match Terminal::spawn(id, self.proxy.clone(), &self.shell, geometry, cwd) {
            Ok(terminal) => match &mut self.tabs {
                Some(tabs) => tabs.push(terminal),
                None => self.tabs = Some(Tabs::new(terminal)),
            },
            Err(error) => {
                eprintln!("koral: cannot start {}: {error}", self.shell);
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
            Action::PageUp => self.scroll_lines(self.page()),
            Action::PageDown => self.scroll_lines(-self.page()),
        }
        self.redraw();
    }

    fn with_tabs(&mut self, change: impl FnOnce(&mut Tabs<Terminal>)) {
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
        if mode.intersects(TermMode::MOUSE_MODE) && !self.mods.shift_key() {
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
        if let Some(tab) = self.active() {
            tab.session.lock().term.selection = Some(Selection::new(SelectionType::Simple, point, Side::Left));
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

    fn draw(&mut self) {
        let (Some(layout), Some(bar)) = (self.layout(), self.bar()) else {
            return;
        };
        let Some(tabs) = &self.tabs else {
            return;
        };

        let confirming = self.confirming.filter(|(_, at)| at.elapsed() < CONFIRM_WINDOW).map(|(id, _)| id);
        let views: Vec<TabView> = tabs
            .iter()
            .map(|tab| {
                let session = tab.session.lock();
                TabView {
                    title: tabs::title(&session.blocks, self.home.as_deref()),
                    activity: session.blocks.activity(),
                    confirming: confirming == Some(tab.id),
                }
            })
            .collect();

        let mut frame = Frame::default();
        if let Some(tab) = tabs.active() {
            let session = tab.session.lock();
            let rows = view::rows(&session.term, &session.blocks);
            frame::draw_terminal(
                &mut frame,
                &session.term,
                &session.blocks,
                &rows,
                tab.scroll,
                self.focused,
                layout,
                &self.theme,
            );
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
            bar: &bar,
        };
        frame::draw_chrome(&mut frame, &chrome, layout, &self.theme);

        if let (Some(window), Some(view)) = (&self.window, views.get(tabs.active_index())) {
            window.set_title(&view.title);
        }
        if let Some(renderer) = &mut self.renderer {
            renderer.draw(&frame.quads, &frame.lines, rgba(self.theme.background, OPACITY));
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
        self.renderer = Some(pollster::block_on(Renderer::new(window.clone(), instance)));
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
            WindowEvent::ModifiersChanged(modifiers) => self.mods = modifiers.state(),
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                if self.mode().contains(TermMode::FOCUS_IN_OUT)
                    && let Some(tab) = self.active()
                {
                    tab.write(if focused { b"\x1b[I".to_vec() } else { b"\x1b[O".to_vec() });
                }
                self.redraw();
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if let Some(action) = binding(&event.logical_key, self.mods) {
                    self.run(action, event_loop);
                    return;
                }
                let text = if self.mods.control_key() { None } else { event.text.as_deref() };
                if let Some(bytes) = input::encode(&event.logical_key, text, self.mods, self.mode()) {
                    if let Some(tab) = self.active() {
                        tab.session.lock().term.selection = None;
                    }
                    self.write_active(bytes);
                    self.redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => self.cursor_moved(position),
            WindowEvent::MouseInput { state: ElementState::Pressed, button, .. } => {
                if self.layout().is_some_and(|layout| self.cursor.1 < layout.strip()) {
                    self.press_strip(button, event_loop);
                } else {
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
            event_loop.set_control_flow(ControlFlow::Wait);
            self.redraw();
        }
    }
}
