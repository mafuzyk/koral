use std::fs::Permissions;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use winit::event_loop::EventLoopProxy;

use crate::pty::UserEvent;
use crate::splits::{Axis, Dir};

const IO_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_LINE: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    NewTab {
        cwd: Option<PathBuf>,
        title: Option<String>,
        command: Option<(String, Vec<String>)>,
    },
    Split {
        axis: Axis,
        cwd: Option<PathBuf>,
    },
    ClosePane,
    CloseTab,
    Focus(Dir),
    Resize(Dir),
    SetTitle(String),
    SetTheme(String),
    OpenSettings,
    Reload,
    SendText(String),
    List,
    ListJson,
}

pub const CTL_USAGE: &str = "\
Usage: koral ctl [--socket PATH] [--json] COMMAND [ARGS...]

Commands:
  new-tab [--cwd PATH] [--title TEXT] [-e|-- COMMAND [ARGS...]]
  split right|down|horizontal|vertical [--cwd PATH]
  close-pane
  close-tab
  focus left|right|up|down
  resize left|right|up|down
  set-title TEXT
  set-theme ID
  open-settings
  reload
  send-text TEXT
  list [--json]
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneInfo {
    pub id: u64,
    pub active: bool,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabKind {
    Settings,
    Terminal {
        title: String,
        active_pane: u64,
        panes: Vec<PaneInfo>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabInfo {
    pub index: usize,
    pub active: bool,
    pub kind: TabKind,
}

pub fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                use std::fmt::Write;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn json_error(message: &str) -> String {
    format!("{{\"version\":1,\"error\":{}}}", json_string(message))
}

pub fn render_list_text(tabs: &[TabInfo]) -> String {
    let mut out = String::new();
    for tab in tabs {
        let tab_mark = if tab.active { "*" } else { " " };
        match &tab.kind {
            TabKind::Settings => {
                out.push_str(&format!("tab {}{} settings\n", tab.index, tab_mark));
            }
            TabKind::Terminal {
                title,
                active_pane,
                panes,
            } => {
                out.push_str(&format!(
                    "tab {}{} panes={} active_pane={} title={title}\n",
                    tab.index,
                    tab_mark,
                    panes.len(),
                    active_pane
                ));
                for pane in panes {
                    let pane_mark = if pane.active { "*" } else { " " };
                    out.push_str(&format!(
                        "  pane {}{} title={}\n",
                        pane.id, pane_mark, pane.title
                    ));
                }
            }
        }
    }
    out
}

pub fn render_list_json(tabs: &[TabInfo]) -> String {
    use std::fmt::Write;
    let mut out = String::from("{\"version\":1,\"tabs\":[");
    for (tab_pos, tab) in tabs.iter().enumerate() {
        if tab_pos > 0 {
            out.push(',');
        }
        match &tab.kind {
            TabKind::Settings => {
                let _ = write!(
                    out,
                    "{{\"index\":{},\"active\":{},\"kind\":\"settings\"}}",
                    tab.index, tab.active
                );
            }
            TabKind::Terminal {
                title,
                active_pane,
                panes,
            } => {
                let _ = write!(
                    out,
                    "{{\"index\":{},\"active\":{},\"kind\":\"terminal\",\"title\":{},\"active_pane\":{},\"panes\":[",
                    tab.index,
                    tab.active,
                    json_string(title),
                    active_pane
                );
                for (pane_pos, pane) in panes.iter().enumerate() {
                    if pane_pos > 0 {
                        out.push(',');
                    }
                    let _ = write!(
                        out,
                        "{{\"id\":{},\"active\":{},\"title\":{}}}",
                        pane.id,
                        pane.active,
                        json_string(&pane.title)
                    );
                }
                out.push_str("]}");
            }
        }
    }
    out.push_str("]}");
    out
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            other => out.push(other),
        }
    }
    out
}

fn unescape(text: &str) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some(other) => return Err(format!("invalid escape '\\{other}'")),
            None => return Err("trailing backslash".to_string()),
        }
    }
    Ok(out)
}

