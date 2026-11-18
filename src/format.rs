use std::time::Duration;

pub fn duration(value: Duration) -> String {
    // Round to the unit the text shows, and let the rounded value pick the format,
    // so 9.96s reads 10s and 59.96s reads 1m 0s.
    let tenths = (value.as_millis() + 50) / 100;
    let seconds = ((value.as_millis() + 500) / 1000) as u64;
    if tenths < 100 {
        return format!("{}.{}s", tenths / 10, tenths % 10);
    }
    match seconds {
        10..60 => format!("{seconds}s"),
        60..3600 => format!("{}m {}s", seconds / 60, seconds % 60),
        _ => {
            let minutes = ((value.as_millis() + 30_000) / 60_000) as u64;
            format!("{}h {}m", minutes / 60, minutes % 60)
        }
    }
}

pub fn lines(count: usize) -> String {
    match count {
        1 => "1 line".to_string(),
        2..1000 | 0 => format!("{count} lines"),
        _ => format!("{:.1}k lines", count as f32 / 1000.0),
    }
}
