#!/usr/bin/env bash
# Negative controls: re-introduce a known defect, require the named test to FAIL,
# restore the file from git, require the test to PASS again and the tree to be clean.
set -u
cd "$(dirname "$0")/.."
run() { cargo test -q --release -p zstd-rs "$@" >/dev/null 2>&1; }
control() {
  local name=$1 file=$2 from=$3 to=$4; shift 4
  if ! grep -qF -- "$from" "$file"; then echo "CONTROL NOT APPLIED ($name): pattern missing"; return 1; fi
  python3 - "$file" "$from" "$to" <<'PY'
import sys; p, a, b = sys.argv[1:]; s = open(p).read(); open(p, 'w').write(s.replace(a, b, 1))
PY
  if git diff --quiet -- "$file"; then echo "CONTROL NOT APPLIED ($name): no diff"; return 1; fi
  if run "$@"; then red="NO (defect not detected!)"; else red="yes"; fi
  git checkout -q -- "$file"
  git diff --quiet -- "$file" || { echo "RESTORE FAILED ($name)"; return 1; }
  if run "$@"; then green="yes"; else green="NO"; fi
  echo "$name: red=$red, green after restore=$green"
}
control "dictionary distance capped at window" zstd-rs/src/encode/matchers/mod.rs \
  "    if m + min > dlen {" "    if m + min > dlen || ip + dlen - m > b.max_dist {" \
  --test ours_to_c dictionary_actually_helps_small_messages
control "ll==0 repeat-offset rule ignored" zstd-rs/src/encode/seqstore.rs \
  "    let (code, new) = if ll > 0 {" "    let (code, new) = if ll > 0 || true {" \
  --test ours_to_c all_levels_all_sizes
control "output cap not enforced" zstd-rs/src/decode/execute.rs \
  "        if end > self.limit {" "        if end > usize::MAX - 1 {" \
  --test c_to_ours output_limit_is_enforced
control "Huffman end-of-stream check removed" zstd-rs/src/huf/dtable.rs \
  "        if !r.finished() {" "        if false && !r.finished() {" \
  --lib huf::ctable::tests::streams_must_be_consumed_exactly
control "decoder repeat-offset update wrong" zstd-rs/src/decode/sequences.rs \
  "                    reps = [o, reps[0], reps[2]];" "                    reps = [o, reps[2], reps[0]];" \
  --test c_to_ours decodes_c_frames_all_levels
control "jobs forget unknown repeat offsets" zstd-rs/src/encode/mod.rs \
  "            (false, _) => [0, 0, 0]," "            (false, _) => [1, 4, 8]," \
  --test par jobs_are_executor_independent_and_valid