pub fn encode(req: &Request) -> String {
    let mut parts: Vec<String> = Vec::new();
    match req {
        Request::NewTab {
            cwd,
            title,
            command,
        } => {
            parts.push("new-tab".to_string());
            if let Some(cwd) = cwd {
                parts.push(format!("cwd:{}", escape(&cwd.to_string_lossy())));
            }
            if let Some(title) = title {
                parts.push(format!("title:{}", escape(title)));
            }
            if let Some((prog, args)) = command {
                parts.push(format!("cmd:{}", escape(prog)));
                for arg in args {
                    parts.push(format!("arg:{}", escape(arg)));
                }
            }
        }
        Request::Split { axis, cwd } => {
            parts.push("split".to_string());
            parts.push(match axis {
                Axis::Horizontal => "horizontal".to_string(),
                Axis::Vertical => "vertical".to_string(),
            });
            if let Some(cwd) = cwd {
                parts.push(format!("cwd:{}", escape(&cwd.to_string_lossy())));
            }
        }
        Request::ClosePane => parts.push("close-pane".to_string()),
        Request::CloseTab => parts.push("close-tab".to_string()),
        Request::Focus(dir) => {
            parts.push("focus".to_string());
            parts.push(dir_name(*dir).to_string());
        }
        Request::Resize(dir) => {
            parts.push("resize".to_string());
            parts.push(dir_name(*dir).to_string());
        }
        Request::SetTitle(title) => {
            parts.push("set-title".to_string());
            parts.push(escape(title));
        }
        Request::SetTheme(id) => {
            parts.push("set-theme".to_string());
            parts.push(escape(id));
        }
        Request::OpenSettings => parts.push("open-settings".to_string()),
        Request::Reload => parts.push("reload".to_string()),
        Request::SendText(text) => {
            parts.push("send-text".to_string());
            parts.push(escape(text));
        }
        Request::List => parts.push("list".to_string()),
        Request::ListJson => parts.push("list-json".to_string()),
    }
    let mut line = parts.join("\t");
    line.push('\n');
    line
}

pub fn decode(line: &str) -> Result<Request, String> {
    let trimmed = line.trim_end_matches(['\r', '\n']);
    let mut fields = trimmed.split('\t');
    let cmd = fields.next().ok_or_else(|| "empty request".to_string())?;
    match cmd {
        "new-tab" => {
            let mut cwd = None;
            let mut title = None;
            let mut prog = None;
            let mut cmd_args = Vec::new();
            for field in fields {
                if let Some(val) = field.strip_prefix("cwd:") {
                    cwd = Some(PathBuf::from(unescape(val)?));
                } else if let Some(val) = field.strip_prefix("title:") {
                    title = Some(unescape(val)?);
                } else if let Some(val) = field.strip_prefix("cmd:") {
                    prog = Some(unescape(val)?);
                } else if let Some(val) = field.strip_prefix("arg:") {
                    cmd_args.push(unescape(val)?);
                } else {
                    return Err(format!("unknown field '{field}'"));
                }
            }
            Ok(Request::NewTab {
                cwd,
                title,
                command: prog.map(|p| (p, cmd_args)),
            })
        }
        "split" => {
            let axis = parse_axis(
                fields
                    .next()
                    .ok_or_else(|| "missing split axis".to_string())?,
            )?;
            let mut cwd = None;
            for field in fields {
                if let Some(val) = field.strip_prefix("cwd:") {
                    cwd = Some(PathBuf::from(unescape(val)?));
                } else {
                    return Err(format!("unknown split field '{field}'"));
                }
            }
            Ok(Request::Split { axis, cwd })
        }
        "close-pane" => Ok(Request::ClosePane),
        "close-tab" => Ok(Request::CloseTab),
        "focus" => Ok(Request::Focus(parse_dir(
            fields
                .next()
                .ok_or_else(|| "missing focus direction".to_string())?,
        )?)),
        "resize" => Ok(Request::Resize(parse_dir(
            fields
                .next()
                .ok_or_else(|| "missing resize direction".to_string())?,
        )?)),
        "set-title" => Ok(Request::SetTitle(unescape(
            fields.next().ok_or_else(|| "missing title".to_string())?,
        )?)),
        "set-theme" => Ok(Request::SetTheme(unescape(
            fields.next().ok_or_else(|| "missing theme".to_string())?,
        )?)),
        "open-settings" => Ok(Request::OpenSettings),
        "reload" => Ok(Request::Reload),
        "send-text" => Ok(Request::SendText(unescape(
            fields.next().ok_or_else(|| "missing text".to_string())?,
        )?)),
        "list" => Ok(Request::List),
        "list-json" => Ok(Request::ListJson),
        other => Err(format!("unknown ipc command '{other}'")),
    }
}

