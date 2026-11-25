use std::fs::Permissions;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
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
    ClosePane {
        pane: Option<u64>,
    },
    CloseTab,
    Focus(Dir),
    Resize(Dir),
    SetTitle {
        pane: Option<u64>,
        title: String,
    },
    SetTheme(String),
    OpenSettings,
    Reload,
    SendText {
        pane: Option<u64>,
        text: String,
    },
    List,
    ListJson,
    ListBlocks {
        pane: Option<u64>,
        json: bool,
    },
    GetBlockOutput {
        pane: Option<u64>,
        selector: BlockSelector,
        json: bool,
    },
    Watch {
        pane: Option<u64>,
    },
    SaveSession(String),
    RestoreSession(String),
    ListSessions,
    ListSessionsJson,
}

pub const CTL_USAGE: &str = "\
Usage: koral ctl [--socket PATH] [--json] COMMAND [ARGS...]

Commands:
  new-tab [--cwd PATH] [--title TEXT] [-e|-- COMMAND [ARGS...]]
  split right|down|horizontal|vertical [--cwd PATH]
  close-pane [--pane ID]
  close-tab
  focus left|right|up|down
  resize left|right|up|down
  set-title [--pane ID] TEXT
  set-theme ID
  open-settings
  reload
  send-text [--pane ID] TEXT
  list [--json]
  list-blocks [--pane ID] [--json]
  get-block-output [--pane ID] [--block SEL] [--json]
  watch [--pane ID] [--json]
  save-session NAME
  restore-session NAME
  list-sessions [--json]
";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlockSelector {
    #[default]
    Last,
    Offset(usize),
    LastFailed,
    Id(u32),
}

