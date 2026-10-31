use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use crate::marks::Mark;

pub const MAX_BLOCKS: usize = 20_000;
// Ids become private-use characters in the grid, so they have to stay small.
const ID_RANGE: u32 = 0xFFFE;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Block {
    pub id: u32,
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub status: Option<i32>,
    pub duration: Option<Duration>,
    pub folded: bool,
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
}

impl BlockStore {
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
                self.latest_mut()?.command = command;
                None
            }
            (Phase::Input | Phase::Running, Mark::CommandEnd(status)) => {
                let duration = self.started.take().map(|started| now - started);
                self.phase = Phase::Idle;
                let block = self.latest_mut()?;
                block.status = status;
                block.duration = duration;
                None
            }
            (Phase::Idle | Phase::Prompt | Phase::Input, Mark::Cwd(path)) => {
                self.cwd = Some(path);
                None
            }
            _ => None,
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

    pub fn toggle_fold(&mut self, id: u32) {
        if let Some(block) = self.blocks.get_mut(&id) {
            block.folded = !block.folded;
        }
    }

    pub fn activity(&self) -> Activity {
        if self.phase == Phase::Running {
            return Activity::Running;
        }
        let finished = self.order.iter().rev().find_map(|id| self.blocks[id].status);
        match finished {
            None => Activity::Idle,
            Some(0) => Activity::Succeeded,
            Some(_) => Activity::Failed,
        }
    }

    fn start_block(&mut self) {
        let id = self.next_id;
        self.next_id = (id + 1) % ID_RANGE;
        self.blocks.insert(id, Block { id, cwd: self.cwd.clone(), ..Block::default() });
        self.order.push_back(id);
        self.phase = Phase::Prompt;

        if self.order.len() > MAX_BLOCKS
            && let Some(oldest) = self.order.pop_front()
        {
            self.blocks.remove(&oldest);
        }
    }

    fn drop_latest(&mut self) {
        if let Some(id) = self.order.pop_back() {
            self.blocks.remove(&id);
        }
    }

    fn latest_mut(&mut self) -> Option<&mut Block> {
        self.blocks.get_mut(self.order.back()?)
    }
}
