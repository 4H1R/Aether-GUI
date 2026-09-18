#!/usr/bin/env bash
# Compatibility wrapper; npm run fetch:core works on Windows as well.
set -euo pipefail
node "$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)/scripts/fetch-aether.mjs"
