use std::time::Duration;

/// Print duration as HH:MM:SS format
pub(crate) fn format_duration(duration: &Duration) -> String {
    let seconds = duration.as_secs();
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let remaining_minutes = minutes % 60;
    let remaining_seconds = seconds % 60;

    format!("{hours:02}:{remaining_minutes:02}:{remaining_seconds:02}")
}