pub fn parse_block_selector(spec: &str) -> Result<BlockSelector, String> {
    let raw = spec.strip_prefix('@').unwrap_or(spec).trim();
    if raw.is_empty() {
        return Err("empty block selector".to_string());
    }
    if raw.eq_ignore_ascii_case("last") {
        return Ok(BlockSelector::Last);
    }
    if raw.eq_ignore_ascii_case("err")
        || raw.eq_ignore_ascii_case("error")
        || raw.eq_ignore_ascii_case("failed")
    {
        return Ok(BlockSelector::LastFailed);
    }
    if let Some(rest) = raw.strip_prefix('-') {
        let offset = rest
            .parse::<usize>()
            .ok()
            .filter(|n| *n >= 1)
            .ok_or_else(|| format!("invalid relative block selector '{spec}'"))?;
        return Ok(BlockSelector::Offset(offset));
    }
    let id = raw
        .parse::<u32>()
        .map_err(|_| format!("invalid block selector '{spec}'"))?;
    Ok(BlockSelector::Id(id))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockEvent {
    pub pane_id: u64,
    pub kind: String,
    pub block_id: Option<u32>,
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub status: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockInfo {
    pub id: u32,
    pub pane_id: u64,
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub status: Option<i32>,
    pub folded: bool,
}

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

pub fn render_blocks_text(blocks: &[BlockInfo]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for b in blocks {
        let status = b
            .status
            .map_or_else(|| "-".to_string(), |code| code.to_string());
        let _ = write!(
            out,
            "block {} pane={} status={} folded={}",
            b.id, b.pane_id, status, b.folded
        );
        if let Some(cwd) = &b.cwd {
            let _ = write!(out, " cwd={cwd}");
        }
        if let Some(cmd) = &b.command {
            let _ = write!(out, " cmd={cmd}");
        }
        out.push('\n');
    }
    out
}

pub fn render_blocks_json(blocks: &[BlockInfo]) -> String {
    use std::fmt::Write;
    let mut out = String::from("{\"version\":1,\"blocks\":[");
    for (idx, b) in blocks.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        let cmd = b
            .command
            .as_deref()
            .map_or_else(|| "null".to_string(), json_string);
        let cwd = b
            .cwd
            .as_deref()
            .map_or_else(|| "null".to_string(), json_string);
        let status = b
            .status
            .map_or_else(|| "null".to_string(), |code| code.to_string());
        let _ = write!(
            out,
            "{{\"id\":{},\"pane_id\":{},\"command\":{cmd},\"cwd\":{cwd},\"status\":{status},\"folded\":{}}}",
            b.id, b.pane_id, b.folded
        );
    }
    out.push_str("]}");
    out
}

pub fn render_block_output_json(pane_id: u64, block_id: u32, output: &str) -> String {
    format!(
        "{{\"version\":1,\"pane_id\":{pane_id},\"block_id\":{block_id},\"output\":{}}}",
        json_string(output)
    )
}

pub fn render_event_ndjson(event: &BlockEvent) -> String {
    use std::fmt::Write;
    let block_id = event
        .block_id
        .map_or_else(|| "null".to_string(), |id| id.to_string());
    let command = event
        .command
        .as_deref()
        .map_or_else(|| "null".to_string(), json_string);
    let cwd = event
        .cwd
        .as_deref()
        .map_or_else(|| "null".to_string(), json_string);
    let status = event
        .status
        .map_or_else(|| "null".to_string(), |code| code.to_string());
    let mut out = String::new();
    let _ = write!(
        out,
        "{{\"version\":1,\"event\":{},\"pane_id\":{},\"block_id\":{block_id},\"command\":{command},\"cwd\":{cwd},\"status\":{status}}}",
        json_string(&event.kind),
        event.pane_id
    );
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
        Request::ClosePane { pane } => {
            parts.push("close-pane".to_string());
            if let Some(id) = pane {
                parts.push(format!("pane:{id}"));
            }
        }
        Request::CloseTab => parts.push("close-tab".to_string()),
        Request::Focus(dir) => {
            parts.push("focus".to_string());
            parts.push(dir_name(*dir).to_string());
        }
        Request::Resize(dir) => {
            parts.push("resize".to_string());
            parts.push(dir_name(*dir).to_string());
        }
        Request::SetTitle { pane, title } => {
            parts.push("set-title".to_string());
            if let Some(id) = pane {
                parts.push(format!("pane:{id}"));
            }
            parts.push(escape(title));
        }
        Request::SetTheme(id) => {
            parts.push("set-theme".to_string());
            parts.push(escape(id));
        }
        Request::OpenSettings => parts.push("open-settings".to_string()),
        Request::Reload => parts.push("reload".to_string()),
        Request::SendText { pane, text } => {
            parts.push("send-text".to_string());
            if let Some(id) = pane {
                parts.push(format!("pane:{id}"));
            }
            parts.push(escape(text));
        }
        Request::List => parts.push("list".to_string()),
        Request::ListJson => parts.push("list-json".to_string()),
        Request::ListBlocks { pane, json } => {
            parts.push(if *json {
                "list-blocks-json".to_string()
            } else {
                "list-blocks".to_string()
            });
            if let Some(id) = pane {
                parts.push(format!("pane:{id}"));
            }
        }
        Request::GetBlockOutput {
            pane,
            selector,
            json,
        } => {
            parts.push(if *json {
                "get-block-output-json".to_string()
            } else {
                "get-block-output".to_string()
            });
            if let Some(id) = pane {
                parts.push(format!("pane:{id}"));
            }
            match selector {
                BlockSelector::Last => {}
                BlockSelector::Offset(n) => parts.push(format!("block:offset:{n}")),
                BlockSelector::LastFailed => parts.push("block:err".to_string()),
                BlockSelector::Id(bid) => parts.push(format!("block:{bid}")),
            }
        }
        Request::Watch { pane } => {
            parts.push("watch".to_string());
            if let Some(id) = pane {
                parts.push(format!("pane:{id}"));
            }
        }
        Request::SaveSession(name) => {
            parts.push("save-session".to_string());
            parts.push(escape(name));
        }
        Request::RestoreSession(name) => {
            parts.push("restore-session".to_string());
            parts.push(escape(name));
        }
        Request::ListSessions => parts.push("list-sessions".to_string()),
        Request::ListSessionsJson => parts.push("list-sessions-json".to_string()),
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
        "close-pane" => Ok(Request::ClosePane {
            pane: parse_optional_pane_field(fields.next())?,
        }),
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
        "set-title" => {
            let (pane, title) = parse_pane_and_value(fields, "missing title")?;
            Ok(Request::SetTitle { pane, title })
        }
        "set-theme" => Ok(Request::SetTheme(unescape(
            fields.next().ok_or_else(|| "missing theme".to_string())?,
        )?)),
        "open-settings" => Ok(Request::OpenSettings),
        "reload" => Ok(Request::Reload),
        "send-text" => {
            let (pane, text) = parse_pane_and_value(fields, "missing text")?;
            Ok(Request::SendText { pane, text })
        }
        "list" => Ok(Request::List),
        "list-json" => Ok(Request::ListJson),
        "list-blocks" => Ok(Request::ListBlocks {
            pane: parse_optional_pane_field(fields.next())?,
            json: false,
        }),
        "list-blocks-json" => Ok(Request::ListBlocks {
            pane: parse_optional_pane_field(fields.next())?,
            json: true,
        }),
        "get-block-output" | "get-block-output-json" => {
            let mut pane = None;
            let mut selector = BlockSelector::Last;
            for field in fields {
                if let Some(id) = field.strip_prefix("pane:") {
                    pane = Some(
                        id.parse::<u64>()
                            .map_err(|_| format!("invalid pane id '{id}'"))?,
                    );
                } else if let Some(sel) = field.strip_prefix("block:") {
                    selector = if sel == "err" {
                        BlockSelector::LastFailed
                    } else if let Some(off) = sel.strip_prefix("offset:") {
                        let n = off
                            .parse::<usize>()
                            .ok()
                            .filter(|n| *n >= 1)
                            .ok_or_else(|| format!("invalid block offset '{off}'"))?;
                        BlockSelector::Offset(n)
                    } else {
                        BlockSelector::Id(
                            sel.parse::<u32>()
                                .map_err(|_| format!("invalid block id '{sel}'"))?,
                        )
                    };
                } else {
                    return Err(format!("unknown field '{field}'"));
                }
            }
            Ok(Request::GetBlockOutput {
                pane,
                selector,
                json: cmd == "get-block-output-json",
            })
        }
        "watch" => Ok(Request::Watch {
            pane: parse_optional_pane_field(fields.next())?,
        }),
        "save-session" => Ok(Request::SaveSession(unescape(
            fields
                .next()
                .ok_or_else(|| "missing session name".to_string())?,
        )?)),
        "restore-session" => Ok(Request::RestoreSession(unescape(
            fields
                .next()
                .ok_or_else(|| "missing session name".to_string())?,
        )?)),
        "list-sessions" => Ok(Request::ListSessions),
        "list-sessions-json" => Ok(Request::ListSessionsJson),
        other => Err(format!("unknown ipc command '{other}'")),
    }
}

