use koral::boxes::quads;
use koral::render::Quad;

const INK: [f32; 4] = [1.0; 4];

fn cell(ch: char) -> Option<Vec<Quad>> {
    quads(ch, 0.0, 0.0, 10.0, 20.0, 2.0, INK)
}

fn reaches(quads: &[Quad]) -> (f32, f32, f32, f32) {
    let left = quads.iter().map(|quad| quad.x).fold(f32::MAX, f32::min);
    let top = quads.iter().map(|quad| quad.y).fold(f32::MAX, f32::min);
    let right = quads.iter().map(|quad| quad.x + quad.width).fold(0.0, f32::max);
    let bottom = quads.iter().map(|quad| quad.y + quad.height).fold(0.0, f32::max);
    (left, top, right, bottom)
}

#[test]
fn a_vertical_line_spans_the_cell() {
    let quads = cell('│').unwrap();
    assert_eq!(quads.len(), 1);
    assert_eq!((quads[0].y, quads[0].height, quads[0].width), (0.0, 20.0, 2.0));
    assert_eq!(quads[0].x, 4.0);
}

#[test]
fn a_horizontal_line_spans_the_cell() {
    let quads = cell('─').unwrap();
    assert_eq!(quads.len(), 1);
    assert_eq!((quads[0].x, quads[0].width, quads[0].height), (0.0, 10.0, 2.0));
}

#[test]
fn a_cross_reaches_all_four_edges() {
    assert_eq!(reaches(&cell('┼').unwrap()), (0.0, 0.0, 10.0, 20.0));
}

#[test]
fn a_heavy_line_is_twice_as_thick() {
    assert_eq!(cell('┃').unwrap()[0].width, 4.0);
}

#[test]
fn a_corner_has_no_notch() {
    let quads = cell('┌').unwrap();
    assert_eq!(reaches(&quads), (4.0, 9.0, 10.0, 20.0));
    assert_eq!(quads.iter().map(|quad| quad.y).fold(f32::MAX, f32::min), 9.0);
    assert!(quads.iter().any(|quad| quad.x == 4.0 && quad.y == 9.0));
}

#[test]
fn a_double_corner_nests_its_lines() {
    let quads = cell('╔').unwrap();
    assert_eq!(quads.len(), 4);
    assert_eq!(reaches(&quads), (2.0, 7.0, 10.0, 20.0));
    // The outer line starts at the outer corner, the inner one past the other inner line.
    assert!(quads.iter().any(|quad| quad.x == 2.0 && quad.y == 7.0));
    assert!(quads.iter().any(|quad| quad.x == 8.0 && quad.y == 11.0));
}

#[test]
fn a_mixed_corner_keeps_both_of_its_weights() {
    assert_eq!(cell('╓').unwrap().len(), 3);
    assert_eq!(cell('╒').unwrap().len(), 3);
}

#[test]
fn half_lines_stop_in_the_middle() {
    assert_eq!(reaches(&cell('╴').unwrap()), (0.0, 9.0, 5.0, 11.0));
}

#[test]
fn quads_land_on_whole_pixels() {
    for ch in ['│', '┼', '╬', '▀', '▚', '▂'] {
        for quad in quads(ch, 10.4, 7.6, 9.7, 19.3, 1.5, INK).unwrap() {
            for edge in [quad.x, quad.y, quad.x + quad.width, quad.y + quad.height] {
                assert_eq!(edge, edge.round(), "{ch}");
            }
        }
    }
}

#[test]
fn neighbours_leave_no_seam() {
    let first = quads('█', 10.4, 0.0, 9.7, 20.0, 1.0, INK).unwrap();
    let second = quads('█', 10.4 + 9.7, 0.0, 9.7, 20.0, 1.0, INK).unwrap();
    assert_eq!(first[0].x + first[0].width, second[0].x);
}

#[test]
fn a_full_block_fills_the_cell() {
    assert_eq!(reaches(&cell('█').unwrap()), (0.0, 0.0, 10.0, 20.0));
}

#[test]
fn an_upper_half_fills_the_top() {
    assert_eq!(reaches(&cell('▀').unwrap()), (0.0, 0.0, 10.0, 10.0));
}

#[test]
fn eighths_grow_from_their_edge() {
    assert_eq!(reaches(&cell('▂').unwrap()), (0.0, 15.0, 10.0, 20.0));
    assert_eq!(reaches(&cell('▏').unwrap()), (0.0, 0.0, 1.0, 20.0));
    assert_eq!(reaches(&cell('▕').unwrap()), (9.0, 0.0, 10.0, 20.0));
}

#[test]
fn a_quadrant_fills_its_corner() {
    assert_eq!(reaches(&cell('▗').unwrap()), (5.0, 10.0, 10.0, 20.0));
    assert_eq!(cell('▚').unwrap().len(), 2);
}

#[test]
fn a_light_shade_is_translucent() {
    assert_eq!(cell('░').unwrap()[0].color[3], 0.25);
}

#[test]
fn arcs_stay_with_the_font() {
    assert!(cell('╭').is_none());
    assert!(cell('a').is_none());
    assert!(cell('┄').is_none());
}
