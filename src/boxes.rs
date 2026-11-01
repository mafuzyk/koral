use crate::render::Quad;

/// Arms of U+2500 to U+257F as left, right, up, down: 1 light, 2 heavy, 3 double.
// shortcut: arcs, diagonals and dashed lines stay with the font, quads cannot draw curves; a mask atlas is the upgrade.
#[rustfmt::skip]
const ARMS: [&[u8; 4]; 128] = [
    b"1100", b"2200", b"0011", b"0022", b"0000", b"0000", b"0000", b"0000",
    b"0000", b"0000", b"0000", b"0000", b"0101", b"0201", b"0102", b"0202",
    b"1001", b"2001", b"1002", b"2002", b"0110", b"0210", b"0120", b"0220",
    b"1010", b"2010", b"1020", b"2020", b"0111", b"0211", b"0121", b"0112",
    b"0122", b"0221", b"0212", b"0222", b"1011", b"2011", b"1021", b"1012",
    b"1022", b"2021", b"2012", b"2022", b"1101", b"2101", b"1201", b"2201",
    b"1102", b"2102", b"1202", b"2202", b"1110", b"2110", b"1210", b"2210",
    b"1120", b"2120", b"1220", b"2220", b"1111", b"2111", b"1211", b"2211",
    b"1121", b"1112", b"1122", b"2121", b"1221", b"2112", b"1212", b"2221",
    b"2212", b"2122", b"1222", b"2222", b"0000", b"0000", b"0000", b"0000",
    b"3300", b"0033", b"0301", b"0103", b"0303", b"3001", b"1003", b"3003",
    b"0310", b"0130", b"0330", b"3010", b"1030", b"3030", b"0311", b"0133",
    b"0333", b"3011", b"1033", b"3033", b"3301", b"1103", b"3303", b"3310",
    b"1130", b"3330", b"3311", b"1133", b"3333", b"0000", b"0000", b"0000",
    b"0000", b"0000", b"0000", b"0000", b"1000", b"0010", b"0100", b"0001",
    b"2000", b"0020", b"0200", b"0002", b"1200", b"0012", b"2100", b"0021",
];

/// The quads for a character Koral draws itself. None means the font draws it.
pub fn quads(ch: char, x: f32, y: f32, width: f32, height: f32, thickness: f32, color: [f32; 4]) -> Option<Vec<Quad>> {
    let area = Area { left: x.round(), top: y.round(), right: (x + width).round(), bottom: (y + height).round() };
    match ch as u32 {
        0x2500..=0x257F => lines(ch as usize - 0x2500, area, thickness.round().max(1.0), color),
        0x2580..=0x259F => Some(blocks(ch as u32, area, x, y, width, height, color)),
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct Area {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

fn lines(index: usize, area: Area, thickness: f32, color: [f32; 4]) -> Option<Vec<Quad>> {
    let [left, right, up, down] = ARMS[index].map(|digit| digit - b'0');
    if [left, right, up, down] == [0; 4] {
        return None;
    }
    let (x, y) = (((area.left + area.right) / 2.0).floor(), ((area.top + area.bottom) / 2.0).floor());
    let across = strokes([left, right], [up, down], [area.left, area.right], [x, y], thickness);
    let along = strokes([up, down], [left, right], [area.top, area.bottom], [y, x], thickness);
    let wide = across.into_iter().map(|[start, end, top, bottom]| Quad::new(start, top, end - start, bottom - top, color));
    let tall = along.into_iter().map(|[start, end, left, right]| Quad::new(left, start, right - left, end - start, color));
    Some(wide.chain(tall).collect())
}

/// The whole pixels a line of this weight covers around `center`.
fn edges(center: f32, weight: u8, thickness: f32) -> (f32, f32) {
    let width = thickness * f32::from(weight);
    let low = center - (width / 2.0).floor();
    (low, low + width)
}

/// Strokes along one axis as start, end and the two edges across it. `sides` are the arms that
/// cross it, which decide where each stroke stops so that corners close and doubles nest.
fn strokes(arms: [u8; 2], sides: [u8; 2], span: [f32; 2], center: [f32; 2], thickness: f32) -> Vec<[f32; 4]> {
    if arms[0] == arms[1] && matches!(arms[0], 1 | 2) {
        let (low, high) = edges(center[1], arms[0], thickness);
        return vec![[span[0], span[1], low, high]];
    }
    let reach = |weight| edges(center[0], weight, thickness);
    let mut strokes = Vec::new();
    for (weight, before) in [(arms[0], true), (arms[1], false)] {
        if weight == 0 {
            continue;
        }
        let mut push = |inner: f32, low: f32, high: f32| {
            strokes.push(if before { [span[0], inner, low, high] } else { [inner, span[1], low, high] });
        };
        let (low, high) = edges(center[1], weight, thickness);
        if weight == 3 {
            for (line, (top, bottom)) in [(low, low + thickness), (high - thickness, high)].into_iter().enumerate() {
                let (near, far) = (sides[line], sides[1 - line]);
                let inner = match (before, near != 0) {
                    (true, true) => reach(near).0,
                    (true, false) => reach(far).1,
                    (false, true) => reach(near).1,
                    (false, false) => reach(far).0,
                };
                push(inner, top, bottom);
            }
        } else if before {
            push(reach(sides[0]).1.max(reach(sides[1]).1), low, high);
        } else {
            push(reach(sides[0]).0.min(reach(sides[1]).0), low, high);
        }
    }
    strokes
}

fn blocks(ch: u32, area: Area, x: f32, y: f32, width: f32, height: f32, color: [f32; 4]) -> Vec<Quad> {
    // Edges in eighths of the cell, so a fraction lands on the same pixel in every cell.
    let rect = |left: u32, top: u32, right: u32, bottom: u32, color: [f32; 4]| {
        let across = |eighth: u32| match eighth {
            0 => area.left,
            8 => area.right,
            _ => (x + width * eighth as f32 / 8.0).round(),
        };
        let down = |eighth: u32| match eighth {
            0 => area.top,
            8 => area.bottom,
            _ => (y + height * eighth as f32 / 8.0).round(),
        };
        Quad::new(across(left), down(top), across(right) - across(left), down(bottom) - down(top), color)
    };
    let shade = |alpha: f32| vec![rect(0, 0, 8, 8, [color[0], color[1], color[2], color[3] * alpha])];
    match ch {
        0x2580 => vec![rect(0, 0, 8, 4, color)],
        0x2581..=0x2588 => vec![rect(0, 0x2588 - ch, 8, 8, color)],
        0x2589..=0x258F => vec![rect(0, 0, 0x2590 - ch, 8, color)],
        0x2590 => vec![rect(4, 0, 8, 8, color)],
        0x2591 => shade(0.25),
        0x2592 => shade(0.5),
        0x2593 => shade(0.75),
        0x2594 => vec![rect(0, 0, 8, 1, color)],
        0x2595 => vec![rect(7, 0, 8, 8, color)],
        _ => {
            const CORNERS: [u8; 10] = [4, 8, 1, 13, 9, 7, 11, 2, 6, 14];
            let mask = CORNERS[(ch - 0x2596) as usize];
            [(1, 0, 0), (2, 4, 0), (4, 0, 4), (8, 4, 4)]
                .into_iter()
                .filter(|(bit, ..)| mask & bit != 0)
                .map(|(_, left, top)| rect(left, top, left + 4, top + 4, color))
                .collect()
        }
    }
}
