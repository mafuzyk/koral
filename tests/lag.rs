use koral::lag::{self, Cost};

/// The log is how a stutter gets measured, so only the slow frames may reach it.
#[test]
fn a_slow_frame_writes_one_line_and_a_fast_one_none() {
    let path = std::env::temp_dir().join(format!("koral-lag-{}.log", std::process::id()));
    let _ = std::fs::remove_file(&path);
    // SAFETY: this test binary holds one test, and nothing else reads the variable.
    unsafe { std::env::set_var("KORAL_FRAME_LOG", &path) };

    let mut cost = Cost { total: 3.0, ..Cost::default() };
    lag::note(&cost);
    cost.total = 12.5;
    cost.text = 9.0;
    cost.shaped = 48;
    lag::note(&cost);

    let text = std::fs::read_to_string(&path).expect("the log was written");
    let _ = std::fs::remove_file(&path);
    assert_eq!(text.lines().count(), 1, "only the slow frame is logged: {text}");
    assert!(text.contains("frame 12.5ms"), "the total comes first: {text}");
    assert!(text.contains("text=9.0") && text.contains("shaped=48"), "the phases are named: {text}");
}