fn dir_name(dir: Dir) -> &'static str {
    match dir {
        Dir::Left => "left",
        Dir::Right => "right",
        Dir::Up => "up",
        Dir::Down => "down",
    }
}

fn parse_dir(text: &str) -> Result<Dir, String> {
    match text {
        "left" => Ok(Dir::Left),
        "right" => Ok(Dir::Right),
        "up" => Ok(Dir::Up),
        "down" => Ok(Dir::Down),
        other => Err(format!("unknown direction '{other}'")),
    }
}

fn parse_axis(text: &str) -> Result<Axis, String> {
    match text {
        "right" | "horizontal" | "h" => Ok(Axis::Horizontal),
        "down" | "vertical" | "v" => Ok(Axis::Vertical),
        other => Err(format!("unknown split direction '{other}'")),
    }
}

pub fn parse_cli(args: &[String]) -> Result<(Option<PathBuf>, Request), String> {
    let mut socket = None;
    let mut json = false;
    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.peek() {
        if let Some(path) = arg.strip_prefix("--socket=") {
            socket = Some(PathBuf::from(path));
            iter.next();
        } else if *arg == "--socket" {
            iter.next();
            let path = iter
                .next()
                .ok_or_else(|| "missing value for '--socket'".to_string())?;
            socket = Some(PathBuf::from(path));
        } else if *arg == "--json" {
            json = true;
            iter.next();
        } else {
            break;
        }
    }
    let Some(cmd) = iter.next() else {
        return Err(format!("missing ctl command\n\n{CTL_USAGE}"));
    };
    if json && !matches!(cmd.as_str(), "list" | "-h" | "--help" | "help") {
        return Err("--json is only supported for list".to_string());
    }
    let request = match cmd.as_str() {
        "-h" | "--help" | "help" => return Err(CTL_USAGE.to_string()),
        "new-tab" => {
            let mut cwd = None;
            let mut title = None;
            let mut command = None;
            while let Some(arg) = iter.next() {
                if let Some(val) = arg
                    .strip_prefix("--cwd=")
                    .or_else(|| arg.strip_prefix("--working-directory="))
                {
                    cwd = Some(PathBuf::from(val));
                } else if let Some(val) = arg.strip_prefix("--title=") {
                    title = Some(val.to_string());
                } else {
                    match arg.as_str() {
                        "--cwd" | "--working-directory" => {
                            let val = iter
                                .next()
                                .ok_or_else(|| format!("missing value for '{arg}'"))?;
                            cwd = Some(PathBuf::from(val));
                        }
                        "--title" => {
                            let val = iter
                                .next()
                                .ok_or_else(|| "missing value for '--title'".to_string())?;
                            title = Some(val.clone());
                        }
                        "-e" | "--command" | "--" => {
                            let prog = iter
                                .next()
                                .ok_or_else(|| format!("missing command for '{arg}'"))?;
                            command = Some((prog.clone(), iter.cloned().collect()));
                            break;
                        }
                        other => return Err(format!("unknown new-tab option '{other}'")),
                    }
                }
            }
            Request::NewTab {
                cwd,
                title,
                command,
            }
        }
        "split" => {
            let dir = iter
                .next()
                .ok_or_else(|| "split requires right|down".to_string())?;
            let axis = parse_axis(dir)?;
            let mut cwd = None;
            while let Some(arg) = iter.next() {
                if let Some(val) = arg.strip_prefix("--cwd=") {
                    cwd = Some(PathBuf::from(val));
                } else if arg == "--cwd" {
                    let val = iter
                        .next()
                        .ok_or_else(|| "missing value for '--cwd'".to_string())?;
                    cwd = Some(PathBuf::from(val));
                } else {
                    return Err(format!("unknown split option '{arg}'"));
                }
            }
            Request::Split { axis, cwd }
        }
        "close-pane" => Request::ClosePane,
        "close-tab" => Request::CloseTab,
        "focus" => Request::Focus(parse_dir(
            iter.next()
                .ok_or_else(|| "focus requires a direction".to_string())?,
        )?),
        "resize" => Request::Resize(parse_dir(
            iter.next()
                .ok_or_else(|| "resize requires a direction".to_string())?,
        )?),
        "set-title" => Request::SetTitle(
            iter.next()
                .ok_or_else(|| "set-title requires TEXT".to_string())?
                .clone(),
        ),
        "set-theme" => Request::SetTheme(
            iter.next()
                .ok_or_else(|| "set-theme requires a theme id".to_string())?
                .clone(),
        ),
        "open-settings" => Request::OpenSettings,
        "reload" => Request::Reload,
        "send-text" => Request::SendText(
            iter.next()
                .ok_or_else(|| "send-text requires TEXT".to_string())?
                .clone(),
        ),
        "list" => {
            if let Some(extra) = iter.next() {
                if extra == "--json" && iter.next().is_none() {
                    Request::ListJson
                } else {
                    return Err(format!("unknown list option '{extra}'"));
                }
            } else if json {
                Request::ListJson
            } else {
                Request::List
            }
        }
        other => return Err(format!("unknown ctl command '{other}'")),
    };
    Ok((socket, request))
}