fn parse_optional_pane_field(field: Option<&str>) -> Result<Option<u64>, String> {
    let Some(field) = field else {
        return Ok(None);
    };
    let id = field
        .strip_prefix("pane:")
        .ok_or_else(|| format!("unknown field '{field}'"))?;
    id.parse::<u64>()
        .map(Some)
        .map_err(|_| format!("invalid pane id '{id}'"))
}

fn parse_pane_and_value<'a>(
    mut fields: impl Iterator<Item = &'a str>,
    missing: &str,
) -> Result<(Option<u64>, String), String> {
    let first = fields.next().ok_or_else(|| missing.to_string())?;
    if let Some(id_str) = first.strip_prefix("pane:")
        && let Some(val) = fields.next()
    {
        let id = id_str
            .parse::<u64>()
            .map_err(|_| format!("invalid pane id '{id_str}'"))?;
        return Ok((Some(id), unescape(val)?));
    }
    Ok((None, unescape(first)?))
}

fn take_pane_flag<'a>(
    arg: &str,
    iter: &mut impl Iterator<Item = &'a String>,
    pane: &mut Option<u64>,
) -> Result<bool, String> {
    if let Some(val) = arg.strip_prefix("--pane=") {
        let id = val
            .parse::<u64>()
            .map_err(|_| format!("invalid pane id '{val}'"))?;
        *pane = Some(id);
        Ok(true)
    } else if arg == "--pane" {
        let val = iter
            .next()
            .ok_or_else(|| "missing value for '--pane'".to_string())?;
        let id = val
            .parse::<u64>()
            .map_err(|_| format!("invalid pane id '{val}'"))?;
        *pane = Some(id);
        Ok(true)
    } else {
        Ok(false)
    }
}

