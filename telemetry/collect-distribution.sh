#!/usr/bin/env bash
# Distribution-side stats snapshot (ADR-026 component A). No client code —
# this collects what package infrastructure already publishes:
#   crates.io downloads, GitHub repo stats, Homebrew tap install analytics.
# Run weekly (cron or workflow); emits one JSON document to stdout.
set -euo pipefail

UA="ganglion-stats-collector (RobotDen/ganglion telemetry/collect-distribution.sh)"
CRATE="gang"
REPO="RobotDen/ganglion"

crates=$(curl -sf -H "User-Agent: $UA" "https://crates.io/api/v1/crates/$CRATE" \
  | python3 -c "import json,sys; d=json.load(sys.stdin)['crate']; print(json.dumps({'downloads': d['downloads'], 'recent_downloads': d.get('recent_downloads'), 'max_version': d['max_version']}))")

github=$(curl -sf -H "User-Agent: $UA" ${GITHUB_TOKEN:+-H "Authorization: Bearer $GITHUB_TOKEN"} \
  "https://api.github.com/repos/$REPO" \
  | python3 -c "import json,sys; d=json.load(sys.stdin); print(json.dumps({'stars': d['stargazers_count'], 'forks': d['forks_count'], 'watchers': d['subscribers_count']}))")

# Homebrew analytics are published by Homebrew from ITS users' opt-out
# telemetry; third-party taps appear once installs register.
brew=$(curl -sf -H "User-Agent: $UA" \
  "https://formulae.brew.sh/api/analytics/install/homebrew-core/365d.json" 2>/dev/null \
  | python3 -c "import json,sys; d=json.load(sys.stdin); items=[i for i in d.get('items',[]) if 'robotden' in i.get('formula','')]; print(json.dumps(items))" \
  || echo "[]")

python3 - "$crates" "$github" "$brew" <<'PYEOF'
import json, sys, datetime
print(json.dumps({
    "collected_at": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
    "crates_io": json.loads(sys.argv[1]),
    "github": json.loads(sys.argv[2]),
    "homebrew": json.loads(sys.argv[3]),
}, indent=2))
PYEOF
