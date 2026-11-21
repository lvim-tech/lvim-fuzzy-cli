# lvim-fuzzy-cli

`lvim-fuzzy` is a fuzzy finder for the terminal, compatible with
[fzf](https://github.com/junegunn/fzf), built on the
[lvim-fuzzy](https://github.com/lvim-tech/lvim-fuzzy-nvim) scoring model, with the
look of lvim-picker — without needing Neovim.

Status: early development.

- Accepts fzf's command line options, its key bindings and the `--bind`
  grammar. Environment variables use the `LVIM_FUZZY_` prefix only:
  `LVIM_FUZZY_DEFAULT_COMMAND`, `LVIM_FUZZY_DEFAULT_OPTS`,
  `LVIM_FUZZY_DEFAULT_OPTS_FILE`, `LVIM_FUZZY_CONFIG`.
- Extended search syntax: `'exact`, `^prefix`, `suffix$`, `!negation`, `a | b`.
- The lvim look: panels, a title bar, a button bar and file icons; colours
  come from lvim-colorscheme themes (see [Themes](#themes)).
- Previews, tried in order for each item (the first that can show it wins):
  a real embedded Neovim (files), the built-in file preview (syntect colours
  from the theme's palette, ~150 languages), the `--preview` command (anything
  that is not a file), and your own previewers from the config.
- Full screen, `--height` inline mode, or `--popup` (fzf's `--tmux` is an
  alias): a floating pane of Zellij or tmux — whichever it runs in — with
  stdin and the result passed through; outside both it runs in place.
  `--popup=[center|top|bottom|left|right][,W%][,H%]`.
- Layouts, multi-select, mouse.
- F1 shows the active keys, F2 the modes.
- File icons as Material IMAGES where the terminal takes kitty placeholders (kitty, ghostty, zellij
  built with them), Nerd glyphs elsewhere: `[icons] mode = "auto" | "image" | "font"`. The set is shared
  with lvim-icons and lvim-fuzzy-commander in `~/.local/share/lvim-icons/material`; `lvim-fuzzy --install-icons`
  (run by `scripts/install.sh`) puts it there when it is missing or older.

## Shell integration

```sh
eval "$(lvim-fuzzy --zsh)"      # ~/.zshrc
eval "$(lvim-fuzzy --bash)"     # ~/.bashrc
lvim-fuzzy --fish | source      # ~/.config/fish/config.fish
```

Nothing is bound by default: the keys come from the config's `[shell]`.

```toml
[shell]
completion = "**"            # <Tab> completion trigger (zsh, fish); absent = off
height     = "50%"           # below the prompt, outside Zellij / tmux
popup      = "center,80%"    # a floating pane in Zellij / tmux

[shell.keys]                 # key = "MODE[:insert|cd|replace]"
"ctrl-t" = "files"           # insert the chosen items at the cursor (default)
"ctrl-r" = "shell-history"   # the shell's own history (multi-line kept)
"alt-c"  = "dirs:cd"         # cd into the chosen directory
"ctrl-g" = "git-status"
"alt-z"  = "zoxide:cd"
```

MODE is any built-in or user mode, or `shell-history`. Keys: `ctrl-x`,
`alt-x`, `ctrl-alt-x`. With `completion`, `**<Tab>` completes the word before
it (directories after `cd` / `pushd` / `rmdir`; `src/**<Tab>` searches `src/`).
`LVIM_FUZZY_BIN`, `LVIM_FUZZY_HEIGHT` and `LVIM_FUZZY_POPUP` override at run time.

## Modes

A mode is a preset: where the items come from, how they are previewed, what the
filter reads. Every mode starts as a subcommand; words after it are the
starting query:

```sh
lvim-fuzzy files
lvim-fuzzy grep health          # live ripgrep, query "health"
lvim-fuzzy grep-filter TODO     # grep once, then filter the results
lvim-fuzzy git-log
lvim-fuzzy mode                 # the list of every mode
lvim-fuzzy --mode history       # the same, as an option
```

Inside the finder, **F2** opens the mode menu: pick another mode and the finder
switches in place, keeping the query.

Built-in modes: `files`, `dirs`, `grep`, `grep-filter`, `git-files`,
`git-status`, `git-log`, `git-branches`, `git-stash`, `git-tags`, `git-reflog`,
`history`, `processes`, `ports`, `man`, `env`, `commands`, `tmux`, `zellij`,
`docker`, `docker-images`, `systemd`, `systemd-user`, `journal`, `ssh`, `fonts`,
`packages`, `zoxide`, `kube-pods`, `gh-prs`, `gh-issues`. A mode whose program
is missing is listed with what it needs.

### Grep results: the filter scope

For `path:line:text` results the filter reads one part, cycled with **F4**
and shown in the title and on the footer: `line` (the line's text, default),
`path` (the file path) or `file` (the whole file, exact terms).

In a live mode (`grep`: every change re-runs ripgrep) there is nothing to
filter yet: **F3** freezes the results and clears the input, which from then
on filters them (and F4's scope applies); F3 again goes back to the live grep
with its query. The keys: F1 help, F2 modes, F3 grep ⇄ filter, F4 scope,
F5 theme.

**Enter** opens the item in the editor (grep results at their line; `editor`
in the config, `nvim` by default, `{file}` `{line}` `{col}` in a template);
**ctrl-y** prints it. When the output goes to a program (the shell keys,
`$(lvim-fuzzy)`, scripts) enter prints, as in fzf; an item that is not a file
is printed too. A mode says what open means for its items with `open`: the
built-in ones switch the tmux / Zellij session, show a commit, tag or stash,
switch a branch, open a man page, connect with ssh, show a unit's status or a
container's log, open a PR / issue in the browser; files and grep results go
to the editor. `open` is an action like any other, so `[keys]` can put it on
another key.

## Themes

Like the other lvim-tech CLIs (secmon, keyforge, ...): theme files live in
`~/.config/lvim-fuzzy/themes/<Name>.toml` and one is picked with
`theme = "<Name>"` in the config (`--theme` and `$LVIM_FUZZY_THEME` — a name or
a path — override it, in that order). The files are lvim-colorscheme's
`extras/lvim_fuzzy/<Name>.toml` (`scripts/build lvim_fuzzy`); **themer** installs
them: on every theme switch it fetches the file and rewrites the `theme` line.
By hand:

```sh
mkdir -p ~/.config/lvim-fuzzy/themes
cp lvim-colorscheme/extras/lvim_fuzzy/*.toml ~/.config/lvim-fuzzy/themes/
```

Without a theme (or with a missing one) the lvim layout is drawn in the
terminal's own colours; a role a theme lacks falls back the same way — a theme
never stops the finder. **F5** picks a theme live; `ctrl-w` in the picker saves
it to the config.

## Config

`~/.config/lvim-fuzzy/config.toml` (or `$LVIM_FUZZY_CONFIG`, `--config PATH`,
`--no-config`):

```toml
theme = "LvimNord_dark"
bar = "bottom"              # the button bar: top | bottom
title_pos = "left"          # left | center | right
options = ["--cycle"]       # any lvim-fuzzy / fzf options

# every popup (notifications, F1 help, F2 modes, F5 themes): the border in
# Neovim's order: top-left, top, top-right, right, bottom-right, bottom,
# bottom-left, left ("" leaves a side out), or a style: rounded, sharp, bold,
# double, block, thinblock, space, none. The default is eight spaces; the
# colour comes from the theme, the background is the terminal's a step off.
[popup]
border = [" ", " ", " ", " ", " ", " ", " ", " "]

[preview]
order = ["bat", "nvim", "file", "command"]   # built-in names + your previewers
nvim = "nvim"
nvim_appname = "nvim-preview"

[previewers.bat]            # your own preview: bat, glow, eza --tree, ...
command  = "bat --color=always --style=numbers --highlight-line {line} {file}"
for      = "file"           # file | dir | any
requires = ["bat"]          # skipped when bat is missing
# extensions = ["md"]       # only these file types

[keys]
"ctrl-o" = "execute(less {})"
# "ctrl-l" = "open"         # open (enter by default) on another key as well

[modes.todo]                # a user mode: in the F2 menu and `lvim-fuzzy todo`
description = "TODO and FIXME comments"
title = "TODO"
command = "rg -n --no-heading 'TODO|FIXME'"
target = "{1}:{2}"          # file:line for the Neovim / file preview
scope = "line"
# open = "nvim +{2} {1}"    # what `open` (enter) does here: a command, or an
#                           # action: execute(...), become(...), accept
# live = true               # re-run the command with {q} on every change
# pattern = true            # run once with the starting query, then filter
# preview = "bat --color=always {1}"
# requires = ["rg"]
# options = ["--multi"]
```

The config is checked before anything starts and every problem is reported,
with its file and line. A user mode may not take a built-in mode's name.
`lvim-fuzzy config check` only checks; `lvim-fuzzy config path` prints the path.
`lvim-fuzzy --config-template` prints every setting, commented out (also in
`config.example.toml`); `[styles.<look>]` and `[show]` are described there.

## Doctor

`lvim-fuzzy doctor` lists every problem at once: the config (with file and
line), the themes directory and each theme file (missing roles included), the
theme a start would use, the themer target, modes and previewers whose programs
are missing, Neovim and its `lvim-fuzzy.preview` module, the pool, the usual programs
(rg, fd, git, bat) and the terminal (TERM, truecolor). It exits 1 on an error.

## Warm preview (pool)

The Neovim preview starts Neovim with your config, which costs a few hundred
milliseconds before the first file shows (the deferred plugins, the
colorscheme, treesitter). `lvim-fuzzy pool` keeps one Neovim started and warmed
for the next finder, which attaches to it instead of starting its own:

- there is always one free instance; a finder takes it and the pool starts
  warming the next one;
- when the finder closes, its instance is ended — a used instance is never
  given back, the free one is always fresh;
- a finder that finds no free instance (no pool running, or one still warming)
  starts its own Neovim as before: the pool only makes the preview faster.

Warming means attaching a UI for a moment, so the deferred plugins load before
any finder needs them. The pool restarts its idle instances when your Neovim
config or this config changes (hidden state directories aside), or on
`lvim-fuzzy pool reload`.

```sh
lvim-fuzzy pool            # run it (foreground)
lvim-fuzzy pool status     # its instances: free / warming / taken
lvim-fuzzy pool reload     # restart the idle instances
lvim-fuzzy pool stop
```

To start it at login, a systemd user service is in
`contrib/systemd/lvim-fuzzy-pool.service` (instructions inside); without
systemd, `lvim-fuzzy pool >/dev/null 2>&1 &!` in `.zprofile` does the same.
The sockets live in `$XDG_RUNTIME_DIR/lvim-fuzzy` (`LVIM_FUZZY_POOL_DIR`
overrides). `[pool] enabled = false` or `--no-pool` makes a finder ignore it;
`lvim-fuzzy doctor` reports its state.

## Compatibility and speed

`cargo test --test compat` runs the same input, queries and options through
`fzf --filter` and `lvim-fuzzy --filter` and compares the results (it is
skipped when fzf is not installed). `bench/filter.py [LINES] [PROGRAM...]`
times `--filter` on a generated list of paths; on 1 000 000 lines:

| query | fzf 0.75 | lvim-fuzzy |
|---|---:|---:|
| `lvim` | 469 ms | 76 ms |
| `tuiview` | 169 ms | 60 ms |
| `src lua !md` | 279 ms | 64 ms |
| `'theme ^src .rs$` | 132 ms | 55 ms |
| `pickerfuzzyengine` | 122 ms | 54 ms |
| `src lua !md` (`--no-sort`) | 969 ms | 61 ms |
| `zzzz` | 148 ms | 58 ms |

Without a source (no stdin, no default command) the finder walks the current
directory itself, as fzf does — in parallel, on several threads, like fzf's
walker. One difference by design: a directory reached through several paths
(symlinks) is listed ONCE, always under the same path — its real path when the
walk reaches it anyway, else the first symlink to it in path order. fzf lists it
again under every path (a home directory with flatpak and mise links: 3.2 M
entries in fzf, 2.8 M here). `--walker=file,follow,hidden,dup` walks every path,
as fzf, with the same count. On that home directory (cache warm): fzf 3.7–5.2 s,
lvim-fuzzy 0.8–0.9 s (`dup`: ~1.1 s).

`--typos=N` (1 or 2) lets a fuzzy term miss that many of its characters
(`--typos=auto`: none up to 4 characters, 1 up to 8, 2 beyond); an exact match
always ranks before one with a typo, and the default (0) is fzf's behaviour.

The search engine is [lvim-fuzzy-core](https://github.com/lvim-tech/lvim-fuzzy-core),
the same as the lvim-fuzzy Neovim plugin's; its `bench/run.sh` times the
matcher alone against frizbee (and other matchers) on synthetic paths, real
paths, code lines and Cyrillic text. A match window wider than
`--exact-window=N` characters (default 4096) is matched greedily in linear
time; `--exact-window=0` scores every line exactly.

television has no filter mode, so `bench/interactive.sh CORPUS [QUERY...]`
times the interactive case with hyperfine: start in a pseudo-terminal, load
the 1 000 000 lines, search, take the top hit (`--sync --bind start:accept`;
tv: `--input --take-1`):

| query | fzf 0.75 | lvim-fuzzy | television 0.15.9 |
|---|---:|---:|---:|
| `lvim` | 166 ms | 162 ms | 388 ms |
| `tuiview` | 157 ms | 131 ms | 333 ms |
| `src lua` | 178 ms | 151 ms | 370 ms |
| `pickerfuzzyengine` | 157 ms | 129 ms | 262 ms |

With a file on stdin `--filter` maps it instead of reading it, splits the
records on every core and puts the lines straight into the engine's store (one
buffer, no string per line); the finder keeps per line only what differs from
the line itself (--with-nth, --nth, --ansi).

`--listen[=[ADDR:]PORT|SOCKET]` serves the finder over HTTP as fzf's does:
`POST /` runs actions, `GET /` returns the state as JSON; set
`LVIM_FUZZY_API_KEY` to require it as `X-API-Key`.

## Install

    scripts/install.sh

builds the release binary and installs `~/.local/bin/lvim-fuzzy`, the short
name `lfz`, the man page (`man lvim-fuzzy`) and, when `~/.config/zsh` holds
zsh modules, `86-lvim-fuzzy.zsh` with the completion and your `[shell.keys]`;
otherwise it prints the line for `.zshrc` / `.bashrc` / `config.fish`.
With an `lvim-colorscheme` checkout next to it (or `LVIM_COLORSCHEME=path`) it
also copies all 48 themes into `~/.config/lvim-fuzzy/themes`, so F5 lists them;
themer still installs and switches the current one.
`--zsh` / `--bash` / `--fish` complete modes, subcommands, options and their
values (themes and user modes are read live).

## Build

```sh
cargo build --release
./target/release/lvim-fuzzy --help
```

## License

BSD-3-Clause

### The preview plugins, for other front ends

Every preview that is not plain text — a picture, a video, a PDF, a font, a document, a sound, a directory, a CSV
table, an archive, a binary file — is made by the preview plugin (`lvim-fuzzy-plugin-preview`): which engine shows a file
type is that plugin's configuration, lfz only draws the answer. `lvim-fuzzy preview --plugin [--plugins JSON]
[--host NAME] [--size COLSxROWS] [--px WxH] [--skip N] -- PATH` prints the plugin's answer as a JSON array of
instructions (`[{"op":"preview","facts":…,"missing":…,"picture":{"path","width","height"},"body":…}]`, or
`[{"op":"pass"}]`); the picture is a PNG in `$XDG_CACHE_HOME/lvim-media/`. lvc, lfz and the Neovim picker
(lvim-picker) all ask it, so they show a file the same way. lfz's own settings for a plugin go in
`[plugins.<name>]` of its config; the provider chain starts with `plugin` (`--preview-order plugin,nvim,file,command`
is the default).

### Where the preview sits

`alt-p` turns the preview through right → left → up → hidden → right, `alt-P` the other way, `alt-e` hides and
shows it (`rotate-preview`, `rotate-preview-back`, `toggle-preview`: bind them to other keys with `--bind`). The
Neovim picker (lvim-picker) turns its preview through the same places with `<C-n>` / `<C-p>` and hides it with
`<C-e>`. A preview put at the bottom with `--preview-window=down` is off the cycle: forward from it hides it, back
goes up.

### Opening a file

Enter on a file opens it the way lvim-fuzzy-commander and the Neovim picker (lvim-fuzzy-picker) do — one rule, the
open plugin's (`lvim-fuzzy-plugin-open`): which program opens which type is its configuration (default: the editor for
text, `xdg-open` for the rest). lfz's own layer goes in `[plugins.lvim-fuzzy-plugin-open]` of its config:

```toml
[plugins.lvim-fuzzy-plugin-open.types.pictures]
mime   = ["image/*"]
engine = "imv"
[plugins.lvim-fuzzy-plugin-open.engines.imv]
kind    = "command"
command = "imv {}"        # {} = the file, quoted for the shell; without it the file is appended
```

`lvim-fuzzy open --list` shows which program opens which type as it is configured now (and what is not installed);
`lvim-fuzzy preview --list` does the same for the previews.

A viewer starts detached and the finder stays open (so several pictures can be opened in a row); with several
files marked the text ones go to the editor together, the others each to their program. A program that is not
installed is said in a message and nothing is opened. `lvim-fuzzy open --resolve [--plugins JSON] -- PATH` prints the
plugin's decision as JSON (`{"action":"editor"}`, `{"action":"exec","cmd":"..."}`, `{"action":"missing",...}` or
`{"action":"none"}`), which is how the Neovim picker asks.
