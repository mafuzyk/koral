use koral::diff::{DiffTag, diff_lines, format_unified};

#[test]
fn identical_outputs_produce_no_hunks() {
    assert!(diff_lines("a\nb\nc\n", "a\nb\nc\n", 3).is_empty());
    assert!(format_unified(&[], "a", "b", false).is_empty());
}

#[test]
fn detects_additions_removals_and_keeps_context() {
    let old = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\n";
    let new = "l1\nl2\nl3\nchanged\nl5\nl6\nl7\nl8\nextra\n";
    let hunks = diff_lines(old, new, 1);
    assert_eq!(hunks.len(), 2);

    let text = format_unified(&hunks, "@-2", "@-1", false);
    assert!(text.contains("--- @-2\n+++ @-1\n"));
    assert!(text.contains("-l4\n+changed\n"));
    assert!(text.contains("+extra\n"));

    let removed: Vec<_> = hunks[0]
        .lines
        .iter()
        .filter(|l| l.tag == DiffTag::Removed)
        .map(|l| l.text.as_str())
        .collect();
    assert_eq!(removed, vec!["l4"]);
}

#[test]
fn oversized_middle_falls_back_cleanly_without_oom() {
    let old: String = (0..1600).map(|i| format!("old-{i}\n")).collect();
    let new: String = (0..1600).map(|i| format!("new-{i}\n")).collect();
    let hunks = diff_lines(&old, &new, 2);
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0].lines.len(), 3200);
}
