#!/bin/sh
# The interactive case, for finders without a filter mode (television):
# start in a pseudo-terminal, load the corpus, search QUERY, take the top hit.
#   bench/interactive.sh CORPUS [QUERY...]
# Needs hyperfine; tv is skipped when it is not installed.
set -e
here=$(cd "$(dirname "$0")" && pwd)
corpus=${1:?usage: bench/interactive.sh CORPUS [QUERY...]}
shift
me=$here/../target/release/lvim-fuzzy
[ $# -gt 0 ] || set -- lvim tuiview "src lua" pickerfuzzyengine
for q in "$@"; do
    echo "== $q"
    set -- -n fzf "python3 $here/ptyrun.py sh -c \"fzf --query '$q' --sync --bind start:accept < $corpus > /dev/null\"" \
        -n lvim-fuzzy-cli "python3 $here/ptyrun.py sh -c \"$me --query '$q' --sync --bind start:accept < $corpus > /dev/null\""
    if command -v tv >/dev/null; then
        set -- "$@" -n tv "python3 $here/ptyrun.py sh -c \"tv --source-command 'cat $corpus' --input '$q' --take-1 --no-preview > /dev/null\""
    fi
    hyperfine -N -w 2 -r 10 "$@"
done
