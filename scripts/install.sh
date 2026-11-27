#!/bin/sh
# Build lvim-fuzzy-cli and install it for the current user:
#   ~/.local/bin/lvim-fuzzy-cli    the binary
#   ~/.local/bin/lfz               a short name (symlink)
#   ~/.local/share/man/man1/       lvim-fuzzy-cli.1 and lfz.1
#   ~/.config/zsh/86-lvim-fuzzy.zsh   completion + [shell.keys], when ~/.config/zsh
#                                     holds zsh modules (else: the line to add)
# PREFIX (default ~/.local) and ZSH_MODULES override the places.
set -e
cd "$(dirname "$0")/.."
prefix=${PREFIX:-$HOME/.local}
bin=$prefix/bin
man=$prefix/share/man/man1

cargo build --release
target=${CARGO_TARGET_DIR:-target}   # CARGO_TARGET_DIR: where cargo builds (a full disk, a shared target)
mkdir -p "$bin" "$man"
install -m 755 "$target/release/lvim-fuzzy-cli" "$bin/lvim-fuzzy-cli"
ln -sf lvim-fuzzy-cli "$bin/lfz"
"$bin/lvim-fuzzy-cli" --man > "$man/lvim-fuzzy-cli.1"
ln -sf lvim-fuzzy-cli.1 "$man/lfz.1"
echo "installed: $bin/lvim-fuzzy-cli, $bin/lfz, $man/lvim-fuzzy-cli.1"
# the program used to be called lvim-fuzzy (the name read like the family, not the terminal finder): the old file and man
# page go, and what called it by that name is pointed at the new one
if [ -e "$bin/lvim-fuzzy" ] || [ -L "$bin/lvim-fuzzy" ]; then rm -f "$bin/lvim-fuzzy" "$man/lvim-fuzzy.1"; echo "removed the old name: $bin/lvim-fuzzy"; fi
unit=${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/lvim-fuzzy-pool.service
if [ -f "$unit" ] && grep -q "/bin/lvim-fuzzy pool" "$unit"; then
    sed -i "s|/bin/lvim-fuzzy pool|/bin/lvim-fuzzy-cli pool|" "$unit"
    echo "systemd: $unit now runs lvim-fuzzy-cli;  run: systemctl --user daemon-reload && systemctl --user restart lvim-fuzzy-pool"
fi
# the Material icons, shared with lvim-icons and lvim-commander: written when missing there or older
"$bin/lvim-fuzzy-cli" --install-icons

# every lvim theme, from an lvim-colorscheme checkout next to this one (themer
# installs and switches the current one; existing files are left alone)
themes=${XDG_CONFIG_HOME:-$HOME/.config}/lvim-fuzzy/themes
src=${LVIM_COLORSCHEME:-../lvim-colorscheme}/extras/lvim_fuzzy
if [ -d "$src" ]; then
    mkdir -p "$themes"
    n=0
    for f in "$src"/*.toml; do
        [ -e "$themes/$(basename "$f")" ] || { cp "$f" "$themes/"; n=$((n + 1)); }
    done
    echo "themes: $n added to $themes ($(ls "$themes" | wc -l) in all)"
else
    echo "themes: no lvim-colorscheme checkout at $src (LVIM_COLORSCHEME=path); themer installs the current one"
fi

line='(( $+commands[lvim-fuzzy-cli] )) && eval "$(lvim-fuzzy-cli --zsh)"'
modules=${ZSH_MODULES:-${XDG_CONFIG_HOME:-$HOME/.config}/zsh}
if [ -d "$modules" ]; then
    cat > "$modules/86-lvim-fuzzy.zsh" <<ZSH
# 86-lvim-fuzzy — lvim-fuzzy-cli (lfz): completion of modes, options and values,
# and the keys from [shell.keys] in ~/.config/lvim-fuzzy/config.toml.
# After 30-completion (compinit) and 85-zinit (the ZLE plugins).
__lvf_load() { $line; }
__lvf_load
# zsh-vi-mode (and other ZLE plugins) set their keymaps at the first prompt:
# bind the keys once more then, after them, and drop the hook
__lvf_late() { add-zsh-hook -d precmd __lvf_late; __lvf_load; }
autoload -Uz add-zsh-hook && add-zsh-hook precmd __lvf_late
# the man page (lvim-fuzzy-cli.1 / lfz.1); MANPATH is set explicitly elsewhere
[[ ":\$MANPATH:" == *":$prefix/share/man:"* ]] || export MANPATH="$prefix/share/man:\$MANPATH"
ZSH
    echo "zsh: $modules/86-lvim-fuzzy.zsh"
else
    echo "zsh: add to ~/.zshrc after compinit:  $line"
    echo "bash: add to ~/.bashrc:  eval \"\$(lvim-fuzzy-cli --bash)\""
    echo "fish: add to config.fish:  lvim-fuzzy-cli --fish | source"
fi
case ":$PATH:" in *":$bin:"*) ;; *) echo "note: $bin is not in PATH";; esac
# the plugin host and the preview / open plugins lfz runs through: fetched and built when missing
boot=../lvim-fuzzy-plugin/scripts/bootstrap.sh
if [ -f "$boot" ]; then sh "$boot" host preview open; else curl -fsSL https://raw.githubusercontent.com/lvim-tech/lvim-fuzzy-plugin/main/scripts/bootstrap.sh | sh -s -- host preview open; fi
