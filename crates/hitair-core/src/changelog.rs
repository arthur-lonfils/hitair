//! The changelog, embedded at build time and parsed into per-version sections so
//! the app can show players "what's new" — the whole history, and a one-time
//! popup of the newest section right after an update.

/// The raw `CHANGELOG.md` from the repo root.
pub const RAW: &str = include_str!("../../../CHANGELOG.md");

/// One released version's notes.
#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub date: String,
    /// The section body (kept as `### Heading` / `- bullet` lines; `**` stripped).
    pub body: String,
}

/// Released sections, newest first — the empty `[Unreleased]` placeholder and any
/// bodyless sections are skipped.
pub fn releases() -> Vec<Release> {
    let mut out: Vec<Release> = Vec::new();
    let mut cur: Option<Release> = None;

    let flush = |out: &mut Vec<Release>, r: Option<Release>| {
        if let Some(r) = r
            && !r.version.eq_ignore_ascii_case("unreleased")
            && !r.body.trim().is_empty()
        {
            out.push(r);
        }
    };

    for line in RAW.lines() {
        if let Some(rest) = line.strip_prefix("## [") {
            flush(&mut out, cur.take());
            let (version, date) = match rest.split_once(']') {
                Some((v, tail)) => (
                    v.to_string(),
                    tail.trim_start_matches([' ', '-']).trim().to_string(),
                ),
                None => (rest.trim_end_matches(']').to_string(), String::new()),
            };
            cur = Some(Release {
                version,
                date,
                body: String::new(),
            });
        } else if let Some(r) = cur.as_mut() {
            r.body.push_str(&line.replace("**", ""));
            r.body.push('\n');
        }
    }
    flush(&mut out, cur.take());
    out
}

/// The notes for one specific version, if present.
pub fn for_version(version: &str) -> Option<Release> {
    releases().into_iter().find(|r| r.version == version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_released_sections_newest_first() {
        let rels = releases();
        assert!(!rels.is_empty(), "changelog should have releases");
        // Never surface the Unreleased placeholder.
        assert!(
            !rels
                .iter()
                .any(|r| r.version.eq_ignore_ascii_case("unreleased"))
        );
        // Newest first: the top release matches the crate version.
        assert_eq!(rels[0].version, env!("CARGO_PKG_VERSION"));
        // Bodies carry their bullet content and no bold markers.
        assert!(rels[0].body.contains('-'));
        assert!(!rels[0].body.contains("**"));
    }
}
