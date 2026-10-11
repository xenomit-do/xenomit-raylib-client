use std::fmt::Display;

/// Info-log each distinct `key` once per session (diagnostics).
pub(crate) fn log_once(key: &'static str, msg: impl Display) {
    use std::sync::Mutex;
    static SEEN: Mutex<Option<std::collections::HashSet<&'static str>>> = Mutex::new(None);
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if seen
        .get_or_insert_with(std::collections::HashSet::new)
        .insert(key)
    {
        ::log::info!("{key}: {msg}");
    }
}
