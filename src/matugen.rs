use std::path::{Path, PathBuf};
use std::time::Duration;

use winit::event_loop::EventLoopProxy;

use crate::config;
use crate::pty::UserEvent;
use crate::theme::Theme;

pub const TEMPLATE: &str = include_str!("../assets/koral.matugen");

fn colors_path() -> Option<PathBuf> {
    Some(config::home()?.join("koral/matugen.colors"))
}

pub fn theme(enabled: bool) -> Theme {
    let colors = colors_path().filter(|_| enabled).and_then(|path| std::fs::read_to_string(path).ok());
    colors.map_or_else(Theme::default, |colors| Theme::with_colors(&colors))
}

/// Matugen's config text with Koral's template in it, or None when it is already there.
pub fn add_template(source: &str, template: &Path, output: &Path) -> Option<String> {
    if source.contains("[templates.koral]") {
        return None;
    }
    let mut updated = source.to_string();
    if !updated.is_empty() {
        if !updated.ends_with('\n') {
            updated.push('\n');
        }
        updated.push('\n');
    }
    updated.push_str(&format!("[templates.koral]\ninput_path = {template:?}\noutput_path = {output:?}\n"));
    Some(updated)
}

/// Makes the next matugen run write Koral's colours.
pub fn install() -> Result<(), String> {
    let home = config::home().ok_or("Not saved: no config folder")?;
    // Dotfile managers link this file: writing through the link keeps it theirs.
    let matugen = std::fs::canonicalize(home.join("matugen/config.toml")).map_err(|_| "No matugen config found")?;
    let source = std::fs::read_to_string(&matugen).map_err(|error| format!("Cannot read matugen config: {error}"))?;

    let template = home.join("koral/matugen.template");
    let write = |error: std::io::Error| format!("Not saved: {error}");
    config::save(&template, TEMPLATE).map_err(write)?;
    if let Some(updated) = add_template(&source, &template, &home.join("koral/matugen.colors")) {
        config::save(&matugen, &updated).map_err(write)?;
    }
    Ok(())
}

fn modified() -> Option<std::time::SystemTime> {
    std::fs::metadata(colors_path()?).ok()?.modified().ok()
}

// shortcut: polls once a second, swap for inotify if the wake-ups ever show up in a profile.
pub fn watch(proxy: EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || {
        let mut seen = modified();
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let now = modified();
            if now != seen {
                seen = now;
                if proxy.send_event(UserEvent::Colors).is_err() {
                    return;
                }
            }
        }
    });
}
