pub(crate) fn normalize_slug(raw: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in raw.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if matches!(c, '-' | '_' | ' ' | '\t' | '\n' | '\r') {
            if !prev_dash && !out.is_empty() {
                out.push('-');
                prev_dash = true;
            }
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.len() > 36 {
        out.truncate(36);
        while out.ends_with('-') {
            out.pop();
        }
    }
    out
}

pub(crate) fn fallback_slug(name: &str, goal: &str, now_unix: u64) -> String {
    let joined = format!("{} {}", name, goal);
    let s = normalize_slug(&joined);
    if s.is_empty() {
        format!("project-{}", now_unix)
    } else {
        s
    }
}
