#!/usr/bin/env bash
# Reproduce the eight negative review probes in a NEW disposable clone.
# Usage: bash reproduce.sh /path/to/octoscode /private/tmp/new-review-clone
# Prerequisite: the source repository contains the pinned PR objects.
set -euo pipefail
source_repo="${1:?source repository required}"
review_clone="${2:?new clone path required}"
artifact_dir="$(cd -- "$(dirname -- "$0")" && pwd)"
if [ -e "$review_clone" ]; then
    echo "Refusing to modify an existing path: $review_clone" >&2
    exit 2
fi
git clone --shared --no-checkout "$source_repo" "$review_clone"
git -C "$review_clone" checkout --detach 0c223656926f80509c6d5592ad092230c0e65b10
git -C "$review_clone" cherry-pick --no-commit 9bcf4099c2719cd8ee63090a1849a2c6f3766999 276c866af758dd75f2f47a57590c7da35b700a61
python3 - "$review_clone" "$artifact_dir" <<'PY'
from pathlib import Path
import json, sys
clone, evidence = map(Path, sys.argv[1:])
for module in ('store', 'transport'):
    path = clone / 'src' / (module + '.rs')
    text = path.read_text()
    marker = 'mod tests {'
    assert text.count(marker) == 1, (path, text.count(marker))
    include = '\n    include!(' + json.dumps(str(evidence / (module + '-repros.rs'))) + ');'
    path.write_text(text.replace(marker, marker + include, 1))
PY
cd -- "$review_clone"
set +e
CARGO_INCREMENTAL=0 cargo test --locked --lib outer_review_ -- --test-threads=1 --nocapture > review-probes.log 2>&1
probe_status=$?
set -e
cat review-probes.log
if [ "$probe_status" -ne 101 ]; then
    echo "Unexpected cargo exit: $probe_status; inspect review-probes.log" >&2
    exit 1
fi
python3 - review-probes.log <<'PY'
from pathlib import Path
import sys
text = Path(sys.argv[1]).read_text()
assert '0 passed; 8 failed; 0 ignored;' in text, 'Did not reproduce all eight failures'
print('All eight negative probes reproduced. This demonstrates defects, not passing product tests.')
PY
