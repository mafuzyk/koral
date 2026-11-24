use crate::tabs::Rect;

const MIN_RATIO: f32 = 0.15;
const MAX_RATIO: f32 = 0.85;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Side-by-side panes (left and right), separated by a vertical divider.
    Horizontal,
    /// Stacked panes (top and bottom), separated by a horizontal divider.
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

impl Dir {
    pub fn axis(self) -> Axis {
        match self {
            Self::Left | Self::Right => Axis::Horizontal,
            Self::Up | Self::Down => Axis::Vertical,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseResult {
    Closed,
    LastPane,
    NotFound,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Divider {
    pub path: Vec<bool>,
    pub axis: Axis,
    pub rect: Rect,
    pub parent: Rect,
}

#[derive(Debug)]
enum Node<T> {
    Leaf {
        id: u64,
        item: T,
    },
    Split {
        axis: Axis,
        ratio: f32,
        first: Box<Node<T>>,
        second: Box<Node<T>>,
    },
}

impl<T> Node<T> {
    fn len(&self) -> usize {
        match self {
            Self::Leaf { .. } => 1,
            Self::Split { first, second, .. } => first.len() + second.len(),
        }
    }

    fn contains(&self, target: u64) -> bool {
        match self {
            Self::Leaf { id, .. } => *id == target,
            Self::Split { first, second, .. } => first.contains(target) || second.contains(target),
        }
    }

    fn first_id(&self) -> u64 {
        match self {
            Self::Leaf { id, .. } => *id,
            Self::Split { first, .. } => first.first_id(),
        }
    }

    fn get(&self, target: u64) -> Option<&T> {
        match self {
            Self::Leaf { id, item } if *id == target => Some(item),
            Self::Leaf { .. } => None,
            Self::Split { first, second, .. } => first.get(target).or_else(|| second.get(target)),
        }
    }

    fn get_mut(&mut self, target: u64) -> Option<&mut T> {
        match self {
            Self::Leaf { id, item } if *id == target => Some(item),
            Self::Leaf { .. } => None,
            Self::Split { first, second, .. } => {
                first.get_mut(target).or_else(|| second.get_mut(target))
            }
        }
    }

    fn collect<'a>(&'a self, out: &mut Vec<(u64, &'a T)>) {
        match self {
            Self::Leaf { id, item } => out.push((*id, item)),
            Self::Split { first, second, .. } => {
                first.collect(out);
                second.collect(out);
            }
        }
    }

    fn collect_mut<'a>(&'a mut self, out: &mut Vec<(u64, &'a mut T)>) {
        match self {
            Self::Leaf { id, item } => out.push((*id, item)),
            Self::Split { first, second, .. } => {
                first.collect_mut(out);
                second.collect_mut(out);
            }
        }
    }

    fn layout(
        &self,
        area: Rect,
        divider: f32,
        path: &mut Vec<bool>,
        panes: &mut Vec<(u64, Rect)>,
        dividers: &mut Vec<Divider>,
    ) {
        match self {
            Self::Leaf { id, .. } => panes.push((*id, area)),
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let (first_rect, div_rect, second_rect) = split_rect(area, *axis, *ratio, divider);
                dividers.push(Divider {
                    path: path.clone(),
                    axis: *axis,
                    rect: div_rect,
                    parent: area,
                });
                path.push(false);
                first.layout(first_rect, divider, path, panes, dividers);
                path.pop();
                path.push(true);
                second.layout(second_rect, divider, path, panes, dividers);
                path.pop();
            }
        }
    }

    fn resize_along(&mut self, target: u64, axis: Axis, delta: f32) -> bool {
        let Self::Split {
            axis: node_axis,
            ratio,
            first,
            second,
        } = self
        else {
            return false;
        };
        let resized_child = if first.contains(target) {
            first.resize_along(target, axis, delta)
        } else if second.contains(target) {
            second.resize_along(target, axis, delta)
        } else {
            return false;
        };
        if resized_child {
            return true;
        }
        if *node_axis == axis {
            *ratio = (*ratio + delta).clamp(MIN_RATIO, MAX_RATIO);
            return true;
        }
        false
    }

    fn set_ratio(&mut self, path: &[bool], new_ratio: f32) -> bool {
        let Self::Split {
            ratio,
            first,
            second,
            ..
        } = self
        else {
            return false;
        };
        match path.split_first() {
            None => {
                *ratio = new_ratio.clamp(MIN_RATIO, MAX_RATIO);
                true
            }
            Some((false, rest)) => first.set_ratio(rest, new_ratio),
            Some((true, rest)) => second.set_ratio(rest, new_ratio),
        }
    }
}

fn split_rect(area: Rect, axis: Axis, ratio: f32, divider: f32) -> (Rect, Rect, Rect) {
    let measure = |total: f32| {
        let div = divider.min((total - 2.0).max(0.0));
        let avail = (total - div).max(0.0);
        let lo = 1.0_f32.min(avail);
        let hi = (avail - 1.0).max(lo);
        let first = (avail * ratio).round().clamp(lo, hi);
        (first, div, (avail - first).max(0.0))
    };
    match axis {
        Axis::Horizontal => {
            let (w1, div, w2) = measure(area.width);
            (
                Rect {
                    x: area.x,
                    y: area.y,
                    width: w1,
                    height: area.height,
                },
                Rect {
                    x: area.x + w1,
                    y: area.y,
                    width: div,
                    height: area.height,
                },
                Rect {
                    x: area.x + w1 + div,
                    y: area.y,
                    width: w2,
                    height: area.height,
                },
            )
        }
        Axis::Vertical => {
            let (h1, div, h2) = measure(area.height);
            (
                Rect {
                    x: area.x,
                    y: area.y,
                    width: area.width,
                    height: h1,
                },
                Rect {
                    x: area.x,
                    y: area.y + h1,
                    width: area.width,
                    height: div,
                },
                Rect {
                    x: area.x,
                    y: area.y + h1 + div,
                    width: area.width,
                    height: h2,
                },
            )
        }
    }
}

fn split_node<T>(node: Node<T>, target: u64, axis: Axis, new_id: u64, new_item: T) -> Node<T> {
    match node {
        Node::Leaf { id, item } if id == target => Node::Split {
            axis,
            ratio: 0.5,
            first: Box::new(Node::Leaf { id, item }),
            second: Box::new(Node::Leaf {
                id: new_id,
                item: new_item,
            }),
        },
        leaf @ Node::Leaf { .. } => leaf,
        Node::Split {
            axis: node_axis,
            ratio,
            first,
            second,
        } => {
            if first.contains(target) {
                Node::Split {
                    axis: node_axis,
                    ratio,
                    first: Box::new(split_node(*first, target, axis, new_id, new_item)),
                    second,
                }
            } else {
                Node::Split {
                    axis: node_axis,
                    ratio,
                    first,
                    second: Box::new(split_node(*second, target, axis, new_id, new_item)),
                }
            }
        }
    }
}

enum Removed<T> {
    Sibling { node: Node<T>, fallback: u64 },
    Missing(Node<T>),
}

fn remove_node<T>(node: Node<T>, target: u64) -> Removed<T> {
    match node {
        leaf @ Node::Leaf { .. } => Removed::Missing(leaf),
        Node::Split {
            axis,
            ratio,
            first,
            second,
        } => {
            if matches!(*first, Node::Leaf { id, .. } if id == target) {
                let fallback = second.first_id();
                return Removed::Sibling {
                    node: *second,
                    fallback,
                };
            }
            if matches!(*second, Node::Leaf { id, .. } if id == target) {
                let fallback = first.first_id();
                return Removed::Sibling {
                    node: *first,
                    fallback,
                };
            }
            if first.contains(target) {
                match remove_node(*first, target) {
                    Removed::Sibling { node, fallback } => Removed::Sibling {
                        node: Node::Split {
                            axis,
                            ratio,
                            first: Box::new(node),
                            second,
                        },
                        fallback,
                    },
                    Removed::Missing(first) => Removed::Missing(Node::Split {
                        axis,
                        ratio,
                        first: Box::new(first),
                        second,
                    }),
                }
            } else if second.contains(target) {
                match remove_node(*second, target) {
                    Removed::Sibling { node, fallback } => Removed::Sibling {
                        node: Node::Split {
                            axis,
                            ratio,
                            first,
                            second: Box::new(node),
                        },
                        fallback,
                    },
                    Removed::Missing(second) => Removed::Missing(Node::Split {
                        axis,
                        ratio,
                        first,
                        second: Box::new(second),
                    }),
                }
            } else {
                Removed::Missing(Node::Split {
                    axis,
                    ratio,
                    first,
                    second,
                })
            }
        }
    }
}

#[derive(Debug)]
pub struct SplitTree<T> {
    root: Option<Node<T>>,
    active: u64,
}

impl<T> SplitTree<T> {
    pub fn new(id: u64, item: T) -> Self {
        Self {
            root: Some(Node::Leaf { id, item }),
            active: id,
        }
    }

    pub fn len(&self) -> usize {
        self.root.as_ref().map_or(0, Node::len)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn active_id(&self) -> u64 {
        self.active
    }

    pub fn active(&self) -> &T {
        self.get(self.active).expect("active pane exists")
    }

    pub fn active_mut(&mut self) -> &mut T {
        let id = self.active;
        self.get_mut(id).expect("active pane exists")
    }

    pub fn get(&self, id: u64) -> Option<&T> {
        self.root.as_ref()?.get(id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut T> {
        self.root.as_mut()?.get_mut(id)
    }

    pub fn contains(&self, id: u64) -> bool {
        self.root.as_ref().is_some_and(|root| root.contains(id))
    }

    pub fn snapshot(
        &self,
        mut map: impl FnMut(u64, &T) -> Option<String>,
    ) -> crate::workspace::SavedNode {
        fn build<T>(
            node: &Node<T>,
            active: u64,
            map: &mut impl FnMut(u64, &T) -> Option<String>,
        ) -> crate::workspace::SavedNode {
            match node {
                Node::Leaf { id, item } => crate::workspace::SavedNode::Pane {
                    cwd: map(*id, item),
                    active: *id == active,
                },
                Node::Split {
                    axis,
                    ratio,
                    first,
                    second,
                } => crate::workspace::SavedNode::Split {
                    axis: *axis,
                    ratio: *ratio,
                    first: Box::new(build(first, active, map)),
                    second: Box::new(build(second, active, map)),
                },
            }
        }
        match &self.root {
            Some(root) => build(root, self.active, &mut map),
            None => crate::workspace::SavedNode::Pane {
                cwd: None,
                active: true,
            },
        }
    }

    pub fn from_saved<E>(
        saved: &crate::workspace::SavedNode,
        make_pane: &mut impl FnMut(Option<&str>) -> Result<(u64, T), E>,
    ) -> Result<Self, E> {
        fn build<T, E>(
            saved: &crate::workspace::SavedNode,
            make_pane: &mut impl FnMut(Option<&str>) -> Result<(u64, T), E>,
            active_id: &mut Option<u64>,
        ) -> Result<Node<T>, E> {
            match saved {
                crate::workspace::SavedNode::Pane { cwd, active } => {
                    let (id, item) = make_pane(cwd.as_deref())?;
                    if *active && active_id.is_none() {
                        *active_id = Some(id);
                    }
                    Ok(Node::Leaf { id, item })
                }
                crate::workspace::SavedNode::Split {
                    axis,
                    ratio,
                    first,
                    second,
                } => Ok(Node::Split {
                    axis: *axis,
                    ratio: ratio.clamp(MIN_RATIO, MAX_RATIO),
                    first: Box::new(build(first, make_pane, active_id)?),
                    second: Box::new(build(second, make_pane, active_id)?),
                }),
            }
        }
        let mut active_id = None;
        let root = build(saved, make_pane, &mut active_id)?;
        let active = active_id.unwrap_or_else(|| root.first_id());
        Ok(Self {
            root: Some(root),
            active,
        })
    }

    pub fn iter(&self) -> Vec<(u64, &T)> {
        let mut out = Vec::with_capacity(self.len());
        if let Some(root) = &self.root {
            root.collect(&mut out);
        }
        out
    }

    pub fn iter_mut(&mut self) -> Vec<(u64, &mut T)> {
        let mut out = Vec::with_capacity(self.len());
        if let Some(root) = &mut self.root {
            root.collect_mut(&mut out);
        }
        out
    }

    pub fn split_active(&mut self, axis: Axis, new_id: u64, new_item: T) {
        if let Some(root) = self.root.take() {
            self.root = Some(split_node(root, self.active, axis, new_id, new_item));
            self.active = new_id;
        }
    }

    pub fn close(&mut self, id: u64) -> CloseResult {
        if !self.contains(id) {
            return CloseResult::NotFound;
        }
        if self.len() <= 1 {
            return CloseResult::LastPane;
        }
        let Some(root) = self.root.take() else {
            return CloseResult::NotFound;
        };
        match remove_node(root, id) {
            Removed::Sibling { node, fallback } => {
                self.root = Some(node);
                if self.active == id {
                    self.active = fallback;
                }
                CloseResult::Closed
            }
            Removed::Missing(node) => {
                self.root = Some(node);
                CloseResult::NotFound
            }
        }
    }

    pub fn layout(&self, area: Rect, divider: f32) -> (Vec<(u64, Rect)>, Vec<Divider>) {
        let mut panes = Vec::new();
        let mut dividers = Vec::new();
        let mut path = Vec::new();
        if let Some(root) = &self.root {
            root.layout(area, divider, &mut path, &mut panes, &mut dividers);
        }
        (panes, dividers)
    }

    pub fn focus(&mut self, id: u64) -> bool {
        if self.contains(id) {
            self.active = id;
            true
        } else {
            false
        }
    }

    pub fn focus_dir(&mut self, dir: Dir, area: Rect, divider: f32) -> bool {
        let (panes, _) = self.layout(area, divider);
        let Some((_, cur)) = panes.iter().find(|(id, _)| *id == self.active).copied() else {
            return false;
        };
        let cur_cx = cur.x + cur.width / 2.0;
        let cur_cy = cur.y + cur.height / 2.0;
        let mut best: Option<(u8, i64, i64, u64)> = None;

        for (id, rect) in panes {
            if id == self.active {
                continue;
            }
            let (in_dir, primary, overlap, perp_dist) = match dir {
                Dir::Left => (
                    rect.x + rect.width <= cur.x + divider + 1.0 && rect.x < cur.x,
                    (cur.x - (rect.x + rect.width)).max(0.0),
                    rect.y < cur.y + cur.height && rect.y + rect.height > cur.y,
                    ((rect.y + rect.height / 2.0) - cur_cy).abs(),
                ),
                Dir::Right => (
                    rect.x >= cur.x + cur.width - 1.0 && rect.x + rect.width > cur.x + cur.width,
                    (rect.x - (cur.x + cur.width)).max(0.0),
                    rect.y < cur.y + cur.height && rect.y + rect.height > cur.y,
                    ((rect.y + rect.height / 2.0) - cur_cy).abs(),
                ),
                Dir::Up => (
                    rect.y + rect.height <= cur.y + divider + 1.0 && rect.y < cur.y,
                    (cur.y - (rect.y + rect.height)).max(0.0),
                    rect.x < cur.x + cur.width && rect.x + rect.width > cur.x,
                    ((rect.x + rect.width / 2.0) - cur_cx).abs(),
                ),
                Dir::Down => (
                    rect.y >= cur.y + cur.height - 1.0 && rect.y + rect.height > cur.y + cur.height,
                    (rect.y - (cur.y + cur.height)).max(0.0),
                    rect.x < cur.x + cur.width && rect.x + rect.width > cur.x,
                    ((rect.x + rect.width / 2.0) - cur_cx).abs(),
                ),
            };
            if !in_dir {
                continue;
            }
            let key = (
                u8::from(!overlap),
                (primary * 100.0) as i64,
                (perp_dist * 100.0) as i64,
                id,
            );
            if best.is_none_or(|other| key < other) {
                best = Some(key);
            }
        }
        if let Some((_, _, _, id)) = best {
            self.active = id;
            true
        } else {
            false
        }
    }

    pub fn resize_active(&mut self, dir: Dir, delta: f32) -> bool {
        let signed = match dir {
            Dir::Right | Dir::Down => delta,
            Dir::Left | Dir::Up => -delta,
        };
        let active = self.active;
        self.root
            .as_mut()
            .is_some_and(|root| root.resize_along(active, dir.axis(), signed))
    }

    pub fn set_ratio(&mut self, path: &[bool], ratio: f32) -> bool {
        self.root
            .as_mut()
            .is_some_and(|root| root.set_ratio(path, ratio))
    }

    pub fn pane_at(&self, area: Rect, divider: f32, x: f32, y: f32) -> Option<(u64, Rect)> {
        let (panes, _) = self.layout(area, divider);
        panes.into_iter().find(|(_, rect)| {
            (rect.x..rect.x + rect.width).contains(&x)
                && (rect.y..rect.y + rect.height).contains(&y)
        })
    }

    pub fn divider_at(
        &self,
        area: Rect,
        divider: f32,
        hit_margin: f32,
        x: f32,
        y: f32,
    ) -> Option<Divider> {
        let (_, dividers) = self.layout(area, divider);
        dividers.into_iter().find(|div| match div.axis {
            Axis::Horizontal => {
                (div.rect.x - hit_margin..div.rect.x + div.rect.width + hit_margin).contains(&x)
                    && (div.rect.y..div.rect.y + div.rect.height).contains(&y)
            }
            Axis::Vertical => {
                (div.rect.x..div.rect.x + div.rect.width).contains(&x)
                    && (div.rect.y - hit_margin..div.rect.y + div.rect.height + hit_margin)
                        .contains(&y)
            }
        })
    }
}
