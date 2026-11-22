//! `--help` and `--man`: one text, printed plainly or as a man page.

pub const HELP: &str = r#"lvim-fuzzy-cli - a fuzzy finder for the terminal, compatible with fzf,
with lvim-picker's look

Usage: lvim-fuzzy-cli [MODE [QUERY...]] [options]
       lvim-fuzzy-cli mode | doctor | config [check|path]

  It reads items from stdin (or runs a mode's source, or walks the current
  directory) and prints the chosen ones. fzf's options, keys and --bind actions
  work as in fzf; the extensions are marked with *.

SUBCOMMANDS *
  MODE [QUERY]            Start a mode (files, grep, git-log, ... or a user mode)
  mode                    The list of every mode; Enter starts one
  doctor                  Every problem: config, themes, modes, Neovim, terminal
  config check|path       Check the config / print where it is read from

SEARCH
  -e, --exact             Exact match                 +x, --no-extended
  -i, --ignore-case       Case-insensitive            +i, --no-ignore-case
      --smart-case        Smart case (default)        --literal (no normalising)
      --scheme=SCHEME     default | path | history    --tiebreak=CRI[,..]
  -n, --nth=N[,..]        Fields to match             --with-nth=N[,..]
      --accept-nth=N[,..] Fields to print             -d, --delimiter=STR
  +s, --no-sort           Keep the input order        --tail=NUM
      --disabled          No search (live sources)    --track, --id-nth=N[,..]
      --scope=S *         Grep results: line | path | file | all
      --typos=N|auto *    Fuzzy terms may miss N characters (auto: by length)
      --exact-window=N *  Score exactly up to N chars, greedily past (0: always)

INPUT / OUTPUT
      --read0, --print0   NUL-separated               --ansi, --sync
      --command=CMD *     Source command               --mode=NAME *
      --remote=R[:PATH] * The files of a remote directory (the remote plugin walks it)

LOOK
      --theme=NAME *      An lvim theme (~/.config/lvim-fuzzy/themes)
      --title=TEXT *      --title-pos=left|center|right *   --bar=top|bottom *
      --color=SPEC        Colours over the theme's      --no-color
      --height=[~]H[%]    Below the cursor              --min-height=H
      --popup[=OPTS] *    Floating pane in Zellij / tmux (--tmux is an alias)
      --margin=TRBL       TRBL | TB,RL | T,RL,B | T,R,B,L
      --hide=PARTS *      --show=PARTS *  title, counter, search-label, footer,
                          icons, preview-title, separator

LIST
  -m, --multi[=MAX]       Multi-select                  --cycle, --raw
      --wrap, --wrap-sign --marker, --tabstop
      --scroll-off, --hscroll-off, --no-hscroll, --freeze-left
      --scrollbar[=C], --no-scrollbar                  --jump-labels=CHARS

INPUT SECTION
      --prompt=STR, --ghost=TEXT, --no-input, --filepath-word

PREVIEW
      --preview=CMD       A command ({} {+} {q} {n} {1} ...)
      --preview-window=OPT, --preview-wrap-sign
      --preview-nvim[=T] *  A real embedded Neovim (T: file[:line[:col]] template)
      --target=T *        The item's file[:line[:col]] for the Neovim / file preview
      --preview-order=L * nvim,file,command and [previewers.*] from the config
      --nvim=PROG *, --nvim-appname=NAME *
      --icon-mode=auto|image|font *  Material images where the terminal takes them (default auto)
      --no-zellij-images *  this zellij does not take kitty placeholders
      --install-icons *   put the Material icons into ~/.local/share/lvim-icons/material (if newer)
      --no-pool *         Always start the preview's Neovim (not one from `lvim-fuzzy-cli pool`)

HEADER
      --header=STR, --header-lines=N

SCRIPTING
  -q, --query=STR   -1, --select-1   -0, --exit-0   -f, --filter=STR
      --print-query, --expect=KEYS, --bind=BINDINGS, --bind-help
      --history=FILE, --history-size=N
      --listen[=[ADDR:]PORT|SOCKET], --listen-unsafe     (LVIM_FUZZY_API_KEY)
      --with-shell=STR, --walker=OPTS, --walker-root=DIR, --walker-skip=DIRS
                          --walker: file,dir,follow,hidden (fzf's) and dup * (walk a
                          directory again under every path to it, as fzf)

SHELL INTEGRATION
      --zsh, --bash, --fish   Print the script; keys come from [shell] in the config

KEYS IN THE FINDER *
  F1 keys   F2 modes   F3 grep/filter   F4 filter scope   F5 theme
  alt-p / alt-P preview side (right, left, up, hidden)
  alt-e hide / show the preview
  ctrl-] focus the Neovim preview

FILES AND ENVIRONMENT
  ~/.config/lvim-fuzzy/config.toml   (LVIM_FUZZY_CONFIG, --config, --no-config)
  ~/.config/lvim-fuzzy/themes/       (themer installs them)
  LVIM_FUZZY_DEFAULT_COMMAND, LVIM_FUZZY_DEFAULT_OPTS, LVIM_FUZZY_DEFAULT_OPTS_FILE,
  LVIM_FUZZY_THEME. Commands it runs get LVIM_FUZZY_QUERY, LVIM_FUZZY_POS,
  LVIM_FUZZY_MATCH_COUNT, LVIM_FUZZY_PREVIEW_LINES/COLUMNS and the other
  LVIM_FUZZY_* variables.

EXIT STATUS
  0 chosen   1 no match   2 error   130 aborted
"#;

/// The help text as a man page (roff).
pub fn man() -> String {
    let mut out = String::from(
        ".TH LVIM-FUZZY 1\n.SH NAME\nlvim-fuzzy \\- a fuzzy finder for the terminal, compatible with fzf\n",
    );
    for block in HELP.split("\n\n").skip(1) {
        let mut lines = block.lines();
        let Some(first) = lines.next() else { continue };
        let is_heading = first.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            && first
                .trim_end_matches(" *")
                .chars()
                .all(|c| c.is_ascii_uppercase() || c == ' ' || c == '/');
        if first.starts_with("Usage:") {
            out.push_str(".SH SYNOPSIS\n.nf\n");
            out.push_str(&esc(first));
            out.push('\n');
        } else if is_heading {
            out.push_str(&format!(".SH {}\n.nf\n", first.trim()));
        } else {
            out.push_str(".nf\n");
            out.push_str(&esc(first));
            out.push('\n');
        }
        for l in lines {
            out.push_str(&esc(l));
            out.push('\n');
        }
        out.push_str(".fi\n");
    }
    out
}

fn esc(s: &str) -> String {
    let s = s.replace('\\', "\\\\").replace('-', "\\-");
    if s.starts_with('.') || s.starts_with('\'') {
        format!("\\&{s}")
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn man_has_sections() {
        let m = super::man();
        assert!(m.starts_with(".TH LVIM-FUZZY 1"));
        assert!(m.contains(".SH SEARCH") && m.contains(".SH EXIT STATUS"));
    }
}
