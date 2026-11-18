use std::time::Duration;

pub fn duration(value: Duration) -> String {
    let seconds = value.as_secs();
    match seconds {
        0..10 => format!("{:.1}s", value.as_secs_f32()),
        10..60 => format!("{seconds}s"),
        60..3600 => format!("{}m {}s", seconds / 60, seconds % 60),
        _ => format!("{}h {}m", seconds / 3600, seconds / 60 % 60),
    }
}

pub fn lines(count: usize) -> String {
    match count {
        1 => "1 line".to_string(),
        2..1000 | 0 => format!("{count} lines"),
        _ => format!("{:.1}k lines", count as f32 / 1000.0),
    }
}
