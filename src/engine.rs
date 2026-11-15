//! Item store and the background matcher: the search itself is
//! lvim-fuzzy-core's, over a store of the items' match targets.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;

use lvim_fuzzy_core::search;

use crate::item::Items;

pub use lvim_fuzzy_core::{Hit, Hits, Scope, narrows};

/// How to match and order: core's options (the matcher config, sort, --tac,
/// the tiebreaks, the grep scope).
pub type MatchSettings = lvim_fuzzy_core::Options;

/// With --nth the grep scopes do not apply (the target is the selected
/// fields, not `path:line:text`).
fn scope_for(items: &Items, scope: Scope) -> Scope {
    if items.nth { Scope::All } else { scope }
}

/// Compute the ordered hits for `query` over `items`, or None if cancelled.
pub fn run_match(
    items: &Items,
    query: &str,
    s: &MatchSettings,
    cancel: Option<(&AtomicU64, u64)>,
) -> Option<Hits> {
    run_match_in(items, query, s, cancel, None)
}

/// run_match, optionally over a subset: `subset` = (the previous matches,
/// the first item index added since).
pub fn run_match_in(
    items: &Items,
    query: &str,
    s: &MatchSettings,
    cancel: Option<(&AtomicU64, u64)>,
    subset: Option<(&[u32], usize)>,
) -> Option<Hits> {
    let s = MatchSettings {
        scope: scope_for(items, s.scope),
        ..s.clone()
    };
    search(items.store(), query, &s, cancel, subset)
}

/// Shared state between the UI thread and the matcher thread.
pub struct Engine {
    pub items: Arc<RwLock<Items>>,
    /// Bumped (under the items' write lock) whenever the items are replaced
    /// (reload, change-nth, --tail): earlier matches and the store no longer
    /// say anything about them.
    pub resets: Arc<AtomicU64>,
    pub result: Arc<Mutex<Option<(u64, Hits, usize)>>>,
    pub generation: Arc<AtomicU64>,
    tx: Sender<(u64, String, bool, Scope)>,
}

impl Engine {
    pub fn new(settings: MatchSettings, wake: Sender<()>) -> Engine {
        let items: Arc<RwLock<Items>> = Arc::new(RwLock::new(Items::default()));
        let resets = Arc::new(AtomicU64::new(0));
        let result = Arc::new(Mutex::new(None));
        let generation = Arc::new(AtomicU64::new(0));
        let (tx, rx): (
            Sender<(u64, String, bool, Scope)>,
            Receiver<(u64, String, bool, Scope)>,
        ) = channel();
        {
            let items = items.clone();
            let result = result.clone();
            let generation = generation.clone();
            let resets = resets.clone();
            thread::spawn(move || {
                // the last complete match: (query, scope, items reset count,
                // item count, the matching indices)
                let mut last: Option<(String, Scope, u64, usize, Vec<u32>)> = None;
                while let Ok(mut job) = rx.recv() {
                    while let Ok(j) = rx.try_recv() {
                        job = j;
                    }
                    let (g, query, sort, scope) = job;
                    if generation.load(Ordering::Relaxed) != g {
                        continue;
                    }
                    // searched under the read lock: the store and the count
                    // that goes with it stay as they are meanwhile
                    let guard = items.read().unwrap();
                    let reset = resets.load(Ordering::Relaxed);
                    let n = guard.len();
                    let scope = scope_for(&guard, scope);
                    let settings = MatchSettings {
                        sort,
                        scope,
                        ..settings.clone()
                    };
                    let subset = last
                        .as_ref()
                        .filter(|(q, sc, r, m, _)| {
                            *sc == scope && *r == reset && *m <= n && narrows(q, &query)
                        })
                        .map(|(_, _, _, m, idx)| (&idx[..], *m));
                    let found = search(
                        guard.store(),
                        &query,
                        &settings,
                        Some((&generation, g)),
                        subset,
                    );
                    drop(guard);
                    if let Some(h) = found {
                        // kept in item order: the next pass then reads the
                        // store front to back
                        last = h.indices().map(|i| (query.clone(), scope, reset, n, i));
                        *result.lock().unwrap() = Some((g, h, n));
                        let _ = wake.send(());
                    }
                }
            });
        }
        Engine {
            items,
            resets,
            result,
            generation,
            tx,
        }
    }