fn take_pane_and_arg<'a>(
    iter: &mut impl Iterator<Item = &'a String>,
    missing: &str,
) -> Result<(Option<u64>, String), String> {
    let mut pane = None;
    while let Some(arg) = iter.next() {
        if arg == "--" {
            let val = iter.next().ok_or_else(|| missing.to_string())?;
            return Ok((pane, val.clone()));
        }
        if take_pane_flag(arg, iter, &mut pane)? {
            continue;
        }
        return Ok((pane, arg.clone()));
    }
    Err(missing.to_string())
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
    if json
        && !matches!(
            cmd.as_str(),
            "list"
                | "list-sessions"
                | "list-blocks"
                | "get-block-output"
                | "watch"
                | "-h"
                | "--help"
                | "help"
        )
    {
        return Err(format!("--json is not supported for '{cmd}'"));
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
        "close-pane" => {
            let mut pane = None;
            while let Some(arg) = iter.next() {
                if !take_pane_flag(arg, &mut iter, &mut pane)? {
                    return Err(format!("unknown close-pane option '{arg}'"));
                }
            }
            Request::ClosePane { pane }
        }
        "close-tab" => Request::CloseTab,
        "focus" => Request::Focus(parse_dir(
            iter.next()
                .ok_or_else(|| "focus requires a direction".to_string())?,
        )?),
        "resize" => Request::Resize(parse_dir(
            iter.next()
                .ok_or_else(|| "resize requires a direction".to_string())?,
        )?),
        "set-title" => {
            let (pane, title) = take_pane_and_arg(&mut iter, "set-title requires TEXT")?;
            Request::SetTitle { pane, title }
        }
        "set-theme" => Request::SetTheme(
            iter.next()
                .ok_or_else(|| "set-theme requires a theme id".to_string())?
                .clone(),
        ),
        "open-settings" => Request::OpenSettings,
        "reload" => Request::Reload,
        "send-text" => {
            let (pane, text) = take_pane_and_arg(&mut iter, "send-text requires TEXT")?;
            Request::SendText { pane, text }
        }
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
        "list-blocks" => {
            let mut pane = None;
            let mut flag_json = json;
            while let Some(arg) = iter.next() {
                if arg == "--json" {
                    flag_json = true;
                } else if !take_pane_flag(arg, &mut iter, &mut pane)? {
                    return Err(format!("unknown list-blocks option '{arg}'"));
                }
            }
            Request::ListBlocks {
                pane,
                json: flag_json,
            }
        }
        "get-block-output" => {
            let mut pane = None;
            let mut selector = BlockSelector::Last;
            let mut flag_json = json;
            while let Some(arg) = iter.next() {
                if arg == "--json" {
                    flag_json = true;
                } else if let Some(val) = arg.strip_prefix("--block=") {
                    selector = parse_block_selector(val)?;
                } else if arg == "--block" {
                    let val = iter
                        .next()
                        .ok_or_else(|| "missing value for '--block'".to_string())?;
                    selector = parse_block_selector(val)?;
                } else if !take_pane_flag(arg, &mut iter, &mut pane)? {
                    return Err(format!("unknown get-block-output option '{arg}'"));
                }
            }
            Request::GetBlockOutput {
                pane,
                selector,
                json: flag_json,
            }
        }
        "watch" => {
            let mut pane = None;
            while let Some(arg) = iter.next() {
                if arg == "--json" {
                    continue;
                } else if !take_pane_flag(arg, &mut iter, &mut pane)? {
                    return Err(format!("unknown watch option '{arg}'"));
                }
            }
            Request::Watch { pane }
        }
        "save-session" => {
            let name = iter
                .next()
                .ok_or_else(|| "save-session requires NAME".to_string())?
                .clone();
            if let Some(extra) = iter.next() {
                return Err(format!("unexpected save-session argument '{extra}'"));
            }
            crate::workspace::validate_name(&name)?;
            Request::SaveSession(name)
        }
        "restore-session" => {
            let name = iter
                .next()
                .ok_or_else(|| "restore-session requires NAME".to_string())?
                .clone();
            if let Some(extra) = iter.next() {
                return Err(format!("unexpected restore-session argument '{extra}'"));
            }
            crate::workspace::validate_name(&name)?;
            Request::RestoreSession(name)
        }
        "list-sessions" => {
            if let Some(extra) = iter.next() {
                if extra == "--json" && iter.next().is_none() {
                    Request::ListSessionsJson
                } else {
                    return Err(format!("unknown list-sessions option '{extra}'"));
                }
            } else if json {
                Request::ListSessionsJson
            } else {
                Request::ListSessions
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

type WatcherList = Arc<Mutex<Vec<(Option<u64>, Sender<String>)>>>;

pub struct Server {
    path: PathBuf,
    watchers: WatcherList,
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
        let watchers: WatcherList = Arc::new(Mutex::new(Vec::new()));
        let watchers_bg = watchers.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else {
                    break;
                };
                handle_client(stream, &dispatch, &watchers_bg);
            }
        });
        Ok(Self { path, watchers })
    }

    pub fn publish(&self, pane_id: u64, ndjson_line: &str) -> usize {
        let Ok(mut list) = self.watchers.lock() else {
            return 0;
        };
        let mut delivered = 0usize;
        list.retain(|(filter, tx)| {
            if filter.is_none_or(|target| target == pane_id) {
                let ok = tx.send(ndjson_line.to_string()).is_ok();
                if ok {
                    delivered += 1;
                }
                ok
            } else {
                true
            }
        });
        delivered
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Ok(mut list) = self.watchers.lock() {
            list.clear();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(target_os = "linux")]
pub fn verify_peer(stream: &UnixStream) -> bool {
    use std::os::unix::io::AsRawFd;
    const SOL_SOCKET: i32 = 1;
    const SO_PEERCRED: i32 = 17;
    #[repr(C)]
    struct UCred {
        pid: i32,
        uid: u32,
        gid: u32,
    }
    unsafe extern "C" {
        fn geteuid() -> u32;
        fn getsockopt(
            sockfd: i32,
            level: i32,
            optname: i32,
            optval: *mut UCred,
            optlen: *mut u32,
        ) -> i32;
    }
    let mut cred = UCred {
        pid: 0,
        uid: u32::MAX,
        gid: u32::MAX,
    };
    let mut len = std::mem::size_of::<UCred>() as u32;
    // SAFETY: `cred` matches kernel `struct ucred` and `len` is initialized to its byte size.
    let rc = unsafe {
        getsockopt(
            stream.as_raw_fd(),
            SOL_SOCKET,
            SO_PEERCRED,
            &mut cred,
            &mut len,
        )
    };
    // SAFETY: `geteuid()` has no preconditions and returns the effective UID of this process.
    rc == 0 && cred.uid == unsafe { geteuid() }
}

#[cfg(not(target_os = "linux"))]
pub fn verify_peer(_stream: &UnixStream) -> bool {
    true
}

fn handle_client<F>(mut stream: UnixStream, dispatch: &F, watchers: &WatcherList)
where
    F: Fn(Request, Sender<String>),
{
    if !verify_peer(&stream) {
        let _ = stream.write_all(b"ERR unauthorized\n");
        let _ = stream.flush();
        return;
    }
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
    match decode(&line) {
        Ok(Request::Watch { pane }) => {
            let watchers = watchers.clone();
            std::thread::spawn(move || {
                let (tx, rx) = mpsc::channel::<String>();
                if let Ok(mut list) = watchers.lock() {
                    list.push((pane, tx));
                }
                if stream.write_all(b"OK\n").is_err() || stream.flush().is_err() {
                    return;
                }
                loop {
                    match rx.recv_timeout(Duration::from_millis(500)) {
                        Ok(event_line) => {
                            if stream.write_all(event_line.as_bytes()).is_err()
                                || stream.write_all(b"\n").is_err()
                                || stream.flush().is_err()
                            {
                                break;
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => continue,
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            });
        }
        Ok(request) => {
            let (tx, rx) = mpsc::channel();
            dispatch(request, tx);
            let response = rx
                .recv_timeout(IO_TIMEOUT)
                .unwrap_or_else(|_| "ERR timed out\n".to_string());
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
        Err(error) => {
            let response = format!("ERR {error}\n");
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    }
}

pub fn watch(
    socket: Option<PathBuf>,
    request: &Request,
    mut on_line: impl FnMut(&str),
) -> Result<(), String> {
    let path = socket
        .or_else(default_socket_path)
        .ok_or_else(|| "no running koral socket found".to_string())?;
    let mut stream = UnixStream::connect(&path)
        .map_err(|e| format!("cannot connect to {}: {e}", path.display()))?;
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    stream
        .write_all(encode(request).as_bytes())
        .map_err(|e| format!("write error: {e}"))?;
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let _ = stream.set_read_timeout(None);
    let mut reader = BufReader::new(stream);
    let mut header = String::new();
    let n = reader
        .read_line(&mut header)
        .map_err(|e| format!("read error: {e}"))?;
    if n == 0 {
        return Err("connection closed before acknowledgement".to_string());
    }
    if header != "OK\n" {
        return if let Some(err) = header.strip_prefix("ERR ") {
            Err(err.trim().to_string())
        } else {
            Err(format!("unexpected reply: {}", header.trim()))
        };
    }
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return Ok(()),
            Ok(_) => on_line(line.trim_end_matches(['\r', '\n'])),
            Err(e) => return Err(format!("stream read error: {e}")),
        }
    }
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