pub fn listen_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    dir.join(format!("koral-{}.sock", std::process::id()))
}

pub fn default_socket_path() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("KORAL_SOCKET").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(explicit));
    }
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)?;
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("koral-") && name.ends_with(".sock"))
        })
        .collect();
    candidates.sort();
    candidates
        .into_iter()
        .rev()
        .find(|path| UnixStream::connect(path).is_ok())
}

pub struct Server {
    path: PathBuf,
}

impl Server {
    pub fn start(proxy: EventLoopProxy<UserEvent>) -> std::io::Result<Self> {
        Self::bind_with(listen_path(), move |request, reply| {
            let _ = proxy.send_event(UserEvent::Ipc(request, reply));
        })
    }

    pub fn bind_with<F>(path: PathBuf, dispatch: F) -> std::io::Result<Self>
    where
        F: Fn(Request, Sender<String>) + Send + 'static,
    {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, Permissions::from_mode(0o600))?;
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else {
                    break;
                };
                handle_client(stream, &dispatch);
            }
        });
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn handle_client<F>(mut stream: UnixStream, dispatch: &F)
where
    F: Fn(Request, Sender<String>),
{
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let Ok(cloned) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(cloned);
    let mut line = String::new();
    let Ok(read) = reader.by_ref().take(MAX_LINE as u64).read_line(&mut line) else {
        return;
    };
    if read == 0 {
        return;
    }
    let response = match decode(&line) {
        Ok(request) => {
            let (tx, rx) = mpsc::channel();
            dispatch(request, tx);
            rx.recv_timeout(IO_TIMEOUT)
                .unwrap_or_else(|_| "ERR timed out\n".to_string())
        }
        Err(error) => format!("ERR {error}\n"),
    };
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

pub fn send(socket: Option<PathBuf>, request: &Request) -> Result<String, String> {
    let path = socket
        .or_else(default_socket_path)
        .ok_or_else(|| "no running koral socket found".to_string())?;
    let mut stream = UnixStream::connect(&path)
        .map_err(|e| format!("cannot connect to {}: {e}", path.display()))?;
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    stream
        .write_all(encode(request).as_bytes())
        .map_err(|e| format!("write error: {e}"))?;
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut reply = String::new();
    stream
        .read_to_string(&mut reply)
        .map_err(|e| format!("read error: {e}"))?;
    if let Some(body) = reply.strip_prefix("OK\n") {
        Ok(body.trim_end_matches('\n').to_string())
    } else if reply.trim() == "OK" {
        Ok(String::new())
    } else if let Some(err) = reply.strip_prefix("ERR ") {
        Err(err.trim().to_string())
    } else {
        Err(format!("unexpected reply: {}", reply.trim()))
    }
}
