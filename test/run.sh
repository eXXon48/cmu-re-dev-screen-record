#!/bin/sh
# Manifest checks: valid JSON, version placeholder, the jailed binary + its /usr/bin link.
# usage: sh test/run.sh   (needs python3)
set -eu
cd "$(dirname "$0")/.."
python3 - <<'PY'
import json
m = json.load(open("pkg/manifest.json"))
assert m["name"] == "cmu-re-dev-screen-record", m["name"]
assert m["version"] == "0.0.0", "version placeholder must stay 0.0.0 (mkpkg.sh rewrites it)"
assert m["restart"] is False
f, l = m["files"]
assert f == {"live": "bin/screen-record", "payload": "payload/bin/screen-record", "mode": "0755",
             "owner": "1000:1000", "type": "new", "apply": "whole-file"}, f
assert l["live"] == "/usr/bin/screen-record" and l["target"] == "bin/screen-record" and l["type"] == "link", l
PY
echo ok
