#!/usr/bin/env bash
set -euo pipefail
root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
flags=()
case "${1:-}" in
  --offline) flags+=(--offline); shift ;;
esac
if (($#)); then echo "Usage: ./test.sh [--offline]" >&2; exit 2; fi
python3 -m unittest discover -s scripts/tests -v
export SDL_VIDEODRIVER=dummy SDL_AUDIODRIVER=dummy SDL_RENDER_DRIVER=software
cargo test --release --locked --package advance-liberty --package advance-aot-supplement "${flags[@]}"
export ADVANCE_AOT_TEST_GCC="$(command -v "${CC:-cc}")"
cargo test --release --locked --package advance-aot-supplement "${flags[@]}" compiled_pc_and_shift_operations_match_oracle -- --ignored
