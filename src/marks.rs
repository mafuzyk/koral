const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const GRAPHICS: &[u8] = b"\x1b_G";
const PREFIXES: [&[u8]; 5] = [b"\x1b]133;", b"\x1b]7;", b"\x1b]777;notify;", b"\x1b]9;", GRAPHICS];
// The stream is untrusted: an unterminated mark must not grow without bound.
const MAX_MARK: usize = 64 * 1024;
// Programs send images in 4096-byte chunks, this leaves room for the ones that do not.
const MAX_GRAPHICS: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mark {
    PromptStart,
    PromptEnd,
    CommandStart(Option<String>),
    CommandEnd(Option<i32>),
    Cwd(String),
    Notify { title: String, body: String },
}

#[derive(Debug, PartialEq, Eq)]
pub enum Piece<'a> {
    Bytes(&'a [u8]),
    Mark(Mark),
    Graphics(&'a [u8]),
}

enum Step {
    Keep,
    Release,
    Found(Mark),
    Graphics,
}

/// Takes OSC 133, OSC 7, notifications and image commands out of a PTY stream; vte would drop them.
// shortcut: marks inside DCS or tmux passthrough also match, harmless for now.
#[derive(Default)]
pub struct Scanner {
    pending: Vec<u8>,
}

impl Scanner {
    pub fn feed(&mut self, input: &[u8], sink: &mut impl FnMut(Piece<'_>)) {
        let mut rest = input;
        while !rest.is_empty() {
            if self.pending.is_empty() {
                let Some(start) = rest.iter().position(|byte| *byte == ESC) else {
                    sink(Piece::Bytes(rest));
                    return;
                };
                if start > 0 {
                    sink(Piece::Bytes(&rest[..start]));
                }
                self.pending.push(ESC);
                rest = &rest[start + 1..];
                continue;
            }

            let byte = rest[0];
            rest = &rest[1..];
            self.pending.push(byte);
            match self.step() {
                Step::Keep => {}
                Step::Release => {
                    sink(Piece::Bytes(&self.pending));
                    self.pending.clear();
                }
                Step::Found(mark) => {
                    sink(Piece::Mark(mark));
                    self.pending.clear();
                }
                Step::Graphics => {
                    let end = self.pending.len() - 2;
                    sink(Piece::Graphics(&self.pending[2..end]));
                    self.pending.clear();
                }
            }
        }
    }

    fn step(&self) -> Step {
        let seen = self.pending.as_slice();
        if !PREFIXES.iter().any(|prefix| seen.starts_with(prefix)) {
            return if PREFIXES.iter().any(|prefix| prefix.starts_with(seen)) {
                Step::Keep
            } else {
                Step::Release
            };
        }

        let byte = seen[seen.len() - 1];
        let after_escape = seen[seen.len() - 2] == ESC;
        if seen.starts_with(GRAPHICS) {
            return match byte {
                b'\\' if after_escape => Step::Graphics,
                _ if after_escape || seen.len() >= MAX_GRAPHICS => Step::Release,
                _ => Step::Keep,
            };
        }
        let body = match byte {
            BEL => &seen[2..seen.len() - 1],
            b'\\' if after_escape => &seen[2..seen.len() - 2],
            _ if after_escape || (byte < 0x20 && byte != ESC) || seen.len() >= MAX_MARK => {
                return Step::Release;
            }
            _ => return Step::Keep,
        };
        parse(body).map_or(Step::Release, Step::Found)
    }
}

fn parse(body: &[u8]) -> Option<Mark> {
    let body = std::str::from_utf8(body).ok()?;
    if let Some(url) = body.strip_prefix("7;") {
        let (_host, path) = url.strip_prefix("file://")?.split_once('/')?;
        return Some(Mark::Cwd(format!("/{}", percent_decode(path)?)));
    }

    if let Some(rest) = body.strip_prefix("777;notify;") {
        let (title, body) = rest.split_once(';')?;
        return Some(Mark::Notify { title: title.to_string(), body: body.to_string() });
    }
    if let Some(text) = body.strip_prefix("9;") {
        // ConEmu progress is digits then a semicolon, and stays with vte.
        let digits = text.chars().take_while(char::is_ascii_digit).count();
        let progress = digits > 0 && text[digits..].starts_with(';');
        return (!progress).then(|| Mark::Notify { title: String::new(), body: text.to_string() });
    }

    let mut params = body.strip_prefix("133;")?.split(';');
    match params.next()? {
        "A" => Some(Mark::PromptStart),
        "B" => Some(Mark::PromptEnd),
        "C" => Some(Mark::CommandStart(
            params
                .find_map(|param| param.strip_prefix("cmdline_url="))
                .and_then(percent_decode),
        )),
        "D" => Some(Mark::CommandEnd(params.next().and_then(|status| status.parse().ok()))),
        _ => None,
    }
}

fn percent_decode(text: &str) -> Option<String> {
    let mut decoded = Vec::with_capacity(text.len());
    let mut bytes = text.bytes();
    while let Some(byte) = bytes.next() {
        decoded.push(if byte == b'%' {
            let high = (bytes.next()? as char).to_digit(16)?;
            let low = (bytes.next()? as char).to_digit(16)?;
            (high * 16 + low) as u8
        } else {
            byte
        });
    }
    String::from_utf8(decoded).ok()
}
