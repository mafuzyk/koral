use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::index::{Column, Direction, Point, Side};
use alacritty_terminal::selection::Selection;
use alacritty_terminal::term::{ClipboardType, TermMode};
use arboard::{Clipboard, GetExtLinux, LinuxClipboardKind, SetExtLinux};
use koral::blink::Blink;
use koral::blocks::Activity;
use koral::config::{self, Args, Config, Launch, USAGE};
use koral::diff::{self, DiffLine};
use koral::fonts;
use koral::frame::{self, Chrome, CursorView, Frame, Layout, PaneLayout, TabView};
use koral::input::{self, Action, Clicks, Mouse, resolve_binding};
use koral::ipc::{self, BlockSelector, Request};
use koral::keyboard::{self, KeyPress, Stage};
use koral::lag;
use koral::links::{self, Link};
use koral::matugen;
use koral::notify::{self, Decision, Notifier};
use koral::pty::{Geometry, Terminal, UserEvent};
use koral::render::{Renderer, rgba};
use koral::search::Search;
use koral::settings::{self, Command, FIELDS, Kind, Outcome, Settings, SettingsView, Zone};
use koral::splits::{Axis, CloseResult, Dir, SplitTree};
use koral::tabs::{self, Bar, Hit, Rect, Ring, Tabs};
use koral::theme::Theme;
use koral::view;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::platform::wayland::WindowAttributesExtWayland;
use winit::platform::x11::WindowAttributesExtX11;
use winit::window::{CursorIcon, UserAttentionType, Window, WindowId};

const CONFIRM_WINDOW: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Consent {
    Allow,
    Deny,
}

/// Deliberate permission gesture: exact modifiers, no auto-repeat, Esc separate.
fn consent_key(key: &Key, mods: ModifiersState, repeat: bool) -> Option<Consent> {
    if repeat {
        return None;
    }
    if matches!(key, Key::Named(NamedKey::Enter))
        && mods == (ModifiersState::CONTROL | ModifiersState::SHIFT)
    {
        return Some(Consent::Allow);
    }
    if matches!(key, Key::Named(NamedKey::Escape)) {
        return Some(Consent::Deny);
    }
    None
}

/// stderr for a launch from a terminal, the log for a launch from the desktop icon.
fn report(message: &str) {
    eprintln!("koral: {message}");
    config::log(message);
}

