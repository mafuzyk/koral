use koral::splits::{Axis, CloseResult, Dir, SplitTree};
use koral::tabs::Rect;

const AREA: Rect = Rect {
    x: 0.0,
    y: 0.0,
    width: 1002.0,
    height: 602.0,
};

#[test]
fn single_pane_layout_fills_area() {
    let tree = SplitTree::new(1, "a");
    assert_eq!(tree.len(), 1);
    assert_eq!(tree.active_id(), 1);
    let (panes, dividers) = tree.layout(AREA, 2.0);
    assert_eq!(panes, vec![(1, AREA)]);
    assert!(dividers.is_empty());
}

#[test]
fn splitting_activates_new_pane_and_divides_space() {
    let mut tree = SplitTree::new(1, "left");
    tree.split_active(Axis::Horizontal, 2, "right");
    assert_eq!(tree.len(), 2);
    assert_eq!(tree.active_id(), 2);
    let (panes, dividers) = tree.layout(AREA, 2.0);
    assert_eq!(panes.len(), 2);
    assert_eq!(dividers.len(), 1);
    assert_eq!(panes[0].1.width, 500.0);
    assert_eq!(
        dividers[0].rect,
        Rect {
            x: 500.0,
            y: 0.0,
            width: 2.0,
            height: 602.0
        }
    );
    assert_eq!(panes[1].1.x, 502.0);
    assert_eq!(panes[1].1.width, 500.0);
}

#[test]
fn nested_focus_dir_moves_across_panes() {
    let mut tree = SplitTree::new(1, "a");
    tree.split_active(Axis::Horizontal, 2, "b");
    tree.split_active(Axis::Vertical, 3, "c");
    assert_eq!(tree.active_id(), 3);

    assert!(tree.focus_dir(Dir::Up, AREA, 2.0));
    assert_eq!(tree.active_id(), 2);

    assert!(tree.focus_dir(Dir::Left, AREA, 2.0));
    assert_eq!(tree.active_id(), 1);

    assert!(tree.focus_dir(Dir::Right, AREA, 2.0));
    assert_eq!(tree.active_id(), 2);

    assert!(tree.focus_dir(Dir::Down, AREA, 2.0));
    assert_eq!(tree.active_id(), 3);

    assert!(!tree.focus_dir(Dir::Down, AREA, 2.0));
}

#[test]
fn closing_pane_promotes_sibling_and_refuses_last_pane() {
    let mut tree = SplitTree::new(1, "a");
    assert_eq!(tree.close(1), CloseResult::LastPane);

    tree.split_active(Axis::Horizontal, 2, "b");
    assert_eq!(tree.close(99), CloseResult::NotFound);
    assert_eq!(tree.close(2), CloseResult::Closed);
    assert_eq!(tree.len(), 1);
    assert_eq!(tree.active_id(), 1);
}

#[test]
fn resizing_and_divider_hit_test_stay_clamped() {
    let mut tree = SplitTree::new(1, "a");
    tree.split_active(Axis::Horizontal, 2, "b");
    assert!(tree.resize_active(Dir::Right, 0.1));
    let (panes, _) = tree.layout(AREA, 2.0);
    assert_eq!(panes[0].1.width, 600.0);

    for _ in 0..10 {
        tree.resize_active(Dir::Right, 0.1);
    }
    let (panes, _) = tree.layout(AREA, 2.0);
    assert_eq!(panes[0].1.width, 850.0);

    let div = tree
        .divider_at(AREA, 2.0, 4.0, 851.0, 100.0)
        .expect("divider hit");
    assert!(tree.set_ratio(&div.path, 0.25));
    let (panes, _) = tree.layout(AREA, 2.0);
    assert_eq!(panes[0].1.width, 250.0);
    assert_eq!(
        tree.pane_at(AREA, 2.0, 10.0, 10.0).map(|(id, _)| id),
        Some(1)
    );
    assert_eq!(
        tree.pane_at(AREA, 2.0, 400.0, 10.0).map(|(id, _)| id),
        Some(2)
    );
}
