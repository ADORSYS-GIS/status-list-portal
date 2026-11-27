#!/usr/bin/env bash
# Prints a version's entry in CHANGELOG.md, without its heading, for the GitHub release.
# Fails if there is none, so the release workflow checks for it before building anything.
#
#   bash scripts/release-notes.sh 0.2.0
set -euo pipefail

readonly CHANGELOG=CHANGELOG.md

version=${1:?usage: release-notes.sh <version>}

if [ ! -f "$CHANGELOG" ]; then
    echo "There is no ${CHANGELOG}: release-plz writes it in the release pull request." >&2
    exit 1
fi

# From the version's heading, such as "## [0.2.0](...) - 2026-11-26", to the next one,
# without the blank lines around it. The closing bracket keeps 0.2.0 from matching
# 0.2.0-rc.1.
entry=$(awk -v heading="## [${version}]" '
    index($0, heading) == 1 { found = 1; next }
    found && /^## / { exit }
    found && NF { if (first == 0) first = NR; last = NR }
    found { lines[NR] = $0 }
    END { if (first) for (i = first; i <= last; i++) print lines[i] }
' "$CHANGELOG")

if [ -z "$entry" ]; then
    echo "${CHANGELOG} has no entry for ${version}." >&2
    exit 1
fi
printf '%s\n' "$entry"
