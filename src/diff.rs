use std::fmt::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffTag {
    Equal,
    Removed,
    Added,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub tag: DiffTag,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffHunk {
    pub old_start: usize,
    pub new_start: usize,
    pub lines: Vec<DiffLine>,
}

const MAX_DP_CELLS: usize = 2_000_000;

/// Computes line-level unified hunks between `old` and `new` with `context` surrounding lines.
pub fn diff_lines(old: &str, new: &str, context: usize) -> Vec<DiffHunk> {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let mut prefix = 0usize;
    let min_len = old_lines.len().min(new_lines.len());
    while prefix < min_len && old_lines[prefix] == new_lines[prefix] {
        prefix += 1;
    }

    let mut suffix = 0usize;
    while suffix < (min_len - prefix)
        && old_lines[old_lines.len() - 1 - suffix] == new_lines[new_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let old_mid = &old_lines[prefix..old_lines.len() - suffix];
    let new_mid = &new_lines[prefix..new_lines.len() - suffix];
    if old_mid.is_empty() && new_mid.is_empty() {
        return Vec::new();
    }

    let mut all = Vec::with_capacity(old_lines.len() + new_lines.len());
    for (idx, line) in old_lines[..prefix].iter().enumerate() {
        all.push(DiffLine {
            tag: DiffTag::Equal,
            old_line: Some(idx + 1),
            new_line: Some(idx + 1),
            text: (*line).to_string(),
        });
    }

    let mid_ops = diff_middle(old_mid, new_mid, prefix);
    all.extend(mid_ops);

    let old_tail_start = old_lines.len() - suffix;
    let new_tail_start = new_lines.len() - suffix;
    for offset in 0..suffix {
        all.push(DiffLine {
            tag: DiffTag::Equal,
            old_line: Some(old_tail_start + offset + 1),
            new_line: Some(new_tail_start + offset + 1),
            text: old_lines[old_tail_start + offset].to_string(),
        });
    }

    build_hunks(&all, context)
}

fn diff_middle(old_mid: &[&str], new_mid: &[&str], prefix: usize) -> Vec<DiffLine> {
    let n = old_mid.len();
    let m = new_mid.len();
    if n.saturating_mul(m) > MAX_DP_CELLS {
        let mut out = Vec::with_capacity(n + m);
        for (i, line) in old_mid.iter().enumerate() {
            out.push(DiffLine {
                tag: DiffTag::Removed,
                old_line: Some(prefix + i + 1),
                new_line: None,
                text: (*line).to_string(),
            });
        }
        for (j, line) in new_mid.iter().enumerate() {
            out.push(DiffLine {
                tag: DiffTag::Added,
                old_line: None,
                new_line: Some(prefix + j + 1),
                text: (*line).to_string(),
            });
        }
        return out;
    }

    let cols = m + 1;
    let mut dp = vec![0u32; (n + 1) * cols];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i * cols + j] = if old_mid[i] == new_mid[j] {
                dp[(i + 1) * cols + (j + 1)] + 1
            } else {
                dp[(i + 1) * cols + j].max(dp[i * cols + (j + 1)])
            };
        }
    }

    let mut out = Vec::with_capacity(n + m);
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if old_mid[i] == new_mid[j] {
            out.push(DiffLine {
                tag: DiffTag::Equal,
                old_line: Some(prefix + i + 1),
                new_line: Some(prefix + j + 1),
                text: old_mid[i].to_string(),
            });
            i += 1;
            j += 1;
        } else if dp[(i + 1) * cols + j] >= dp[i * cols + (j + 1)] {
            out.push(DiffLine {
                tag: DiffTag::Removed,
                old_line: Some(prefix + i + 1),
                new_line: None,
                text: old_mid[i].to_string(),
            });
            i += 1;
        } else {
            out.push(DiffLine {
                tag: DiffTag::Added,
                old_line: None,
                new_line: Some(prefix + j + 1),
                text: new_mid[j].to_string(),
            });
            j += 1;
        }
    }
    while i < n {
        out.push(DiffLine {
            tag: DiffTag::Removed,
            old_line: Some(prefix + i + 1),
            new_line: None,
            text: old_mid[i].to_string(),
        });
        i += 1;
    }
    while j < m {
        out.push(DiffLine {
            tag: DiffTag::Added,
            old_line: None,
            new_line: Some(prefix + j + 1),
            text: new_mid[j].to_string(),
        });
        j += 1;
    }
    out
}

fn build_hunks(all: &[DiffLine], context: usize) -> Vec<DiffHunk> {
    let changed: Vec<usize> = all
        .iter()
        .enumerate()
        .filter_map(|(idx, line)| (line.tag != DiffTag::Equal).then_some(idx))
        .collect();
    if changed.is_empty() {
        return Vec::new();
    }

    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for idx in changed {
        let start = idx.saturating_sub(context);
        let end = (idx + context + 1).min(all.len());
        if let Some(last) = ranges.last_mut()
            && start <= last.1
        {
            last.1 = last.1.max(end);
        } else {
            ranges.push((start, end));
        }
    }

    ranges
        .into_iter()
        .map(|(start, end)| {
            let slice = &all[start..end];
            let old_start = slice.iter().find_map(|l| l.old_line).unwrap_or(1);
            let new_start = slice.iter().find_map(|l| l.new_line).unwrap_or(1);
            DiffHunk {
                old_start,
                new_start,
                lines: slice.to_vec(),
            }
        })
        .collect()
}

pub fn format_unified(hunks: &[DiffHunk], old_label: &str, new_label: &str, color: bool) -> String {
    if hunks.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    let _ = writeln!(out, "--- {old_label}");
    let _ = writeln!(out, "+++ {new_label}");
    for hunk in hunks {
        let old_count = hunk.lines.iter().filter(|l| l.old_line.is_some()).count();
        let new_count = hunk.lines.iter().filter(|l| l.new_line.is_some()).count();
        if color {
            let _ = writeln!(
                out,
                "\x1b[36m@@ -{},{} +{},{} @@\x1b[0m",
                hunk.old_start, old_count, hunk.new_start, new_count
            );
        } else {
            let _ = writeln!(
                out,
                "@@ -{},{} +{},{} @@",
                hunk.old_start, old_count, hunk.new_start, new_count
            );
        }
        for line in &hunk.lines {
            match (line.tag, color) {
                (DiffTag::Equal, _) => {
                    let _ = writeln!(out, " {}", line.text);
                }
                (DiffTag::Removed, true) => {
                    let _ = writeln!(out, "\x1b[31m-{}\x1b[0m", line.text);
                }
                (DiffTag::Removed, false) => {
                    let _ = writeln!(out, "-{}", line.text);
                }
                (DiffTag::Added, true) => {
                    let _ = writeln!(out, "\x1b[32m+{}\x1b[0m", line.text);
                }
                (DiffTag::Added, false) => {
                    let _ = writeln!(out, "+{}", line.text);
                }
            }
        }
    }
    out
}