fn main() {
    lag::startup("main");
    let args = match config::parse_args(std::env::args().skip(1)) {
        Ok(Launch::Version) => {
            println!("koral {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Ok(Launch::TerminfoInstall) => match koral::terminfo::install() {
            Ok(dir) => {
                println!("installed {} to {}", koral::terminfo::NAME, dir.display());
                return;
            }
            Err(message) => {
                eprintln!("koral +terminfo install: {message}");
                std::process::exit(1);
            }
        },
        Ok(Launch::Ssh(ssh_args)) => match koral::ssh::run(&ssh_args) {
            Ok(code) => std::process::exit(code),
            Err(message) => {
                eprintln!("koral +ssh: {message}");
                std::process::exit(2);
            }
        },
        Ok(Launch::Help) => {
            print!("{USAGE}");
            return;
        }
        Ok(Launch::Run(args)) => args,
        Ok(Launch::Control(ctl)) => {
            let wants_json = ctl.iter().any(|arg| arg == "--json");
            let result = koral::ipc::parse_cli(&ctl).and_then(|(socket, req)| {
                if matches!(req, Request::Watch { .. }) {
                    koral::ipc::watch(socket, &req, |line| println!("{line}"))?;
                    Ok(String::new())
                } else {
                    koral::ipc::send(socket, &req)
                }
            });
            match result {
                Ok(out) => {
                    if !out.is_empty() {
                        println!("{out}");
                    }
                    return;
                }
                Err(message) => {
                    if wants_json {
                        println!("{}", koral::ipc::json_error(&message));
                    } else {
                        eprintln!("koral ctl: {message}");
                    }
                    std::process::exit(1);
                }
            }
        }
        Ok(Launch::BlockOutput {
            selector,
            pane,
            json,
        }) => {
            let result = ipc::parse_block_selector(&selector).and_then(|sel| {
                ipc::send(
                    None,
                    &Request::GetBlockOutput {
                        pane,
                        selector: sel,
                        json,
                    },
                )
            });
            match result {
                Ok(out) => {
                    if !out.is_empty() {
                        println!("{out}");
                    }
                    return;
                }
                Err(message) => {
                    if json {
                        println!("{}", ipc::json_error(&message));
                    } else {
                        eprintln!("koral: {message}");
                    }
                    std::process::exit(1);
                }
            }
        }
        Ok(Launch::DiffBlocks { left, right, pane }) => {
            let result = (|| -> Result<String, String> {
                let left_sel = ipc::parse_block_selector(&left)?;
                let right_sel = ipc::parse_block_selector(&right)?;
                let left_out = ipc::send(
                    None,
                    &Request::GetBlockOutput {
                        pane,
                        selector: left_sel,
                        json: false,
                    },
                )?;
                let right_out = ipc::send(
                    None,
                    &Request::GetBlockOutput {
                        pane,
                        selector: right_sel,
                        json: false,
                    },
                )?;
                let hunks = diff::diff_lines(&left_out, &right_out, 3);
                Ok(diff::format_unified(
                    &hunks,
                    &left,
                    &right,
                    std::io::stdout().is_terminal(),
                ))
            })();
            match result {
                Ok(text) => {
                    if text.is_empty() {
                        println!("no differences");
                    } else {
                        print!("{text}");
                    }
                    return;
                }
                Err(message) => {
                    eprintln!("koral diff-blocks: {message}");
                    std::process::exit(1);
                }
            }
        }
        Err(message) => {
            eprintln!("koral: {message}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    let event_loop = match EventLoop::<UserEvent>::with_user_event().build() {
        Ok(event_loop) => event_loop,
        Err(error) => {
            report(&format!("cannot start: {error}"));
            std::process::exit(1);
        }
    };
    lag::startup("event_loop");
    let proxy = event_loop.create_proxy();
    let mut app = App::new(proxy, args);
    lag::startup("app_new");
    if let Err(error) = event_loop.run_app(&mut app) {
        report(&format!("cannot run: {error}"));
        std::process::exit(1);
    }
}

fn default_shell() -> String {
    let on_path = std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join("sharky").is_file()));
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

#[derive(Clone, PartialEq)]
enum Drag {
    None,
    Selecting,
    Tab(usize),
    Divider(Vec<bool>, Axis, Rect),
}

enum Tab {
    Terminal(SplitTree<Terminal>),
    Settings(Settings),
}

impl Tab {
    fn as_tree(&self) -> Option<&SplitTree<Terminal>> {
        match self {
            Self::Terminal(tree) => Some(tree),
            Self::Settings(_) => None,
        }
    }

    fn as_tree_mut(&mut self) -> Option<&mut SplitTree<Terminal>> {
        match self {
            Self::Terminal(tree) => Some(tree),
            Self::Settings(_) => None,
        }
    }

    fn active_terminal(&self) -> Option<&Terminal> {
        self.as_tree().map(SplitTree::active)
    }

    fn active_terminal_mut(&mut self) -> Option<&mut Terminal> {
        self.as_tree_mut().map(SplitTree::active_mut)
    }

    fn is_settings(&self) -> bool {
        matches!(self, Self::Settings(_))
    }
}

struct DiffOverlay {
    title: String,
    lines: Vec<DiffLine>,
    scroll: usize,
}

struct App {
    proxy: EventLoopProxy<UserEvent>,
    args: Args,
    window: Option<Arc<Window>>,
    refresh_millihertz: Option<u32>,
    term_name: Option<&'static str>,
    renderer: Option<Renderer>,
    ipc: Option<ipc::Server>,
    tabs: Option<Tabs<Tab>>,
    next_id: u64,
    config: Config,
    source: String,
    settings_error: Option<String>,
    fonts: Vec<String>,
    theme: Theme,
    home: Option<String>,
    clipboard: Option<Option<Clipboard>>,
    pending_clipboard_kind: LinuxClipboardKind,
    pending_clipboard_image: Option<(LinuxClipboardKind, usize, usize, Vec<u8>)>,
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
    notices: Notifier,
    search: Option<Search>,
    diff_overlay: Option<DiffOverlay>,
    link: Option<(u64, Link)>,
    frame: Option<Frame>,
    a11y: Option<accesskit_winit::Adapter>,
}

impl App {
    fn new(proxy: EventLoopProxy<UserEvent>, args: Args) -> Self {
        if let Some(Err(error)) = config::home().map(|home| config::migrate(&home.join("koral"))) {
            report(&format!("cannot migrate config: {error}"));
        }
        let source = config::path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .unwrap_or_default();
        let (config, errors) = config::parse(&source);
        for error in errors {
            report(&format!("koral.conf {error}"));
        }
        let notices = Notifier::with_policies(config.clipboard_write, config.notifications);
        matugen::watch(proxy.clone());
        Self {
            proxy,
            args,
            window: None,
            refresh_millihertz: None,
            term_name: None,
            renderer: None,
            ipc: None,
            tabs: None,
            next_id: 0,
            theme: matugen::resolve_theme(config.matugen, config.theme.as_deref()),
            config,
            source,
            settings_error: None,
            fonts: Vec::new(),
            home: std::env::var("HOME").ok(),
            clipboard: None,
            pending_clipboard_kind: LinuxClipboardKind::Clipboard,
            pending_clipboard_image: None,
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
            notices,
            search: None,
            diff_overlay: None,
            link: None,
            frame: None,
            a11y: None,
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

    fn divider_px(layout: Layout) -> f32 {
        (2.0 * layout.scale).round().max(1.0)
    }

    fn geometry_for(rect: Rect, layout: Layout) -> Geometry {
        let pane = PaneLayout::from_rect(rect, layout);
        Geometry {
            columns: pane.columns(),
            lines: pane.lines(),
            cell_width: layout.cell.width as u16,
            cell_height: layout.cell.height as u16,
        }
    }

    fn active_pane(&self) -> Option<(u64, Rect, PaneLayout)> {
        let layout = self.layout()?;
        let tree = self.tabs.as_ref()?.active()?.as_tree()?;
        let (panes, _) = tree.layout(layout.content_rect(), Self::divider_px(layout));
        let (id, rect) = panes.into_iter().find(|(id, _)| *id == tree.active_id())?;
        Some((id, rect, PaneLayout::from_rect(rect, layout)))
    }

    fn bar(&self) -> Option<Bar> {
        let layout = self.layout()?;
        Some(tabs::layout(
            self.tabs.as_ref()?.len(),
            layout.width,
            layout.strip(),
        ))
    }

    fn redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn sync_refresh_rate(&mut self) {
        let refresh = self
            .window
            .as_ref()
            .and_then(|window| window.current_monitor())
            .and_then(|monitor| monitor.refresh_rate_millihertz());
        if refresh == self.refresh_millihertz {
            return;
        }
        self.refresh_millihertz = refresh;
        if let Some(tabs) = &self.tabs {
            for tree in tabs.iter().filter_map(Tab::as_tree) {
                for (_, terminal) in tree.iter() {
                    terminal.set_refresh_millihertz(refresh);
                }
            }
        }
    }

    fn active(&self) -> Option<&Terminal> {
        self.tabs.as_ref()?.active()?.active_terminal()
    }

    fn active_mut(&mut self) -> Option<&mut Terminal> {
        self.tabs.as_mut()?.active_mut()?.active_terminal_mut()
    }

    fn active_settings_mut(&mut self) -> Option<&mut Settings> {
        match self.tabs.as_mut()?.active_mut()? {
            Tab::Settings(settings) => Some(settings),
            Tab::Terminal(_) => None,
        }
    }

    fn active_is_settings(&self) -> bool {
        self.tabs
            .as_ref()
            .and_then(Tabs::active)
            .is_some_and(Tab::is_settings)
    }

    fn terminal(&self, id: u64) -> Option<&Terminal> {
        self.tabs
            .as_ref()?
            .iter()
            .find_map(|tab| tab.as_tree()?.get(id))
    }

    fn terminal_mut(&mut self, id: u64) -> Option<&mut Terminal> {
        self.tabs
            .as_mut()?
            .iter_mut()
            .find_map(|tab| tab.as_tree_mut()?.get_mut(id))
    }

    fn term_name(&mut self) -> &'static str {
        self.term_name
            .get_or_insert_with(|| koral::terminfo::resolve(&koral::terminfo::search_dirs()))
    }

    fn spawn_terminal(
        &mut self,
        geometry: Geometry,
        cwd: Option<PathBuf>,
        title: Option<String>,
        command: Option<(String, Vec<String>)>,
    ) -> Option<Terminal> {
        let id = self.next_id;
        self.next_id += 1;
        let scrollback = self.config.scrollback;
        let socket = self.ipc.as_ref().map(ipc::Server::path).map(PathBuf::from);
        let term_name = self.term_name();
        let mut terminal = if let Some((prog, cmd_args)) = command {
            let env = vec![("TERM".to_string(), term_name.to_string())];
            match Terminal::spawn_configured(
                id,
                self.proxy.clone(),
                &prog,
                &cmd_args,
                geometry,
                cwd,
                scrollback,
                socket.as_deref(),
                &env,
            ) {
                Ok(terminal) => Some(terminal),
                Err(error) => {
                    report(&format!("cannot start {prog}: {error}"));
                    None
                }
            }
        } else {
            let fallback = default_shell();
            let preferred = std::env::var("KORAL_SHELL")
                .ok()
                .or_else(|| self.config.shell.clone())
                .unwrap_or_else(|| fallback.clone());
            let mut shells = vec![preferred];
            if shells[0] != fallback {
                shells.push(fallback);
            }
            let integration_dir = if self.config.shell_integration {
                let dir = koral::shell::dir();
                match koral::shell::prepare(&dir) {
                    Ok(()) => Some(dir),
                    Err(error) => {
                        report(&format!("cannot prepare shell integration: {error}"));
                        None
                    }
                }
            } else {
                None
            };
            // A bad `shell` setting must not cost the user the window: the default gets a turn.
            shells.iter().find_map(|shell| {
                let mut injection = integration_dir
                    .as_deref()
                    .map(|dir| koral::shell::plan(shell, &|key| std::env::var(key).ok(), dir))
                    .unwrap_or_default();
                injection
                    .env
                    .push(("TERM".to_string(), term_name.to_string()));
                match Terminal::spawn_configured(
                    id,
                    self.proxy.clone(),
                    shell,
                    &injection.args,
                    geometry,
                    cwd.clone(),
                    scrollback,
                    socket.as_deref(),
                    &injection.env,
                ) {
                    Ok(terminal) => Some(terminal),
                    Err(error) => {
                        report(&format!("cannot start {shell}: {error}"));
                        None
                    }
                }
            })
        };
        if let Some(term) = &mut terminal {
            term.set_refresh_millihertz(self.refresh_millihertz);
            if title.is_some() {
                term.title = title;
            }
        }
        terminal
    }

    fn active_cwd(&self) -> Option<PathBuf> {
        self.active().and_then(|tab| {
            let session = tab.session.lock();
            session.blocks.latest()?.cwd.clone().map(PathBuf::from)
        })
    }

    fn open_tab(&mut self) {
        let initial = self.next_id == 0;
        let cwd = if initial { self.args.cwd.take() } else { None };
        let title = if initial {
            self.args.title.clone()
        } else {
            None
        };
        let command = if initial {
            self.args.command.take()
        } else {
            None
        };
        self.open_tab_with(cwd, title, command);
    }

    fn open_tab_with(
        &mut self,
        cwd: Option<PathBuf>,
        title: Option<String>,
        command: Option<(String, Vec<String>)>,
    ) {
        let Some(layout) = self.layout() else {
            return;
        };
        let geometry = Self::geometry_for(layout.content_rect(), layout);
        let cwd = cwd.or_else(|| self.active_cwd());
        let Some(terminal) = self.spawn_terminal(geometry, cwd, title, command) else {
            return;
        };
        let tab = Tab::Terminal(SplitTree::new(terminal.id, terminal));
        match &mut self.tabs {
            Some(tabs) => tabs.push(tab),
            None => self.tabs = Some(Tabs::new(tab)),
        }
        self.resize_all();
        self.redraw();
    }

    fn split_active(&mut self, axis: Axis, cwd: Option<PathBuf>) -> bool {
        let Some(layout) = self.layout() else {
            return false;
        };
        if !self
            .tabs
            .as_ref()
            .and_then(Tabs::active)
            .is_some_and(|t| t.as_tree().is_some())
        {
            return false;
        }
        let cwd = cwd.or_else(|| self.active_cwd());
        let geometry = Self::geometry_for(layout.content_rect(), layout);
        let Some(terminal) = self.spawn_terminal(geometry, cwd, None, None) else {
            return false;
        };
        let new_id = terminal.id;
        if let Some(tree) = self
            .tabs
            .as_mut()
            .and_then(Tabs::active_mut)
            .and_then(Tab::as_tree_mut)
        {
            tree.split_active(axis, new_id, terminal);
        }
        self.search = None;
        self.link = None;
        self.resize_all();
        self.redraw();
        true
    }

    fn close_active_pane(&mut self, event_loop: &ActiveEventLoop) {
        let Some(active_idx) = self.tabs.as_ref().map(Tabs::active_index) else {
            return;
        };
        let is_terminal = self
            .tabs
            .as_ref()
            .and_then(Tabs::active)
            .is_some_and(|t| t.as_tree().is_some());
        if !is_terminal {
            self.request_close(active_idx, event_loop);
            return;
        }
        let result = if let Some(tree) = self
            .tabs
            .as_mut()
            .and_then(Tabs::active_mut)
            .and_then(Tab::as_tree_mut)
        {
            let id = tree.active_id();
            tree.close(id)
        } else {
            CloseResult::NotFound
        };
        match result {
            CloseResult::LastPane => self.request_close(active_idx, event_loop),
            CloseResult::Closed => {
                self.search = None;
                self.link = None;
                self.resize_all();
                self.redraw();
            }
            CloseResult::NotFound => {}
        }
    }

    fn focus_dir(&mut self, dir: Dir) {
        let Some(layout) = self.layout() else {
            return;
        };
        let div = Self::divider_px(layout);
        if let Some(tree) = self
            .tabs
            .as_mut()
            .and_then(Tabs::active_mut)
            .and_then(Tab::as_tree_mut)
            && tree.focus_dir(dir, layout.content_rect(), div)
        {
            self.search = None;
            self.redraw();
        }
    }

    fn resize_dir(&mut self, dir: Dir) {
        if let Some(tree) = self
            .tabs
            .as_mut()
            .and_then(Tabs::active_mut)
            .and_then(Tab::as_tree_mut)
            && tree.resize_active(dir, 0.05)
        {
            self.resize_all();
            self.redraw();
        }
    }

    /// A running command gets a second chance: the tab asks instead of a dialog.
    fn request_close(&mut self, index: usize, event_loop: &ActiveEventLoop) {
        let Some(tab) = self.tabs.as_ref().and_then(|tabs| tabs.iter().nth(index)) else {
            return;
        };
        let Tab::Terminal(tree) = tab else {
            self.confirming = None;
            self.close_tab(index, event_loop);
            return;
        };
        let id = tree.active_id();
        let running = tree
            .iter()
            .into_iter()
            .any(|(_, term)| term.session.lock().blocks.activity() == Activity::Running);
        let confirmed = self
            .confirming
            .is_some_and(|(pending, at)| pending == id && at.elapsed() < CONFIRM_WINDOW);
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
        self.search = None;
        self.link = None;
        self.resize_all();
        self.redraw();
    }

    fn write_active(&mut self, bytes: Vec<u8>) {
        if let Some(tab) = self.active_mut() {
            tab.scroll = 0;
            tab.write(bytes);
        }
    }

    fn mode(&self) -> TermMode {
        self.active()
            .map_or(TermMode::empty(), |tab| *tab.session.lock().term.mode())
    }

    fn ime(&mut self, ime: Ime) {
        let in_settings = self.active_is_settings();
        match ime {
            Ime::Preedit(text, _) => self.preedit = Some(text).filter(|text| !text.is_empty()),
            Ime::Commit(text) if in_settings => {
                self.preedit = None;
                text.chars()
                    .for_each(|ch| self.settings_command(Command::Type(ch), None));
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
        let press = KeyPress {
            key: &event.logical_key,
            unshifted: &unshifted,
            text,
            mods: self.mods,
            stage,
        };
        keyboard::encode(&press, self.mode())
    }

    fn run(&mut self, action: Action, event_loop: &ActiveEventLoop) {
        match action {
            Action::NewTab => self.open_tab(),
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
            Action::JumpLastError => self.jump_to_last_error(),
            Action::CopyOutput => self.copy_output(),
            Action::ToggleFold => self.toggle_fold(),
            Action::DiffBlocks => self.toggle_diff_overlay(),
            Action::Search => {
                self.search.get_or_insert_with(Search::default);
            }
            Action::SearchBlock => self.open_scoped_search(),
            Action::PageUp => self.scroll_lines(self.page()),
            Action::PageDown => self.scroll_lines(-self.page()),
            Action::OpenSettings => {
                self.search = None;
                self.toggle_settings(event_loop);
            }
            Action::ZoomIn => self.zoom_to(self.zoom + 1.0),
            Action::ZoomOut => self.zoom_to(self.zoom - 1.0),
            Action::ZoomReset => self.zoom_to(0.0),
            Action::SplitRight => {
                self.split_active(Axis::Horizontal, None);
            }
            Action::SplitDown => {
                self.split_active(Axis::Vertical, None);
            }
            Action::ClosePane => self.close_active_pane(event_loop),
            Action::FocusLeft => self.focus_dir(Dir::Left),
            Action::FocusRight => self.focus_dir(Dir::Right),
            Action::FocusUp => self.focus_dir(Dir::Up),
            Action::FocusDown => self.focus_dir(Dir::Down),
            Action::ResizeLeft => self.resize_dir(Dir::Left),
            Action::ResizeRight => self.resize_dir(Dir::Right),
            Action::ResizeUp => self.resize_dir(Dir::Up),
            Action::ResizeDown => self.resize_dir(Dir::Down),
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

    fn toggle_settings(&mut self, event_loop: &ActiveEventLoop) {
        let existing = self
            .tabs
            .as_ref()
            .and_then(|tabs| tabs.iter().position(Tab::is_settings));
        match existing {
            Some(index) => {
                let active = self.tabs.as_ref().map(Tabs::active_index) == Some(index);
                let len = self.tabs.as_ref().map_or(0, Tabs::len);
                if active && len > 1 {
                    self.close_tab(index, event_loop);
                } else if !active {
                    self.with_tabs(|tabs| tabs.select(index));
                }
            }
            None => {
                let tab = Tab::Settings(Settings::new(self.fonts.clone()));
                match &mut self.tabs {
                    Some(tabs) => tabs.push(tab),
                    None => self.tabs = Some(Tabs::new(tab)),
                }
            }
        }
        self.settings_error = None;
        self.redraw();
    }

    fn open_or_focus_settings(&mut self) {
        let existing = self
            .tabs
            .as_ref()
            .and_then(|tabs| tabs.iter().position(Tab::is_settings));
        if let Some(index) = existing {
            self.with_tabs(|tabs| tabs.select(index));
        } else {
            let tab = Tab::Settings(Settings::new(self.fonts.clone()));
            match &mut self.tabs {
                Some(tabs) => tabs.push(tab),
                None => self.tabs = Some(Tabs::new(tab)),
            }
        }
        self.settings_error = None;
        self.redraw();
    }

    fn settings_command(&mut self, command: Command, event_loop: Option<&ActiveEventLoop>) {
        let visible_rows = self
            .layout()
            .map(|l| {
                SettingsView::new(l.width, l.height, l.strip(), l.scale, l.cell.height).visible_rows
            })
            .unwrap_or(15);
        let config_snapshot = self.config.clone();
        let outcome = {
            let Some(settings) = self.active_settings_mut() else {
                return;
            };
            let outcome = settings.handle(command, &config_snapshot);
            settings.keep_selected_visible(visible_rows);
            outcome
        };
        self.settings_error = None;
        match outcome {
            Outcome::Nothing => {}
            Outcome::Close => {
                if let (Some(idx), Some(el)) =
                    (self.tabs.as_ref().map(Tabs::active_index), event_loop)
                    && self.tabs.as_ref().is_some_and(|t| t.len() > 1)
                {
                    self.close_tab(idx, el);
                }
            }
            Outcome::Change(key, value) => self.change_setting(key, &value),
        }
        self.redraw();
    }

    fn apply_setting_update(&mut self, key: &str, value: &str) -> Result<(), String> {
        if key == "matugen" && value == "on" {
            matugen::install()?;
        }
        // An editor may have touched the file while koral runs: start from what is on disk now.
        let mut current = config::path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .unwrap_or_else(|| self.source.clone());
        if key == "theme"
            && self.config.matugen
            && let Ok((updated, _)) = config::update(&current, "matugen", "off")
        {
            current = updated;
        }
        let (source, config) = config::update(&current, key, value)?;
        let font_changed =
            config.face() != self.config.face() || config.font_size != self.config.font_size;
        self.source = source;
        self.notices
            .set_policies(config.clipboard_write, config.notifications);
        if self.config.window_decorations != config.window_decorations
            && let Some(window) = &self.window
        {
            window.set_decorations(config.window_decorations);
        }
        self.config = config;
        self.theme = matugen::resolve_theme(self.config.matugen, self.config.theme.as_deref());

        if font_changed && let Some(renderer) = &mut self.renderer {
            renderer.set_font(
                &self.config.face(),
                input::zoomed(self.config.font_size, self.zoom),
            );
        }
        self.resize_all();

        match config::path().map(|path| config::save(&path, &self.source)) {
            Some(Ok(())) => Ok(()),
            Some(Err(error)) => Err(format!("Not saved: {error}")),
            None => Err("Not saved: no config folder".to_string()),
        }
    }

    /// Live first, then saved: the file stays the source of truth for the next start.
    fn change_setting(&mut self, key: &str, value: &str) {
        if let Err(message) = self.apply_setting_update(key, value) {
            self.settings_error = Some(message);
        }
    }

    fn reload_config(&mut self) {
        let Some(source) = config::path().and_then(|path| std::fs::read_to_string(path).ok())
        else {
            return;
        };
        if source == self.source {
            return;
        }
        let (config, errors) = config::parse(&source);
        for error in errors {
            report(&format!("koral.conf {error}"));
        }
        let font_changed =
            config.face() != self.config.face() || config.font_size != self.config.font_size;
        self.source = source;
        self.notices
            .set_policies(config.clipboard_write, config.notifications);
        if self.config.window_decorations != config.window_decorations
            && let Some(window) = &self.window
        {
            window.set_decorations(config.window_decorations);
        }
        self.config = config;
        self.theme = matugen::resolve_theme(self.config.matugen, self.config.theme.as_deref());
        if font_changed && let Some(renderer) = &mut self.renderer {
            renderer.set_font(
                &self.config.face(),
                input::zoomed(self.config.font_size, self.zoom),
            );
        }
        self.resize_all();
        self.redraw();
    }

    fn search_key(&mut self, key: &Key, text: Option<&str>) {
        let direction = if self.mods.shift_key() {
            Direction::Right
        } else {
            Direction::Left
        };
        match key {
            Key::Named(NamedKey::Escape) => self.search = None,
            Key::Named(NamedKey::Enter) => self.search_step(direction),
            Key::Named(NamedKey::ArrowUp) => self.search_step(Direction::Left),
            Key::Named(NamedKey::ArrowDown) => self.search_step(Direction::Right),
            Key::Named(NamedKey::Backspace) => self.search_edit(|query| {
                query.pop();
            }),
            _ if self.mods == ModifiersState::ALT
                && text.is_some_and(|t| t.eq_ignore_ascii_case("b")) =>
            {
                let next_scope = if self.search.as_ref().is_some_and(|s| s.scope.is_some()) {
                    None
                } else {
                    self.active_block_scope()
                };
                if let Some(search) = &mut self.search {
                    search.set_scope(next_scope);
                }
                self.search_step(Direction::Left);
            }
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
        let Some((_, _, pane)) = self.active_pane() else {
            return;
        };
        let lines = pane.lines();
        let Some(mut search) = self.search.take() else {
            return;
        };
        if let Some(tab) = self.active_mut() {
            let mut session = tab.session.lock();
            if let Some(found) = search.step(&session.term, direction) {
                let line = found.start().line;
                let rows = tab.projected_rows(&session);
                if let Some(id) = view::block_of(&rows, line) {
                    session.blocks.toggle_fold(id);
                }
                let rows = tab.projected_rows(&session);
                if let Some(index) = view::row_of(&rows, line) {
                    tab.scroll = rows
                        .len()
                        .saturating_sub(lines + index.saturating_sub(lines / 2));
                }
            }
        }
        self.search = Some(search);
        self.redraw();
    }

    fn settings_key(&mut self, key: &Key, text: Option<&str>, event_loop: &ActiveEventLoop) {
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
            self.settings_command(command, Some(event_loop));
        } else if let Some(text) = text {
            for ch in text.chars().filter(|ch| !ch.is_control()) {
                self.settings_command(Command::Type(ch), Some(event_loop));
            }
        }
    }

    /// True when the active tab is Settings and consumed the click.
    fn press_settings(&mut self, event_loop: &ActiveEventLoop) -> bool {
        let Some(layout) = self.layout() else {
            return false;
        };
        let view = SettingsView::new(
            layout.width,
            layout.height,
            layout.strip(),
            layout.scale,
            layout.cell.height,
        );
        let (x, y) = self.cursor;
        let zone = {
            let Some(Tab::Settings(open)) = self.tabs.as_ref().and_then(Tabs::active) else {
                return false;
            };
            settings::tab_zone(view, x, y, open)
        };
        let picking = self
            .active_settings_mut()
            .is_some_and(|open| open.picker.is_some());
        if picking {
            if let Zone::Row(row) = zone
                && let Some(open) = self.active_settings_mut()
                && open.pick(row, view.picker_rows())
            {
                self.settings_command(Command::Confirm, Some(event_loop));
            }
            self.redraw();
            return true;
        }
        match zone {
            Zone::Outside | Zone::Inside => {}
            Zone::Row(row) => {
                let confirm = {
                    let Some(open) = self.active_settings_mut() else {
                        return true;
                    };
                    let chosen = open.selected == row;
                    if !chosen {
                        open.editing = None;
                        open.selected = row;
                    }
                    chosen
                        || FIELDS[row].kind == Kind::Toggle
                        || matches!(FIELDS[row].kind, Kind::Choice(_))
                };
                if confirm {
                    self.settings_command(Command::Confirm, Some(event_loop));
                }
            }
            Zone::Less(row) => {
                if let Some(open) = self.active_settings_mut() {
                    open.editing = None;
                    open.selected = row;
                }
                self.settings_command(Command::Less, Some(event_loop));
            }
            Zone::More(row) => {
                if let Some(open) = self.active_settings_mut() {
                    open.editing = None;
                    open.selected = row;
                }
                self.settings_command(Command::More, Some(event_loop));
            }
        }
        self.redraw();
        true
    }

    /// True when a permission prompt consumed the key.
    fn permission_key(&mut self, event: &KeyEvent) -> bool {
        let Some(consent) = consent_key(&event.logical_key, self.mods, event.repeat) else {
            return false;
        };
        if !self.focused || self.notices.take_confirmation().is_none() {
            return false;
        }
        match consent {
            Consent::Allow => {
                if self.notices.confirm(Instant::now()) == Some(Decision::Performed) {
                    if let Some(text) = self.notices.take_ready_text() {
                        if let Some((kind, width, height, bytes)) =
                            self.pending_clipboard_image.take()
                            && text == "[image/png]"
                        {
                            self.write_clipboard_image(kind, width, height, bytes);
                        } else {
                            let kind = self.pending_clipboard_kind;
                            self.pending_clipboard_kind = LinuxClipboardKind::Clipboard;
                            if let Some(clipboard) = self.clipboard() {
                                let _ = clipboard.set().clipboard(kind).text(text);
                            }
                        }
                    }
                    if let Some(notice) = self.notices.take_ready_notice(Instant::now()) {
                        notify::show(&notice);
                    }
                }
            }
            Consent::Deny => {
                self.notices.cancel();
                self.pending_clipboard_image = None;
                self.pending_clipboard_kind = LinuxClipboardKind::Clipboard;
            }
        }
        self.redraw();
        true
    }

    fn bell(&mut self, id: u64) {
        let active = self.active().is_some_and(|tab| tab.id == id);
        let Ring { mark, urgent } = tabs::ring(active, self.focused);
        if let Some(tab) = self.terminal_mut(id) {
            tab.rang |= mark;
        }
        if let (true, Some(window)) = (urgent, &self.window) {
            window.request_user_attention(Some(UserAttentionType::Informational));
        }
        self.redraw();
    }

    fn with_tabs(&mut self, change: impl FnOnce(&mut Tabs<Tab>)) {
        self.search = None;
        self.link = None;
        if let Some(tabs) = &mut self.tabs {
            change(tabs);
        }
        self.resize_all();
    }

    fn page(&self) -> i64 {
        self.active_pane()
            .map_or(1, |(_, _, pane)| pane.lines() as i64 - 1)
    }

    fn jump(&mut self, target: fn(&[view::Row], usize) -> Option<usize>) {
        let Some((_, _, pane)) = self.active_pane() else {
            return;
        };
        let lines = pane.lines();
        let Some(tab) = self.active_mut() else {
            return;
        };
        let rows = {
            let session = tab.session.lock();
            tab.projected_rows(&session)
        };
        let top = frame::visible_top(&rows, lines, tab.scroll);
        if let Some(index) = target(&rows, top) {
            tab.scroll = rows.len().saturating_sub(lines + index);
        }
    }

    fn active_block_scope(
        &self,
    ) -> Option<(
        alacritty_terminal::index::Line,
        alacritty_terminal::index::Line,
    )> {
        let (_, _, pane) = self.active_pane()?;
        let lines = pane.lines();
        let tab = self.active()?;
        let session = tab.session.lock();
        let rows = tab.projected_rows(&session);
        let end = frame::visible_top(&rows, lines, tab.scroll) + lines;
        let id = view::fold_target(&rows, end, &session.blocks)
            .or_else(|| session.blocks.latest().map(|b| b.id))?;
        view::block_line_range(&session.term, &session.blocks, id)
    }

    fn open_scoped_search(&mut self) {
        let scope = self.active_block_scope();
        let search = self.search.get_or_insert_with(Search::default);
        search.set_scope(scope);
    }

    fn jump_to_last_error(&mut self) {
        let Some((_, _, pane)) = self.active_pane() else {
            return;
        };
        let lines = pane.lines();
        let Some(tab) = self.active_mut() else {
            return;
        };
        let target_idx = {
            let session = tab.session.lock();
            let Some(err_id) = session.blocks.last_failed_id() else {
                return;
            };
            let rows = tab.projected_rows(&session);
            view::block_row(&rows, err_id).map(|idx| (rows.len(), idx))
        };
        if let Some((total, index)) = target_idx {
            tab.scroll = total.saturating_sub(lines + index.saturating_sub(lines / 4));
        }
    }

    fn copy_output(&mut self) {
        let Some(tab) = self.active() else {
            return;
        };
        let text = {
            let session = tab.session.lock();
            let rows = tab.projected_rows(&session);
            view::fold_target(&rows, rows.len(), &session.blocks)
                .and_then(|id| view::output_text(&session.term, &session.blocks, id))
        };
        if let Some(text) = text {
            self.write_clipboard(text, LinuxClipboardKind::Clipboard);
        }
    }

    fn toggle_fold(&mut self) {
        let Some((_, _, pane)) = self.active_pane() else {
            return;
        };
        let lines = pane.lines();
        let Some(tab) = self.active() else {
            return;
        };
        let scroll = tab.scroll;
        let mut session = tab.session.lock();
        let rows = tab.projected_rows(&session);
        let end = frame::visible_top(&rows, lines, scroll) + lines;
        if let Some(id) = view::fold_target(&rows, end, &session.blocks) {
            session.blocks.toggle_fold(id);
        }
    }

    fn toggle_diff_overlay(&mut self) {
        if self.diff_overlay.is_some() {
            self.diff_overlay = None;
            return;
        }
        let overlay = {
            let Some(tab) = self.active() else {
                return;
            };
            let session = tab.session.lock();
            let completed: Vec<_> = session
                .blocks
                .iter()
                .filter(|b| b.status.is_some())
                .collect();
            let pair = completed.last().copied().and_then(|right| {
                let left = completed[..completed.len().saturating_sub(1)]
                    .iter()
                    .rev()
                    .find(|b| right.command.is_some() && b.command == right.command)
                    .or_else(|| completed.get(completed.len().saturating_sub(2)))
                    .copied()?;
                Some((left, right))
            });
            match pair {
                None => DiffOverlay {
                    title: "Diff blocks (need 2 completed blocks)".to_string(),
                    lines: Vec::new(),
                    scroll: 0,
                },
                Some((left, right)) => {
                    let left_text = view::output_text(&session.term, &session.blocks, left.id)
                        .unwrap_or_default();
                    let right_text = view::output_text(&session.term, &session.blocks, right.id)
                        .unwrap_or_default();
                    let hunks = diff::diff_lines(&left_text, &right_text, 3);
                    let lines: Vec<DiffLine> = hunks.into_iter().flat_map(|h| h.lines).collect();
                    let cmd = right
                        .command
                        .as_deref()
                        .and_then(|c| c.lines().next())
                        .unwrap_or("block");
                    DiffOverlay {
                        title: format!("Diff: {cmd} (#{} vs #{})", left.id, right.id),
                        lines,
                        scroll: 0,
                    }
                }
            }
        };
        self.diff_overlay = Some(overlay);
    }

    fn diff_overlay_key(&mut self, key: &Key, text: Option<&str>) {
        let page = self.page().max(1) as usize;
        let Some(overlay) = &mut self.diff_overlay else {
            return;
        };
        match key {
            Key::Named(NamedKey::Escape) => {
                self.diff_overlay = None;
            }
            Key::Character(_) if text.is_some_and(|t| t.eq_ignore_ascii_case("q")) => {
                self.diff_overlay = None;
            }
            Key::Named(NamedKey::ArrowUp) => {
                overlay.scroll = overlay.scroll.saturating_sub(1);
            }
            Key::Named(NamedKey::ArrowDown) => {
                overlay.scroll = (overlay.scroll + 1).min(overlay.lines.len().saturating_sub(1));
            }
            Key::Character(_) if text == Some("k") => {
                overlay.scroll = overlay.scroll.saturating_sub(1);
            }
            Key::Character(_) if text == Some("j") => {
                overlay.scroll = (overlay.scroll + 1).min(overlay.lines.len().saturating_sub(1));
            }
            Key::Named(NamedKey::PageUp) => {
                overlay.scroll = overlay.scroll.saturating_sub(page);
            }
            Key::Named(NamedKey::PageDown) => {
                overlay.scroll = (overlay.scroll + page).min(overlay.lines.len().saturating_sub(1));
            }
            _ => {}
        }
        self.redraw();
    }

    fn clipboard(&mut self) -> Option<&mut Clipboard> {
        self.clipboard
            .get_or_insert_with(|| Clipboard::new().ok())
            .as_mut()
    }

    /// Explicit user copies are trusted: they never consult permission policy.
    fn write_clipboard(&mut self, text: String, kind: LinuxClipboardKind) {
        let Some(clipboard) = self.clipboard() else {
            return;
        };
        let _ = clipboard.set().clipboard(kind).text(text);
    }

    fn write_clipboard_image(
        &mut self,
        kind: LinuxClipboardKind,
        width: usize,
        height: usize,
        bytes: Vec<u8>,
    ) {
        if let Some(clipboard) = self.clipboard() {
            let _ = clipboard.set().clipboard(kind).image(arboard::ImageData {
                width,
                height,
                bytes: std::borrow::Cow::Owned(bytes),
            });
        }
    }

    /// Program-driven copies (OSC 52 / OSC 5522) obey the configured policy.
    fn store(&mut self, text: String, kind: LinuxClipboardKind) {
        if self.clipboard().is_none() {
            return;
        }
        let text = match self.notices.clipboard_write(text, Instant::now()) {
            Decision::Performed => match self.notices.take_ready_text() {
                Some(text) => text,
                None => return,
            },
            Decision::Confirm => {
                self.pending_clipboard_kind = kind;
                self.pending_clipboard_image = None;
                self.redraw();
                return;
            }
            Decision::Denied | Decision::Held => return,
        };
        if let Some(clipboard) = self.clipboard() {
            let _ = clipboard.set().clipboard(kind).text(text);
        }
    }

    fn store_image(&mut self, png_bytes: &[u8], kind: LinuxClipboardKind) {
        if self.clipboard().is_none() {
            return;
        }
        let Ok((width, height, rgba)) = koral::graphics::decode_png_rgba(png_bytes) else {
            return;
        };
        match self
            .notices
            .clipboard_write("[image/png]".to_string(), Instant::now())
        {
            Decision::Performed => {
                let _ = self.notices.take_ready_text();
                self.write_clipboard_image(kind, width as usize, height as usize, rgba);
            }
            Decision::Confirm => {
                self.pending_clipboard_kind = kind;
                self.pending_clipboard_image = Some((kind, width as usize, height as usize, rgba));
                self.redraw();
            }
            Decision::Denied | Decision::Held => {}
        }
    }

    fn handle_osc5522(&mut self, id: u64, op: koral::marks::ClipboardOp) {
        use base64::Engine;
        use base64::engine::general_purpose::STANDARD;
        use koral::marks::{ClipboardLoc, ClipboardOp};

        let to_kind = |loc| match loc {
            ClipboardLoc::Clipboard => LinuxClipboardKind::Clipboard,
            ClipboardLoc::Primary => LinuxClipboardKind::Primary,
        };
        match op {
            ClipboardOp::Write { loc, mime, data } => {
                let kind = to_kind(loc);
                if mime == "image/png" {
                    self.store_image(&data, kind);
                } else if let Ok(text) = String::from_utf8(data) {
                    self.store(text, kind);
                }
            }
            ClipboardOp::Read { loc, mime } => {
                if !self.config.clipboard_read {
                    if let Some(tab) = self.terminal(id) {
                        tab.write(b"\x1b]5522;type=read:status=EACCES\x1b\\".to_vec());
                    }
                    return;
                }
                let kind = to_kind(loc);
                let payload = if mime == "image/png" {
                    self.clipboard()
                        .and_then(|c| c.get().clipboard(kind).image().ok())
                        .and_then(|img| {
                            koral::graphics::encode_png_rgba(
                                img.width as u32,
                                img.height as u32,
                                &img.bytes,
                            )
                            .ok()
                        })
                } else {
                    self.clipboard()
                        .and_then(|c| c.get().clipboard(kind).text().ok())
                        .map(String::into_bytes)
                };
                if let Some(tab) = self.terminal(id) {
                    if let Some(bytes) = payload {
                        let b64 = STANDARD.encode(&bytes);
                        let reply = format!(
                            "\x1b]5522;type=read:status=OK\x1b\\\x1b]5522;type=rdata:mime={mime};{b64}\x1b\\\x1b]5522;type=read:status=DONE\x1b\\"
                        );
                        tab.write(reply.into_bytes());
                    } else {
                        tab.write(b"\x1b]5522;type=read:status=ENODATA\x1b\\".to_vec());
                    }
                }
            }
        }
    }

    fn copy_selection(&mut self, kind: LinuxClipboardKind) {
        let text = self
            .active()
            .and_then(|tab| tab.session.lock().term.selection_to_string());
        if let Some(text) = text {
            self.write_clipboard(view::without_anchors(&text), kind);
        }
    }

    fn paste(&mut self, kind: LinuxClipboardKind) {
        let text = self
            .clipboard()
            .and_then(|clipboard| clipboard.get().clipboard(kind).text().ok());
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
        let Some((id, _, pane)) = self.active_pane() else {
            return;
        };
        self.scroll_pane(id, pane.lines(), delta);
    }

    fn scroll_pane(&mut self, pane_id: u64, lines: usize, delta: i64) {
        let Some(tab) = self.terminal_mut(pane_id) else {
            return;
        };
        let total = {
            let session = tab.session.lock();
            tab.projected_rows(&session).len()
        };
        let most = total.saturating_sub(lines) as i64;
        tab.scroll = (tab.scroll as i64 + delta).clamp(0, most) as usize;
    }

    fn wheel(&mut self, lines: i64, event_loop: &ActiveEventLoop) {
        if lines == 0 {
            return;
        }
        if self.active_is_settings() {
            let layout = self.layout();
            let picking = self
                .active_settings_mut()
                .is_some_and(|s| s.picker.is_some());
            if picking {
                let step = if lines > 0 {
                    Command::Up
                } else {
                    Command::Down
                };
                (0..lines.abs()).for_each(|_| self.settings_command(step, Some(event_loop)));
            } else if let Some(l) = layout {
                let cap = SettingsView::new(l.width, l.height, l.strip(), l.scale, l.cell.height)
                    .visible_rows;
                if let Some(s) = self.active_settings_mut() {
                    s.scroll_by(lines, cap);
                }
                self.redraw();
            }
            return;
        }
        let target_pane = self.pane_under_cursor();
        let mode = target_pane
            .as_ref()
            .and_then(|(id, _, _)| self.terminal(*id))
            .map_or_else(|| self.mode(), |t| *t.session.lock().term.mode());
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
                if let Some((id, _, _)) = target_pane
                    && let Some(term) = self.terminal_mut(id)
                {
                    term.scroll = 0;
                    term.write(key.repeat(lines.unsigned_abs() as usize).into_bytes());
                } else {
                    self.write_active(key.repeat(lines.unsigned_abs() as usize).into_bytes());
                }
            }
        } else if let Some((id, _, pane)) = target_pane {
            self.scroll_pane(id, pane.lines(), lines);
        } else {
            self.scroll_lines(lines);
        }
        self.redraw();
    }

    fn report_mouse(&mut self, button: u8, pressed: bool) -> bool {
        self.send_mouse(button, pressed, false)
    }

    /// A grabbed program that never sees motion cannot run its own selection, so a
    /// drag follows the pointer unless koral is the one selecting.
    fn report_motion(&mut self) {
        if self.drag == Drag::Selecting || matches!(self.drag, Drag::Divider(..)) {
            return;
        }
        let mode = self.mode();
        if let Some(button) = input::motion_button(mode, self.pressed) {
            self.send_mouse(button, true, true);
        }
    }

    fn send_mouse(&mut self, button: u8, pressed: bool, motion: bool) -> bool {
        let Some((id, _, pane)) = self.pane_under_cursor().or_else(|| self.active_pane()) else {
            return false;
        };
        let (line, column) = pane.cell_at(self.cursor.0, self.cursor.1);
        let mouse = Mouse {
            button,
            column,
            line,
            pressed,
            mods: self.mods,
            motion,
        };
        let Some(term) = self.terminal(id) else {
            return false;
        };
        let mode = *term.session.lock().term.mode();
        match input::mouse_report(mouse, mode) {
            Some(bytes) => {
                term.write(bytes);
                true
            }
            None => false,
        }
    }

    fn pane_under_cursor(&self) -> Option<(u64, Rect, PaneLayout)> {
        let layout = self.layout()?;
        let tree = self.tabs.as_ref()?.active()?.as_tree()?;
        let (id, rect) = tree.pane_at(
            layout.content_rect(),
            Self::divider_px(layout),
            self.cursor.0,
            self.cursor.1,
        )?;
        Some((id, rect, PaneLayout::from_rect(rect, layout)))
    }

    fn point_under_cursor(&self) -> Option<(u64, Point, Option<u32>, PaneLayout)> {
        let (id, _, pane) = self.pane_under_cursor().or_else(|| self.active_pane())?;
        let (row, column) = pane.cell_at(self.cursor.0, self.cursor.1);
        let tab = self.terminal(id)?;
        let session = tab.session.lock();
        let rows = tab.projected_rows(&session);
        let top = frame::visible_top(&rows, pane.lines(), tab.scroll);
        let target = rows.get(top + row)?;
        Some((
            id,
            Point::new(target.line, Column(column)),
            target.block,
            pane,
        ))
    }

    fn press_strip(&mut self, button: MouseButton, event_loop: &ActiveEventLoop) {
        let Some(bar) = self.bar() else {
            return;
        };
        match (hit_at(&bar, self.cursor), button) {
            (Hit::New, MouseButton::Left) => self.open_tab(),
            (Hit::Settings, MouseButton::Left) => self.toggle_settings(event_loop),
            (Hit::Close(index), MouseButton::Left) => self.request_close(index, event_loop),
            (Hit::Tab(index) | Hit::Close(index), MouseButton::Middle) => {
                self.request_close(index, event_loop)
            }
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
        if let Some(layout) = self.layout()
            && button == MouseButton::Left
            && let Some(tree) = self
                .tabs
                .as_ref()
                .and_then(Tabs::active)
                .and_then(Tab::as_tree)
            && let Some(div) = tree.divider_at(
                layout.content_rect(),
                Self::divider_px(layout),
                4.0 * layout.scale,
                self.cursor.0,
                self.cursor.1,
            )
        {
            self.drag = Drag::Divider(div.path, div.axis, div.parent);
            return;
        }
        if let Some((clicked_id, _, _)) = self.pane_under_cursor()
            && let Some(tree) = self
                .tabs
                .as_mut()
                .and_then(Tabs::active_mut)
                .and_then(Tab::as_tree_mut)
            && tree.active_id() != clicked_id
        {
            tree.focus(clicked_id);
        }

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
            if let Some((_, link)) = &self.link {
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

        let Some((id, point, block, pane)) = self.point_under_cursor() else {
            return;
        };
        let (cx, cy) = self.cursor;
        if self.config.block_minimap
            && let Some(tab) = self.terminal_mut(id)
        {
            let target = {
                let session = tab.session.lock();
                let rows = tab.projected_rows(&session);
                frame::minimap_target(&rows, &session.blocks, pane, cx, cy)
                    .map(|idx| (rows.len(), idx))
            };
            if let Some((total, index)) = target {
                tab.scroll =
                    total.saturating_sub(pane.lines() + index.saturating_sub(pane.lines() / 4));
                self.redraw();
                return;
            }
        }
        if pane.in_gutter(self.cursor.0) {
            if let (Some(block_id), Some(tab)) = (block, self.terminal(id)) {
                tab.session.lock().blocks.toggle_fold(block_id);
            }
            self.redraw();
            return;
        }
        let clicks = self
            .clicks
            .press((point.line.0, point.column.0), Instant::now());
        let kind = input::selection_kind(clicks, self.mods);
        if let Some(tab) = self.terminal(id) {
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
        let held = self.mods.control_key()
            && self.drag == Drag::None
            && !self.active_is_settings()
            && self
                .pane_under_cursor()
                .is_some_and(|(_, _, pane)| !pane.in_gutter(self.cursor.0));
        let point = if held {
            self.point_under_cursor()
        } else {
            None
        };
        let link = point.and_then(|(id, point, block, _)| {
            let session = self.terminal(id)?.session.lock();
            let cwd = block
                .and_then(|id| session.blocks.get(id))
                .and_then(|block| block.cwd.as_deref())
                .or_else(|| {
                    session
                        .blocks
                        .latest()
                        .and_then(|block| block.cwd.as_deref())
                });
            links::at_in(&session.term, point, cwd, self.home.as_deref()).map(|l| (id, l))
        });
        if link != self.link {
            let default_icon = self.cursor_icon_under_pointer();
            if let Some(window) = &self.window {
                window.set_cursor(if link.is_some() {
                    CursorIcon::Pointer
                } else {
                    default_icon
                });
            }
            self.link = link;
            self.redraw();
        }
    }

    fn cursor_icon_under_pointer(&self) -> CursorIcon {
        let shape = self
            .pane_under_cursor()
            .and_then(|(id, _, _)| self.terminal(id))
            .and_then(|term| term.session.lock().mouse_cursor.clone());
        osc_cursor_icon(shape.as_deref())
    }

    fn cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        self.cursor = (position.x as f32, position.y as f32);
        self.report_motion();
        match self.drag.clone() {
            Drag::Selecting => {
                if let Some((id, point, _, _)) = self.point_under_cursor()
                    && let Some(tab) = self.terminal(id)
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
            Drag::Divider(path, axis, parent) => {
                let ratio = match axis {
                    Axis::Horizontal => (self.cursor.0 - parent.x) / parent.width.max(1.0),
                    Axis::Vertical => (self.cursor.1 - parent.y) / parent.height.max(1.0),
                };
                if let Some(tree) = self
                    .tabs
                    .as_mut()
                    .and_then(Tabs::active_mut)
                    .and_then(Tab::as_tree_mut)
                    && tree.set_ratio(&path, ratio)
                {
                    self.resize_all();
                    self.redraw();
                }
            }
            Drag::None => {
                if self.link.is_none()
                    && let Some(layout) = self.layout()
                    && let Some(tree) = self
                        .tabs
                        .as_ref()
                        .and_then(Tabs::active)
                        .and_then(Tab::as_tree)
                {
                    let div = tree.divider_at(
                        layout.content_rect(),
                        Self::divider_px(layout),
                        4.0 * layout.scale,
                        self.cursor.0,
                        self.cursor.1,
                    );
                    let default_icon = self.cursor_icon_under_pointer();
                    if let Some(window) = &self.window {
                        let icon = match div.map(|d| d.axis) {
                            Some(Axis::Horizontal) => CursorIcon::EwResize,
                            Some(Axis::Vertical) => CursorIcon::NsResize,
                            None => default_icon,
                        };
                        window.set_cursor(icon);
                    }
                }
            }
        }
        if self
            .layout()
            .is_some_and(|layout| self.cursor.1 < layout.strip())
            || self.drag != Drag::None
        {
            self.redraw();
        }
        self.update_link();
    }

    fn resize_all(&mut self) {
        let Some(layout) = self.layout() else {
            return;
        };
        let area = layout.content_rect();
        let div = Self::divider_px(layout);
        if let Some(tabs) = &mut self.tabs {
            for tab in tabs.iter_mut() {
                if let Some(tree) = tab.as_tree_mut() {
                    let (panes, _) = tree.layout(area, div);
                    for (id, rect) in panes {
                        if let Some(term) = tree.get_mut(id) {
                            term.resize(Self::geometry_for(rect, layout));
                        }
                    }
                }
            }
        }
    }

    fn blinking(&self) -> bool {
        self.focused
            && self.active().is_some_and(|tab| {
                tab.scroll == 0 && tab.session.lock().term.cursor_style().blinking
            })
    }

    fn draw(&mut self) {
        let started = Instant::now();
        let mut cost = lag::Cost::default();
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

        let confirming = self
            .confirming
            .filter(|(_, at)| at.elapsed() < CONFIRM_WINDOW)
            .map(|(id, _)| id);
        let prompt = self.notices.take_confirmation();
        let views: Vec<TabView> = tabs
            .iter()
            .map(|tab| match tab {
                Tab::Settings(_) => TabView {
                    title: "Settings".to_string(),
                    activity: Activity::Idle,
                    confirming: false,
                    rang: false,
                    progress: None,
                },
                Tab::Terminal(tree) => {
                    let active_id = tree.active_id();
                    let active_term = tree.active();
                    let wait_from = Instant::now();
                    let session = active_term.session.lock();
                    cost.wait += lag::millis(wait_from);
                    let panes = tree.len();
                    let mut title = tabs::title(
                        &session.blocks,
                        active_term.title.as_deref(),
                        self.home.as_deref(),
                    );
                    if panes > 1 {
                        title = format!("{title} [{panes}]");
                    }
                    let (mut activity, mut rang, mut progress) = (
                        session.blocks.activity(),
                        active_term.rang,
                        session.progress,
                    );
                    drop(session);
                    for (id, term) in tree.iter() {
                        if id == active_id {
                            continue;
                        }
                        rang |= term.rang;
                        let other_sess = term.session.lock();
                        let other = other_sess.blocks.activity();
                        if activity == Activity::Idle {
                            activity = other;
                        }
                        if progress.is_none() {
                            progress = other_sess.progress;
                        }
                    }
                    TabView {
                        title,
                        activity,
                        confirming: confirming == Some(active_id),
                        rang,
                        progress,
                    }
                }
            })
            .collect();

        let preview_id = tabs.active().and_then(|tab| match tab {
            Tab::Settings(s) => s.preview_theme(),
            Tab::Terminal(_) => None,
        });
        let display_theme = preview_id
            .and_then(Theme::builtin)
            .unwrap_or_else(|| self.theme.clone());

        let mut frame = self.frame.take().unwrap_or_default();
        frame.reset();
        let mut cursor = None;
        let settings_open = tabs.active().is_some_and(Tab::is_settings);
        let coverage = self
            .renderer
            .as_ref()
            .map_or_else(fonts::Coverage::default, Renderer::coverage);

        match tabs.active() {
            Some(Tab::Terminal(tree)) => {
                let area = layout.content_rect();
                let div = Self::divider_px(layout);
                let (pane_rects, dividers) = tree.layout(area, div);
                let active_id = tree.active_id();
                let active_rect = pane_rects
                    .iter()
                    .find(|(id, _)| *id == active_id)
                    .map(|(_, r)| *r);
                let mut per_pane = Vec::with_capacity(pane_rects.len());

                for (id, rect) in &pane_rects {
                    let Some(term) = tree.get(*id) else {
                        continue;
                    };
                    let pane = PaneLayout::from_rect(*rect, layout);
                    let is_active = *id == active_id;
                    let wait_from = Instant::now();
                    let session = term.session.lock();
                    cost.wait += lag::millis(wait_from);
                    let rows = term.projected_rows(&session);
                    let pane_cursor = CursorView {
                        focused: self.focused && is_active,
                        visible: !is_active || self.blink_shown,
                    };
                    frame::draw_pane_base(
                        &mut frame,
                        &session.term,
                        &session.blocks,
                        &rows,
                        term.scroll,
                        pane_cursor,
                        pane,
                        coverage,
                        &display_theme,
                    );
                    frame::draw_pictures_in(
                        &mut frame,
                        &session.term,
                        &session.graphics,
                        &rows,
                        term.scroll,
                        pane,
                    );
                    if is_active {
                        cursor = frame::cursor_position_in(&session.term, &rows, term.scroll, pane);
                    }
                    per_pane.push((*id, pane, rows, pane_cursor, term.scroll));
                }
                frame.pictures.over = frame.quads.len();
                for (id, pane, rows, pane_cursor, scroll) in &per_pane {
                    let Some(term) = tree.get(*id) else {
                        continue;
                    };
                    let session = term.session.lock();
                    frame::draw_pane_overlays(
                        &mut frame,
                        &session.term,
                        &session.blocks,
                        rows,
                        *scroll,
                        *pane_cursor,
                        *pane,
                        &display_theme,
                    );
                    if self.config.block_minimap {
                        frame::draw_block_minimap(
                            &mut frame,
                            &session.blocks,
                            rows,
                            *pane,
                            &display_theme,
                        );
                    }
                    let top = frame::visible_top(rows, pane.lines(), *scroll);
                    let shown = &rows[top..rows.len().min(top + pane.lines())];
                    if *id == active_id
                        && let (Some(search), Some(first), Some(last)) =
                            (&mut self.search, shown.first(), shown.last())
                    {
                        let matches = search.visible(&session.term, first.line, last.line);
                        frame::draw_search_in(
                            &mut frame,
                            rows,
                            top,
                            &matches,
                            search.current.as_ref(),
                            *pane,
                            &display_theme,
                        );
                    }
                    if let Some((link_pane, link)) = &self.link
                        && link_pane == id
                    {
                        frame::draw_link_in(&mut frame, rows, top, link, *pane, &display_theme);
                    }
                }
                frame::draw_pane_dividers(
                    &mut frame,
                    &dividers,
                    active_rect,
                    layout.scale,
                    &display_theme,
                );
            }
            Some(Tab::Settings(_)) | None => {}
        }

        if let (Some(text), Some((x, y))) = (&self.preedit, cursor) {
            frame::draw_preedit(&mut frame, text, x, y, layout, &display_theme);
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
            settings_open,
            bar: &bar,
        };
        frame::draw_chrome(&mut frame, &chrome, layout, &display_theme);
        if let Some(Tab::Settings(settings)) = tabs.active() {
            let substitute = self
                .renderer
                .as_ref()
                .and_then(|renderer| renderer.substitute());
            let notice =
                substitute.map(|used| format!("Can't use {}. Showing {used}.", self.config.font));
            frame::draw_settings(
                &mut frame,
                settings,
                &self.config,
                self.settings_error.as_deref().or(notice.as_deref()),
                layout,
                &display_theme,
            );
        } else if let Some(overlay) = &self.diff_overlay {
            frame::draw_diff_overlay(
                &mut frame,
                &overlay.title,
                &overlay.lines,
                overlay.scroll,
                layout,
                &display_theme,
            );
        } else if let Some(search) = &self.search {
            frame::draw_search_bar_scoped(
                &mut frame,
                &search.query,
                search.current.is_some(),
                search.position,
                search.scope.is_some(),
                layout,
                &display_theme,
            );
        } else if let Some((_, link)) = &self.link {
            frame::draw_link_bar(&mut frame, &link.uri, layout, &display_theme);
        }
        if let Some(prompt) = &prompt {
            frame::draw_consent(&mut frame, prompt, layout, &display_theme);
        }

        if let (Some(window), Some(view)) = (&self.window, views.get(tabs.active_index()))
            && view.title != self.window_title
        {
            window.set_title(&view.title);
            self.window_title = view.title.clone();
        }
        if let (Some(window), Some((x, y)), true) =
            (&self.window, cursor, cursor != self.ime_cursor)
        {
            let size = PhysicalSize::new(layout.cell.width, layout.cell.height);
            window.set_ime_cursor_area(PhysicalPosition::new(x, y), size);
            self.ime_cursor = cursor;
        }
        cost.grid = lag::millis(started);
        if let Some(renderer) = &mut self.renderer {
            renderer.draw(
                &frame.quads,
                &frame.pictures,
                &frame.lines,
                frame.clip,
                rgba(display_theme.background, self.config.opacity),
                &mut cost,
            );
        }
        self.frame = Some(frame);
        self.publish_a11y();
        cost.total = lag::millis(started);
        lag::note(&cost);
    }

    fn publish_a11y(&mut self) {
        if self.a11y.is_none() {
            return;
        }
        let pane_lines = self.layout().map_or(24, |l| {
            let area = l.content_rect();
            let div = Self::divider_px(l);
            self.tabs
                .as_ref()
                .and_then(Tabs::active)
                .and_then(Tab::as_tree)
                .and_then(|tree| {
                    let active_id = tree.active_id();
                    tree.layout(area, div)
                        .0
                        .into_iter()
                        .find(|(id, _)| *id == active_id)
                        .map(|(_, rect)| PaneLayout::from_rect(rect, l).lines())
                })
                .unwrap_or(24)
        });
        let focused = self.focused;
        let window_title = if self.window_title.is_empty() {
            "koral".to_string()
        } else {
            self.window_title.clone()
        };
        let active_title = match self.tabs.as_ref().and_then(Tabs::active) {
            Some(Tab::Settings(_)) => "Settings".to_string(),
            Some(Tab::Terminal(tree)) => {
                let term = tree.active();
                let session = term.session.lock();
                tabs::title(&session.blocks, term.title.as_deref(), self.home.as_deref())
            }
            None => "koral".to_string(),
        };
        let Some(a11y) = &mut self.a11y else {
            return;
        };
        let tabs_ref = &self.tabs;
        a11y.update_if_active(|| {
            let lines = match tabs_ref.as_ref().and_then(Tabs::active) {
                Some(Tab::Terminal(tree)) => {
                    let term = tree.active();
                    let session = term.session.lock();
                    let rows = term.projected_rows(&session);
                    koral::a11y::visible_lines(&session.term, &rows, term.scroll, pane_lines)
                }
                Some(Tab::Settings(_)) => vec!["Settings".to_string()],
                None => Vec::new(),
            };
            koral::a11y::build_tree(&window_title, &active_title, &lines, focused)
        });
    }

    fn handle_ipc(&mut self, request: Request, event_loop: &ActiveEventLoop) -> String {
        match request {
            Request::NewTab {
                cwd,
                title,
                command,
            } => {
                self.open_tab_with(cwd, title, command);
                "OK\n".to_string()
            }
            Request::Split { axis, cwd } => {
                if self.split_active(axis, cwd) {
                    "OK\n".to_string()
                } else {
                    "ERR active tab is not a terminal\n".to_string()
                }
            }
            Request::ClosePane { pane } => match self.close_pane_target(pane, event_loop) {
                Ok(()) => "OK\n".to_string(),
                Err(message) => format!("ERR {message}\n"),
            },
            Request::CloseTab => {
                let Some(idx) = self.tabs.as_ref().map(Tabs::active_index) else {
                    return "ERR no active tab\n".to_string();
                };
                self.close_tab(idx, event_loop);
                "OK\n".to_string()
            }
            Request::Focus(dir) => {
                self.focus_dir(dir);
                "OK\n".to_string()
            }
            Request::Resize(dir) => {
                self.resize_dir(dir);
                "OK\n".to_string()
            }
            Request::SetTitle { pane, title } => {
                let term = match pane {
                    Some(id) => self.terminal_mut(id).ok_or("pane not found"),
                    None => self.active_mut().ok_or("active tab is not a terminal"),
                };
                match term {
                    Ok(term) => {
                        term.title = tabs::clean_title(&title);
                        self.redraw();
                        "OK\n".to_string()
                    }
                    Err(message) => format!("ERR {message}\n"),
                }
            }
            Request::SetTheme(id) => match self.apply_setting_update("theme", &id) {
                Ok(()) => {
                    self.redraw();
                    "OK\n".to_string()
                }
                Err(message) => format!("ERR {message}\n"),
            },
            Request::OpenSettings => {
                self.open_or_focus_settings();
                "OK\n".to_string()
            }
            Request::Reload => {
                self.reload_config();
                "OK\n".to_string()
            }
            Request::SendText { pane, text } => {
                let term = match pane {
                    Some(id) => self.terminal_mut(id).ok_or("pane not found"),
                    None => self.active_mut().ok_or("active tab is not a terminal"),
                };
                match term {
                    Ok(term) => {
                        term.write(text.into_bytes());
                        self.redraw();
                        "OK\n".to_string()
                    }
                    Err(message) => format!("ERR {message}\n"),
                }
            }
            Request::List | Request::ListJson => {
                let json = matches!(request, Request::ListJson);
                let model = self.list_tabs_info();
                if json {
                    format!("OK\n{}\n", ipc::render_list_json(&model))
                } else {
                    format!("OK\n{}", ipc::render_list_text(&model))
                }
            }
            Request::ListBlocks { pane, json } => {
                let target = match pane {
                    Some(id) => self.terminal(id).map(|t| (id, t)).ok_or("pane not found"),
                    None => self
                        .active()
                        .map(|t| (t.id, t))
                        .ok_or("active tab is not a terminal"),
                };
                match target {
                    Ok((pane_id, term)) => {
                        let infos: Vec<ipc::BlockInfo> = {
                            let session = term.session.lock();
                            session
                                .blocks
                                .iter()
                                .map(|b| ipc::BlockInfo {
                                    id: b.id,
                                    pane_id,
                                    command: b.command.clone(),
                                    cwd: b.cwd.clone(),
                                    status: b.status,
                                    folded: b.folded,
                                })
                                .collect()
                        };
                        if json {
                            format!("OK\n{}\n", ipc::render_blocks_json(&infos))
                        } else {
                            format!("OK\n{}", ipc::render_blocks_text(&infos))
                        }
                    }
                    Err(message) => format!("ERR {message}\n"),
                }
            }
            Request::GetBlockOutput {
                pane,
                selector,
                json,
            } => {
                let target = match pane {
                    Some(id) => self.terminal(id).map(|t| (id, t)).ok_or("pane not found"),
                    None => self
                        .active()
                        .map(|t| (t.id, t))
                        .ok_or("active tab is not a terminal"),
                };
                match target {
                    Ok((pane_id, term)) => {
                        let session = term.session.lock();
                        let block_id = match selector {
                            BlockSelector::Last => session
                                .blocks
                                .iter()
                                .rfind(|b| b.status.is_some())
                                .or_else(|| session.blocks.latest())
                                .map(|b| b.id),
                            BlockSelector::Offset(n) => session
                                .blocks
                                .iter()
                                .rev()
                                .filter(|b| b.status.is_some())
                                .nth(n.saturating_sub(1))
                                .map(|b| b.id),
                            BlockSelector::LastFailed => session
                                .blocks
                                .iter()
                                .rev()
                                .find(|b| b.status.is_some_and(|code| code != 0))
                                .map(|b| b.id),
                            BlockSelector::Id(id) => Some(id),
                        };
                        let Some(block_id) = block_id else {
                            return "ERR block not found\n".to_string();
                        };
                        let Some(output) =
                            view::output_text(&session.term, &session.blocks, block_id)
                        else {
                            return "ERR block output not available\n".to_string();
                        };
                        if json {
                            format!(
                                "OK\n{}\n",
                                ipc::render_block_output_json(pane_id, block_id, &output)
                            )
                        } else {
                            format!("OK\n{output}\n")
                        }
                    }
                    Err(message) => format!("ERR {message}\n"),
                }
            }
            Request::Watch { .. } => "ERR watch must stream\n".to_string(),
            Request::SaveSession(name) => match self.save_named_session(&name) {
                Ok(()) => "OK\n".to_string(),
                Err(message) => format!("ERR {message}\n"),
            },
            Request::RestoreSession(name) => match self.restore_named_session(&name) {
                Ok(()) => "OK\n".to_string(),
                Err(message) => format!("ERR {message}\n"),
            },
            Request::ListSessions | Request::ListSessionsJson => {
                let json = matches!(request, Request::ListSessionsJson);
                let names = match koral::workspace::dir() {
                    Some(dir) => match koral::workspace::list(&dir) {
                        Ok(list) => list,
                        Err(message) => return format!("ERR {message}\n"),
                    },
                    None => Vec::new(),
                };
                if json {
                    format!("OK\n{}\n", koral::workspace::render_sessions_json(&names))
                } else {
                    format!("OK\n{}", koral::workspace::render_sessions_text(&names))
                }
            }
        }
    }

    fn list_tabs_info(&self) -> Vec<ipc::TabInfo> {
        let Some(tabs) = &self.tabs else {
            return Vec::new();
        };
        let active_tab = tabs.active_index();
        tabs.iter()
            .enumerate()
            .map(|(index, tab)| {
                let active = index == active_tab;
                let kind = match tab {
                    Tab::Settings(_) => ipc::TabKind::Settings,
                    Tab::Terminal(tree) => {
                        let active_pane = tree.active_id();
                        let active_term = tree.active();
                        let title = {
                            let session = active_term.session.lock();
                            tabs::title(
                                &session.blocks,
                                active_term.title.as_deref(),
                                self.home.as_deref(),
                            )
                        };
                        let panes = tree
                            .iter()
                            .into_iter()
                            .map(|(pane_id, term)| {
                                let pane_title = {
                                    let session = term.session.lock();
                                    tabs::title(
                                        &session.blocks,
                                        term.title.as_deref(),
                                        self.home.as_deref(),
                                    )
                                };
                                ipc::PaneInfo {
                                    id: pane_id,
                                    active: pane_id == active_pane,
                                    title: pane_title,
                                }
                            })
                            .collect();
                        ipc::TabKind::Terminal {
                            title,
                            active_pane,
                            panes,
                        }
                    }
                };
                ipc::TabInfo {
                    index,
                    active,
                    kind,
                }
            })
            .collect()
    }

    fn save_named_session(&self, name: &str) -> Result<(), String> {
        let dir = koral::workspace::dir()
            .ok_or_else(|| "cannot resolve session directory".to_string())?;
        let Some(tabs) = &self.tabs else {
            return Err("no open terminal tabs".to_string());
        };
        let active_idx = tabs.active_index();
        let mut active_tab = 0usize;
        let mut saved_tabs = Vec::new();
        for (idx, tab) in tabs.iter().enumerate() {
            let Tab::Terminal(tree) = tab else {
                continue;
            };
            if idx == active_idx {
                active_tab = saved_tabs.len();
            }
            let title = tree.active().title.clone();
            let layout = tree.snapshot(|_, term| {
                term.session
                    .lock()
                    .blocks
                    .latest()
                    .and_then(|block| block.cwd.clone())
            });
            saved_tabs.push(koral::workspace::SavedTab { title, layout });
        }
        if saved_tabs.is_empty() {
            return Err("no open terminal tabs".to_string());
        }
        let session = koral::workspace::SavedSession {
            version: koral::workspace::VERSION,
            name: name.to_string(),
            active_tab,
            tabs: saved_tabs,
        };
        koral::workspace::save(&dir, &session).map(|_| ())
    }

    fn restore_named_session(&mut self, name: &str) -> Result<(), String> {
        let dir = koral::workspace::dir()
            .ok_or_else(|| "cannot resolve session directory".to_string())?;
        let saved = koral::workspace::load(&dir, name)?;
        if saved.tabs.is_empty() {
            return Err(format!("session '{name}' has no saved tabs"));
        }
        let layout = self
            .layout()
            .ok_or_else(|| "window layout is not ready".to_string())?;
        let geometry = Self::geometry_for(layout.content_rect(), layout);
        let fallback_cwd = self.active_cwd();
        let mut built = Vec::with_capacity(saved.tabs.len());
        for saved_tab in saved.tabs {
            let mut tree = SplitTree::from_saved(&saved_tab.layout, &mut |cwd| {
                let target_cwd = cwd.map(PathBuf::from).or_else(|| fallback_cwd.clone());
                let term = self
                    .spawn_terminal(geometry, target_cwd, None, None)
                    .ok_or_else(|| "failed to spawn terminal pane".to_string())?;
                Ok::<_, String>((term.id, term))
            })?;
            tree.active_mut().title = saved_tab.title.as_deref().and_then(tabs::clean_title);
            built.push(Tab::Terminal(tree));
        }
        let base = self.tabs.as_ref().map_or(0, Tabs::len);
        let target_active = base + saved.active_tab.min(built.len() - 1);
        for tab in built {
            match &mut self.tabs {
                Some(tabs) => tabs.push(tab),
                None => self.tabs = Some(Tabs::new(tab)),
            }
        }
        if let Some(tabs) = &mut self.tabs {
            tabs.select(target_active);
        }
        self.search = None;
        self.link = None;
        self.resize_all();
        self.redraw();
        Ok(())
    }

    fn close_pane_target(
        &mut self,
        pane: Option<u64>,
        event_loop: &ActiveEventLoop,
    ) -> Result<(), &'static str> {
        let Some(tabs) = self.tabs.as_mut() else {
            return Err("no active tab");
        };
        let (tab_idx, res) = match pane {
            Some(id) => {
                let mut found = None;
                for (idx, tab) in tabs.iter_mut().enumerate() {
                    if let Some(tree) = tab.as_tree_mut()
                        && tree.contains(id)
                    {
                        found = Some((idx, tree.close(id)));
                        break;
                    }
                }
                found.ok_or("pane not found")?
            }
            None => {
                let idx = tabs.active_index();
                let res = tabs.active_mut().and_then(Tab::as_tree_mut).map(|tree| {
                    let id = tree.active_id();
                    tree.close(id)
                });
                match res {
                    None => (idx, CloseResult::LastPane),
                    Some(res) => (idx, res),
                }
            }
        };
        match res {
            CloseResult::LastPane => self.close_tab(tab_idx, event_loop),
            CloseResult::Closed => {
                self.search = None;
                self.link = None;
                self.resize_all();
                self.redraw();
            }
            CloseResult::NotFound => return Err("pane not found"),
        }
        Ok(())
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
        let (face, points) = (self.config.face(), self.config.font_size);
        let cache = fonts::cache_path();
        let quick_loader = {
            let (face, cache) = (face.clone(), cache.clone());
            std::thread::spawn(move || {
                cache
                    .as_deref()
                    .and_then(|path| fonts::quick(path, &face))
                    .or_else(|| fonts::listed(&face))
                    .or_else(|| fonts::listed_fallback(&face))
            })
        };
        let title = self.args.title.as_deref().unwrap_or("koral");
        let class = self.args.class.as_deref().unwrap_or("koral");
        let attributes = Window::default_attributes()
            .with_title(title)
            .with_inner_size(LogicalSize::new(960.0, 600.0))
            .with_transparent(true)
            .with_visible(false)
            .with_decorations(self.config.window_decorations);
        let attributes = WindowAttributesExtWayland::with_name(attributes, class, class);
        let attributes = WindowAttributesExtX11::with_name(attributes, class, class);
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                report(&format!("cannot open a window: {error}"));
                event_loop.exit();
                return;
            }
        };
        let a11y = accesskit_winit::Adapter::with_event_loop_proxy(
            event_loop,
            &window,
            self.proxy.clone(),
        );
        self.a11y = Some(a11y);
        lag::startup("window");
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(event_loop.owned_display_handle()),
        ));
        lag::startup("instance");
        let quick = quick_loader.join().ok().flatten();
        lag::startup(if quick.is_some() {
            "fonts_quick"
        } else {
            "fonts_full_pending"
        });
        if quick.is_some() {
            let (proxy, face, cache) = (self.proxy.clone(), face.clone(), cache.clone());
            std::thread::spawn(move || {
                let mut fonts = fonts::full(cache.as_deref(), &face);
                // Off the window thread: no frame pays for reading the fallback faces.
                let usable = fonts::usable(&face, fonts.db());
                fonts::warm(&mut fonts, &usable, points);
                let list = fonts::monospaced(fonts.db());
                let _ = proxy.send_event(UserEvent::Fonts(Box::new(fonts), list));
            });
        }
        let fonts = quick.unwrap_or_else(|| {
            let mut fonts = fonts::full(cache.as_deref(), &face);
            // No cache file and nothing fontconfig knows: this runs before the first frame, so the
            // same warm-up costs the window thread its start-up rather than a later frame.
            let usable = fonts::usable(&face, fonts.db());
            fonts::warm(&mut fonts, &usable, points);
            self.fonts = fonts::monospaced(fonts.db());
            fonts
        });
        lag::startup("fonts_ready");
        self.renderer = Some(pollster::block_on(Renderer::new(
            window.clone(),
            instance,
            &face,
            points,
            fonts,
            self.config.present_mode,
        )));
        lag::startup("renderer_ready");
        window.set_ime_allowed(true);
        window.set_visible(true);
        self.window = Some(window);
        self.sync_refresh_rate();
        if self.ipc.is_none() {
            match ipc::Server::start(self.proxy.clone()) {
                Ok(server) => self.ipc = Some(server),
                Err(error) => report(&format!("cannot listen on ipc socket: {error}")),
            }
        }
        lag::startup("ipc_ready");
        self.open_tab();
        lag::startup("first_tab_ready");
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Redraw => self.redraw(),
            UserEvent::Exited(id) => {
                let target = self.tabs.as_ref().and_then(|tabs| {
                    tabs.iter().enumerate().find_map(|(index, tab)| {
                        tab.as_tree()
                            .is_some_and(|tree| tree.contains(id))
                            .then_some(index)
                    })
                });
                if let Some(index) = target {
                    let result = self
                        .tabs
                        .as_mut()
                        .and_then(|tabs| tabs.iter_mut().nth(index))
                        .and_then(Tab::as_tree_mut)
                        .map_or(CloseResult::NotFound, |tree| tree.close(id));
                    match result {
                        CloseResult::LastPane => self.close_tab(index, event_loop),
                        CloseResult::Closed => {
                            if self.confirming.is_some_and(|(cid, _)| cid == id) {
                                self.confirming = None;
                            }
                            if self.link.as_ref().is_some_and(|(lid, _)| *lid == id) {
                                self.link = None;
                            }
                            self.resize_all();
                            self.redraw();
                        }
                        CloseResult::NotFound => {}
                    }
                }
            }
            UserEvent::Reply(id, text) => {
                if let Some(tab) = self.terminal(id) {
                    tab.write(text.into_bytes());
                }
            }
            UserEvent::Copy(text) => self.store(text, LinuxClipboardKind::Clipboard),
            UserEvent::PasteRequest(id, kind, reply) => {
                if !self.config.clipboard_read {
                    return;
                }
                let kind = match kind {
                    ClipboardType::Selection => LinuxClipboardKind::Primary,
                    ClipboardType::Clipboard => LinuxClipboardKind::Clipboard,
                };
                let text = self
                    .clipboard()
                    .and_then(|c| c.get().clipboard(kind).text().ok());
                if let (Some(text), Some(tab)) = (text, self.terminal(id)) {
                    tab.write(reply(&text).into_bytes());
                }
            }
            UserEvent::Title(id, title) => {
                if let Some(tab) = self.terminal_mut(id) {
                    tab.title = title;
                }
                self.redraw();
            }
            UserEvent::Bell(id) => self.bell(id),
            UserEvent::Notify(id, notice) => {
                let in_view = self.focused && self.active().is_some_and(|tab| tab.id == id);
                if in_view {
                    return;
                }
                let decision = self.notices.notify(notice.clone(), Instant::now());
                match decision {
                    Decision::Denied | Decision::Held | Decision::Confirm => {
                        self.redraw();
                    }
                    Decision::Performed => {
                        if let Some(ready) = self.notices.take_ready_notice(Instant::now()) {
                            notify::show(&notice);
                            let _ = ready;
                        }
                    }
                }
            }
            UserEvent::ClipboardOp(id, op) => self.handle_osc5522(id, op),
            UserEvent::BlockEvent(id, event) => {
                if let Some(ipc) = &self.ipc {
                    ipc.publish(id, &ipc::render_event_ndjson(&event));
                }
            }
            UserEvent::Blink => {
                self.blink.reset(Instant::now());
                self.redraw();
            }
            UserEvent::Colors => {
                self.theme =
                    matugen::resolve_theme(self.config.matugen, self.config.theme.as_deref());
                self.redraw();
            }
            UserEvent::Config => self.reload_config(),
            UserEvent::Fonts(fonts, list) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.set_fonts(*fonts);
                }
                if let Some(tabs) = &mut self.tabs {
                    for tab in tabs.iter_mut() {
                        if let Tab::Settings(settings) = tab {
                            settings.fonts = list.clone();
                        }
                    }
                }
                self.fonts = list;
                self.resize_all();
                self.redraw();
            }
            UserEvent::Size(id, reply) => {
                let rect = self.layout().and_then(|layout| {
                    let area = layout.content_rect();
                    let div = Self::divider_px(layout);
                    self.tabs.as_ref()?.iter().find_map(|tab| {
                        let tree = tab.as_tree()?;
                        if !tree.contains(id) {
                            return None;
                        }
                        tree.layout(area, div)
                            .0
                            .into_iter()
                            .find(|(pid, _)| *pid == id)
                            .map(|(_, r)| (r, layout))
                    })
                });
                if let (Some((rect, layout)), Some(tab)) = (rect, self.terminal(id)) {
                    let geometry = Self::geometry_for(rect, layout);
                    tab.write(reply(geometry.window_size()).into_bytes());
                }
            }
            UserEvent::Color(id, index, reply) => {
                let color = self.theme.palette(index);
                if let Some(tab) = self.terminal(id) {
                    tab.write(reply(color).into_bytes());
                }
            }
            UserEvent::Ipc(request, reply) => {
                let response = self.handle_ipc(request, event_loop);
                let _ = reply.send(response);
            }
            UserEvent::AccessKit(event) => {
                if let accesskit_winit::WindowEvent::InitialTreeRequested = event.window_event {
                    self.publish_a11y();
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let (Some(a11y), Some(window)) = (&mut self.a11y, &self.window) {
            a11y.process_event(window, &event);
        }
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
                    tab.write(if focused {
                        b"\x1b[I".to_vec()
                    } else {
                        b"\x1b[O".to_vec()
                    });
                }
                self.redraw();
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.blink.reset(Instant::now());
                if self.permission_key(&event) {
                    return;
                }
                let action = resolve_binding(&event.logical_key, self.mods, &self.config.bindings);
                let in_settings = self.active_is_settings();
                if in_settings {
                    let tab_nav = matches!(
                        action,
                        Some(
                            Action::OpenSettings
                                | Action::NewTab
                                | Action::CloseTab
                                | Action::NextTab
                                | Action::PreviousTab
                                | Action::SelectTab(_)
                                | Action::ZoomIn
                                | Action::ZoomOut
                                | Action::ZoomReset
                        )
                    );
                    if tab_nav && let Some(action) = action {
                        self.run(action, event_loop);
                    } else {
                        self.settings_key(&event.logical_key, event.text.as_deref(), event_loop);
                    }
                    return;
                }
                if let Some(action) = action {
                    self.run(action, event_loop);
                    return;
                }
                if self.diff_overlay.is_some() {
                    self.diff_overlay_key(&event.logical_key, event.text.as_deref());
                    return;
                }
                if self.search.is_some() {
                    self.search_key(&event.logical_key, event.text.as_deref());
                    return;
                }
                let text = if self.mods.control_key() {
                    None
                } else {
                    event.text.as_deref()
                };
                let stage = if event.repeat {
                    Stage::Repeat
                } else {
                    Stage::Press
                };
                let bytes = self
                    .kitty_key(&event, stage)
                    .or_else(|| input::encode(&event.logical_key, text, self.mods, self.mode()));
                if let Some(bytes) = bytes {
                    let mut needs_redraw = !self.blink_shown;
                    if let Some(tab) = self.active_mut() {
                        needs_redraw |= tab.scroll != 0;
                        tab.scroll = 0;
                        needs_redraw |= tab.session.lock().term.selection.take().is_some();
                        tab.write(bytes);
                    }
                    if needs_redraw {
                        self.redraw();
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. }
                if !self.active_is_settings() && self.search.is_none() =>
            {
                if let (Some(bytes), Some(tab)) =
                    (self.kitty_key(&event, Stage::Release), self.active())
                {
                    tab.write(bytes);
                }
            }
            WindowEvent::Ime(ime) => self.ime(ime),
            WindowEvent::CursorMoved { position, .. } => self.cursor_moved(position),
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button,
                ..
            } => {
                if self
                    .layout()
                    .is_some_and(|layout| self.cursor.1 < layout.strip())
                {
                    self.press_strip(button, event_loop);
                } else if self.active_is_settings() {
                    if button == MouseButton::Left {
                        self.press_settings(event_loop);
                    }
                } else {
                    self.press_terminal(button);
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                ..
            } => self.release(),
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => (y * 3.0) as i64,
                    MouseScrollDelta::PixelDelta(position) => {
                        let height = self.layout().map_or(20.0, |layout| layout.cell.height);
                        (position.y as f32 / height) as i64
                    }
                };
                self.wheel(lines, event_loop);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.sync_refresh_rate();
        if self
            .confirming
            .is_some_and(|(_, at)| at.elapsed() >= CONFIRM_WINDOW)
        {
            self.confirming = None;
            self.redraw();
        }
        let now = Instant::now();
        if let Some(notice) = self.notices.take_ready_notice(now) {
            notify::show(&notice);
        }
        if self
            .notices
            .confirmation_due()
            .is_some_and(|due| now >= due)
        {
            self.notices.cancel();
            self.redraw();
        }
        let mut blink = None;
        if self.blinking() {
            blink = self.blink.next(now);
            if self.blink.visible(now) != self.blink_shown {
                self.redraw();
            }
        }
        let confirm = self.confirming.map(|(_, at)| at + CONFIRM_WINDOW);
        let ask = self.notices.confirmation_due();
        let notice = self.notices.notification_due();
        let mut anim_deadline = None;
        let mut anim_redraw = false;
        if let Some(tabs) = &self.tabs {
            for tab in tabs.iter() {
                if let Some(tree) = tab.as_tree() {
                    for (_, term) in tree.iter() {
                        let mut session = term.session.lock();
                        let (changed, next) = session.graphics.advance_animations(now);
                        if changed {
                            anim_redraw = true;
                        }
                        if let Some(next) = next {
                            anim_deadline =
                                Some(anim_deadline.map_or(next, |prev: Instant| prev.min(next)));
                        }
                    }
                }
            }
        }
        if anim_redraw {
            self.redraw();
        }
        let deadline = blink
            .into_iter()
            .chain(confirm)
            .chain(ask)
            .chain(notice)
            .chain(anim_deadline)
            .min();
        event_loop.set_control_flow(match deadline {
            Some(deadline) => ControlFlow::WaitUntil(deadline),
            None => ControlFlow::Wait,
        });
    }
}

fn osc_cursor_icon(name: Option<&str>) -> CursorIcon {
    match name {
        Some("text" | "xterm") => CursorIcon::Text,
        Some("pointer" | "hand" | "hand2") => CursorIcon::Pointer,
        Some("grab") => CursorIcon::Grab,
        Some("grabbing") => CursorIcon::Grabbing,
        Some("wait") => CursorIcon::Wait,
        Some("progress") => CursorIcon::Progress,
        Some("crosshair") => CursorIcon::Crosshair,
        Some("move" | "all-scroll") => CursorIcon::Move,
        Some("not-allowed" | "no-drop") => CursorIcon::NotAllowed,
        Some("ew-resize" | "col-resize") => CursorIcon::EwResize,
        Some("ns-resize" | "row-resize") => CursorIcon::NsResize,
        Some("nesw-resize") => CursorIcon::NeswResize,
        Some("nwse-resize") => CursorIcon::NwseResize,
        _ => CursorIcon::Default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(key: Key, mods: ModifiersState, repeat: bool) -> Option<Consent> {
        consent_key(&key, mods, repeat)
    }

    #[test]
    fn exact_ctrl_shift_enter_allows() {
        assert_eq!(
            press(
                Key::Named(NamedKey::Enter),
                ModifiersState::CONTROL | ModifiersState::SHIFT,
                false
            ),
            Some(Consent::Allow)
        );
    }

    #[test]
    fn escape_denies() {
        assert_eq!(
            press(Key::Named(NamedKey::Escape), ModifiersState::empty(), false),
            Some(Consent::Deny)
        );
    }

    #[test]
    fn plain_enter_and_partial_modifiers_pass_through() {
        assert_eq!(
            press(Key::Named(NamedKey::Enter), ModifiersState::empty(), false),
            None
        );
        assert_eq!(
            press(Key::Named(NamedKey::Enter), ModifiersState::SHIFT, false),
            None
        );
        assert_eq!(
            press(Key::Named(NamedKey::Enter), ModifiersState::CONTROL, false),
            None
        );
    }

    #[test]
    fn repeats_pass_through() {
        assert_eq!(
            press(
                Key::Named(NamedKey::Enter),
                ModifiersState::CONTROL | ModifiersState::SHIFT,
                true
            ),
            None
        );
        assert_eq!(
            press(Key::Named(NamedKey::Escape), ModifiersState::empty(), true),
            None
        );
    }
}
