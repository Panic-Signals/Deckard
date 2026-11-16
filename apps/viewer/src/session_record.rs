/// The default file name for a session recording: `session-<project>-<stamp>.wav`, or
/// `session-<stamp>.wav` when there is no project name.
pub fn session_filename(project: Option<&str>, stamp: &str) -> String {
    match project.filter(|p| !p.is_empty()) {
        Some(project) => format!("session-{}-{stamp}.wav", sanitize(project)),
        None => format!("session-{stamp}.wav"),
    }
}

/// The local time as `YYYY-MM-DD-HHMM`, from the same clock as `recorder::timestamp`.
pub fn session_stamp() -> String {
    // `recorder::timestamp` is `YYYYMMDDTHHMMSS`; reorder its pieces.
    let ts = crate::recorder::timestamp();
    format!("{}-{}-{}-{}", &ts[0..4], &ts[4..6], &ts[6..8], &ts[9..13])
}

/// Replaces each character outside `[A-Za-z0-9._-]` with `-`, collapsing runs of `-`.
fn sanitize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        let c = if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' };
        if !(c == '-' && out.ends_with('-')) {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_the_default_filename() {
        assert_eq!(session_filename(Some("NightSet"), "2026-11-16-2104"), "session-NightSet-2026-11-16-2104.wav");
        assert_eq!(session_filename(None, "2026-11-16-2104"), "session-2026-11-16-2104.wav");
        assert_eq!(session_filename(Some(""), "2026-11-16-2104"), "session-2026-11-16-2104.wav");
        // Filesystem-unfriendly characters in the project name are made safe.
        assert_eq!(session_filename(Some("Night Set/2"), "2026-11-16-2104"), "session-Night-Set-2-2026-11-16-2104.wav");
    }
}