    /// Request a new match; returns its generation.
    pub fn request(&self, query: &str, sort: bool, scope: Scope) -> u64 {
        let g = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = self.tx.send((g, query.to_string(), sort, scope));
        g
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fields::Delimiter;
    use crate::item::ItemBuilder;
    use crate::matcher::{CaseMode, MatchConfig, Scheme, Tiebreak};

    fn items_of(bld: &ItemBuilder, lines: Vec<String>) -> Items {
        let mut items = Items::default();
        for (i, l) in lines.into_iter().enumerate() {
            items.push(bld.build(i as u32, l));
        }
        items
    }

    #[test]
    fn items_keep_their_texts_in_the_store() {
        let plain = ItemBuilder {
            delimiter: Delimiter::Awk,
            with_nth: None,
            nth: None,
            ansi: false,
        };
        let items = items_of(&plain, vec!["a/b.rs:3:fn x".into(), "plain".into()]);
        // a plain item holds no text: the store has it
        assert!(items.at(0).extra.is_none());
        assert_eq!(items.at(0).raw(), "a/b.rs:3:fn x");
        assert_eq!(items.at(1).display(), "plain");
        // --with-nth: the line is kept, the store holds what is shown
        let with = ItemBuilder {
            with_nth: Some(crate::fields::parse_nth("2").unwrap()),
            ..plain
        };
        let mut items = items_of(&with, vec!["one two three".into()]);
        assert_eq!(items.at(0).raw(), "one two three");
        assert_eq!(items.at(0).display(), "two ");
        assert_eq!(items.at(0).target(), "two ");
        // --tail keeps the last items, renumbered
        items.push(with.build(1, "x y z".into()));
        items.keep_last(1);
        assert_eq!(items.len(), 1);
        assert_eq!(items.at(0).index, 0);
        assert_eq!(items.at(0).raw(), "x y z");
    }

    fn settings(scope: Scope) -> MatchSettings {
        MatchSettings {
            cfg: MatchConfig {
                case: CaseMode::Smart,
                extended: true,
                exact: false,
                normalize: true,
                scheme: Scheme::Default,
                typos: 0,
                exact_window: lvim_fuzzy_core::EXACT_WINDOW,
            },
            sort: true,
            tac: false,
            tiebreak: vec![Tiebreak::Length],
            scope,
        }
    }

    #[test]
    fn grep_scopes() {
        let dir = std::env::temp_dir().join(format!("lvf-scope-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("health.lua");
        let b = dir.join("other.lua");
        std::fs::write(&a, "local h = vim.health\nh.start('x')\n").unwrap();
        std::fs::write(&b, "-- mentions health here\nreturn 1\n").unwrap();
        let bld = ItemBuilder {
            delimiter: Delimiter::Awk,
            with_nth: None,
            nth: None,
            ansi: false,
        };
        let items = items_of(
            &bld,
            vec![
                format!("{}:2:h.start('x')", a.display()),
                format!("{}:2:return 1", b.display()),
                format!("{}:1:-- mentions health here", b.display()),
            ],
        );
        assert!(items.at(0).grep.is_some());
        let ids = |scope| -> Vec<u32> {
            let h = run_match(&items, "health", &settings(scope), None).unwrap();
            let mut v: Vec<u32> = (0..h.len()).map(|i| h.get(i).unwrap()).collect();
            v.sort();
            v
        };
        assert_eq!(ids(Scope::Line), vec![2]); // only the line that says it
        assert_eq!(ids(Scope::Path), vec![0]); // only health.lua's line
        assert_eq!(ids(Scope::File), vec![0, 1, 2]); // both files mention it
        // `file` is exact: scattered letters across a file do not count
        let h = run_match(&items, "hrt", &settings(Scope::File), None).unwrap();
        assert_eq!(h.len(), 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn incremental_matches_equal_full_matches() {
        let bld = ItemBuilder {
            delimiter: Delimiter::Awk,
            with_nth: None,
            nth: None,
            ansi: false,
        };
        let words = [
            "lvim",
            "fuzzy",
            "picker",
            "src",
            "main",
            "Config",
            "théme",
            "плъгин",
            "test",
        ];
        let items = items_of(
            &bld,
            (0..4000u32)
                .map(|i| {
                    let w = |k: u32| words[((i * 7 + k * 13) % words.len() as u32) as usize];
                    format!("{}/{}_{}.{}", w(1), w(2), i, w(3))
                })
                .collect(),
        );
        let s = settings(Scope::All);
        let order = |h: &Hits| {
            (0..h.len())
                .map(|i| h.get(i).unwrap())
                .collect::<Vec<u32>>()
        };
        let chains: [&[&str]; 5] = [
            &["l", "lv", "lvi", "lvim", "lvim f", "lvim fu"],
            &["s", "sr", "src", "src ^"],
            &["th", "thé", "théme"],
            &["п", "пл", "плъг"],
            &["'te", "'tes", "'test"],
        ];
        for chain in chains {
            let mut prev: Option<(String, Vec<u32>)> = None;
            for q in chain {
                let full = run_match(&items, q, &s, None).unwrap();
                let subset = prev
                    .as_ref()
                    .filter(|(p, _)| narrows(p, q))
                    .map(|(_, idx)| (&idx[..], items.len()));
                let inc = run_match_in(&items, q, &s, None, subset).unwrap();
                // the same items in the same order
                assert_eq!(order(&full), order(&inc), "query {q:?}");
                prev = match &full {
                    Hits::List(v) => Some((q.to_string(), v.iter().map(|x| x.idx).collect())),
                    _ => None,
                };
            }
        }
        // what may not narrow
        assert!(!narrows("!te", "!tes"));
        assert!(!narrows("a", "a | b"));
        assert!(!narrows("", "a"));
        assert!(narrows("lv", "lvim"));
    }

    #[test]
    fn grep_shape_parses() {
        assert_eq!(crate::item::grep_shape("a/b.rs:12:3:fn x"), Some((6, 12)));
        assert_eq!(crate::item::grep_shape("a/b.rs:12:fn x"), Some((6, 10)));
        assert_eq!(crate::item::grep_shape("no shape here"), None);
    }
}
