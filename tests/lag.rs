use koral::lag::{self, Cost, LineCost};

/// The log is how a stutter gets measured: only slow frames reach it, the line names the worst one
/// well enough to identify the character and the face that shaped it, and the warm-up reports itself
/// once. One test, because the log file is opened once per process.
#[test]
fn the_log_names_the_worst_line_and_the_warm_up() {
    let path = std::env::temp_dir().join(format!("koral-lag-{}.log", std::process::id()));
    let _ = std::fs::remove_file(&path);
    // SAFETY: this test binary holds one test, and nothing else reads the variable.
    unsafe { std::env::set_var("KORAL_FRAME_LOG", &path) };

    let mut cost = Cost { total: 3.0, ..Cost::default() };
    lag::note(&cost);
    cost.total = 78.9;
    cost.text = 77.5;
    cost.shape = 69.5;
    cost.shaped = 28;
    cost.glyphs = 7_380;
    cost.worst = LineCost {
        micros: 67_332,
        chars: 1,
        spans: 1,
        colors: 1,
        non_ascii: true,
        codes: "1f600".to_string(),
        face: "Noto Color Emoji".to_string(),
        fallback: true,
        again: 44,
    };
    lag::note(&cost);
    lag::warmed(13.4, &["JetBrainsMono Nerd Font Mono".to_string(), "Noto Color Emoji".to_string()], "ação 😀");

    let text = std::fs::read_to_string(&path).expect("the log was written");
    let _ = std::fs::remove_file(&path);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "only the slow frame and the warm-up are logged: {text}");
    let frame = lines[0];
    assert!(frame.starts_with("frame 78.9ms"), "the total comes first: {frame}");
    assert!(frame.contains("text=77.5") && frame.contains("shaped=28"), "the phases are named: {frame}");
    assert!(frame.contains("glyphs=7380"), "the glyph count is what prepare scales with: {frame}");
    assert!(frame.contains("worst=67332us chars=1"), "the slowest line is named: {frame}");
    assert!(frame.contains("codes=1f600"), "the codepoints say which character: {frame}");
    assert!(frame.contains("face=Noto Color Emoji fallback=1"), "the face says whether it fell back: {frame}");
    assert!(frame.contains("again_worst=44us"), "the second shape says whether it was the face: {frame}");
    let warm = lines[1];
    assert!(warm.starts_with("warm 13.4ms"), "its cost is in milliseconds: {warm}");
    assert!(warm.contains("faces=JetBrainsMono Nerd Font Mono,Noto Color Emoji"), "the faces it touched: {warm}");
    assert!(warm.contains("sample=\"ação 😀\""), "the sample it warmed with: {warm}");
}
