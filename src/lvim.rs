//! The lvim look (lvim-fuzzy-ui), plus what reads this program's options.

pub use lvim_fuzzy_ui::lvim::*;

/// The icon for `path` under the options: None with --no-icons; the user's
/// own (--icon / [icons]: the file name, then the extension, `dir` for a
/// directory) before the built-in ones.
pub fn icon_for<'a>(
    opts: &'a crate::options::Options,
    path: &str,
    is_dir: bool,
) -> Option<(&'a str, &'a str)> {
    if !opts.icons || opts.hidden.contains("icons") {
        return None;
    }
    if !opts.icon_map.is_empty() {
        let name = path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(path)
            .to_ascii_lowercase();
        let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
        let keys: &[&str] = if is_dir { &["dir"] } else { &[&name, ext] };
        for k in keys {
            if let Some((_, g, c)) = opts.icon_map.iter().find(|(key, _, _)| key == k) {
                let (_, builtin) = icon(path, is_dir);
                return Some((g.as_str(), c.as_deref().unwrap_or(builtin)));
            }
        }
    }
    Some(icon(path, is_dir))
}
