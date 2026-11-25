use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use crate::marks::Mark;

pub const MAX_BLOCKS: usize = 20_000;
// Ids become private-use characters in the grid, so they have to stay small.
const ID_RANGE: u32 = 0x8000;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Block {
    pub id: u32,
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub status: Option<i32>,
    pub duration: Option<Duration>,
    pub folded: bool,
    /// Rows the prompt takes above the line the anchor sits on.
    pub prompt_lines: usize,
    /// Line feeds the prompt printed. A reflow changes the rows, never these.
    pub prompt_breaks: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Idle,
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    Prompt,
    Input,
    Running,
}

/// Commands seen on one terminal. Any program can print marks, so a mark only
/// counts as a valid transition: while a command runs, only its end does.
#[derive(Default)]
pub struct BlockStore {
    blocks: HashMap<u32, Block>,
    order: VecDeque<u32>,
    phase: Phase,
    cwd: Option<String>,
    started: Option<Instant>,
    next_id: u32,
    /// Monotonic count of changes that can alter projected rows.
    revision: u64,
    /// Test hook: bytes actually looked at by `observe`.
    bytes_scanned: usize,
}

impl BlockStore {
    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn in_prompt(&self) -> bool {
        self.phase == Phase::Prompt
    }

    /// Test-only diagnostic: counts bytes scanned while in the prompt phase.
    pub fn bytes_scanned(&self) -> usize {
        self.bytes_scanned
    }

    /// Returns a block id when its anchor has to go on the cursor line.
    pub fn apply(&mut self, mark: Mark, now: Instant) -> Option<u32> {
        match (self.phase, mark) {
            (Phase::Idle | Phase::Prompt | Phase::Input, Mark::PromptStart) => {
                if self.phase != Phase::Idle {
                    self.drop_latest();
                }
                self.start_block();
                None
            }
            (Phase::Prompt | Phase::Input, Mark::PromptEnd) => {
                self.phase = Phase::Input;
                self.order.back().copied()
            }
            (Phase::Input, Mark::CommandStart(command)) => {
                self.phase = Phase::Running;
                self.started = Some(now);
                let latest = self.latest_mut()?;
                if latest.command != command {
                    latest.command = command;
                    self.changed();
                }
                None
            }
            (Phase::Input | Phase::Running, Mark::CommandEnd(status)) => {
                let duration = self.started.take().map(|started| now - started);
                self.phase = Phase::Idle;
                let block = self.latest_mut()?;
                let previous = block.status;
                block.status = status;
                block.duration = duration;
                if previous != status {
                    self.changed();
                }
                None
            }
            (Phase::Idle | Phase::Prompt | Phase::Input, Mark::Cwd(path)) => {
                self.cwd = Some(path);
                None
            }
            _ => None,
        }
    }

    /// Line feeds count where the grid cannot: at the bottom of a full scrollback
    /// a prompt moves no cursor line and adds no history. The rows they need are
    /// read off the grid, which is what a reflow rewraps.
    pub fn observe(&mut self, bytes: &[u8]) {
        if !self.in_prompt() || self.order.is_empty() {
            return;
        }
        self.bytes_scanned += bytes.len();
        if let Some(block) = self.latest_mut() {
            let breaks = bytes.iter().filter(|byte| **byte == b'\n').count();
            block.prompt_breaks += breaks;
            if breaks > 0 {
                self.changed();
            }
        }
    }

    pub fn set_prompt_lines(&mut self, id: u32, lines: usize) {
        if let Some(block) = self.blocks.get_mut(&id)
            && block.prompt_lines != lines
        {
            block.prompt_lines = lines;
            self.changed();
        }
    }

    pub fn get(&self, id: u32) -> Option<&Block> {
        self.blocks.get(&id)
    }

    pub fn latest(&self) -> Option<&Block> {
        self.get(*self.order.back()?)
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &Block> {
        self.order.iter().filter_map(|id| self.blocks.get(id))
    }

    pub fn toggle_fold(&mut self, id: u32) {
        if let Some(block) = self.blocks.get_mut(&id) {
            block.folded = !block.folded;
            self.changed();
        }
    }

    pub fn any_folded(&self) -> bool {
        self.blocks.values().any(|block| block.folded)
    }

    pub fn activity(&self) -> Activity {
        if self.phase == Phase::Running {
            return Activity::Running;
        }
        let finished = self
            .order
            .iter()
            .rev()
            .find_map(|id| self.blocks[id].status);
        match finished {
            None => Activity::Idle,
            Some(0) => Activity::Succeeded,
            Some(_) => Activity::Failed,
        }
    }

    fn start_block(&mut self) {
        let id = self.next_id;
        self.next_id = (id + 1) % ID_RANGE;
        self.blocks.insert(
            id,
            Block {
                id,
                cwd: self.cwd.clone(),
                ..Block::default()
            },
        );
        self.order.push_back(id);
        self.phase = Phase::Prompt;
        self.changed();

        if self.order.len() > MAX_BLOCKS
            && let Some(oldest) = self.order.pop_front()
        {
            self.blocks.remove(&oldest);
        }
    }

    fn drop_latest(&mut self) {
        if let Some(id) = self.order.pop_back() {
            self.blocks.remove(&id);
            self.changed();
        }
    }

    fn latest_mut(&mut self) -> Option<&mut Block> {
        self.blocks.get_mut(self.order.back()?)
    }
}
