#!/usr/bin/env bash
# Benchmark of the authentication hot paths of Corvis (JWT strategy, JwtService, session middleware).
#
# Usage (Git Bash, from any directory):
#   bash bench/auth/run.sh --label before
#   bash bench/auth/run.sh --label after --compare before
#
# The scenarios, the environment variables (CONNS, DURATION, WARMUP, RATE, SCENARIOS, OHA_THREADS, SKIP_BUILD, PORT)
# and where the binary is built are described in bench/auth/README.md.
#
# The server listens on 127.0.0.1 only. Nothing here is a test: the numbers inform, nothing asserts them.

set -euo pipefail

BENCH_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$BENCH_DIR/../.." && pwd)"
RESULTS_DIR="$BENCH_DIR/results"

CONNS="${CONNS:-50}"
DURATION="${DURATION:-10}"
WARMUP="${WARMUP:-3}"
RATE="${RATE:-1000}"
OHA_THREADS="${OHA_THREADS-4}"
PORT="${PORT:-3290}"
ALL_SCENARIOS="jwt-hs256 jwt-provider-rs256 service-verify-rs256 service-sign-hs256 service-sign-rs256 session-none session-read session-write"
SCENARIOS="${SCENARIOS:-$ALL_SCENARIOS}"
HOST="127.0.0.1"
BASE="http://$HOST:$PORT"

LABEL="run"
COMPARE=""
while [ $# -gt 0 ]; do
  case "$1" in
    --label) LABEL="${2:?--label needs a value}"; shift 2 ;;
    --compare) COMPARE="${2:?--compare needs a value}"; shift 2 ;;
    *) printf 'unknown argument: %s\n' "$1" >&2; exit 2 ;;
  esac
done
case "$LABEL" in
  *[!A-Za-z0-9._-]*|'') printf 'the label may only hold letters, digits, ".", "_" and "-": %s\n' "$LABEL" >&2; exit 2 ;;
esac

log() { printf '\033[36m[auth-bench]\033[0m %s\n' "$*"; }
die() { printf '\033[31m[auth-bench] ERROR:\033[0m %s\n' "$*" >&2; exit 1; }

# The target directory Cargo uses for the workspace (CARGO_TARGET_DIR, a Cargo config, or <repo>/target), with forward
# slashes.
target_dir() {
  (cd "$ROOT_DIR" && cargo metadata --no-deps --format-version 1) |
    node -e "let d='';process.stdin.on('data',c=>d+=c).on('end',()=>console.log(JSON.parse(d).target_directory.replace(/\\\\/g,'/')))"
}

# ---------------------------------------------------------------------------
# Process management: the server we start is always stopped.
# ---------------------------------------------------------------------------
SERVER_PID=""
SERVER_WINPID=""
WORK_DIR=""

cleanup() {
  if [ -n "$SERVER_WINPID" ]; then
    taskkill //F //T //PID "$SERVER_WINPID" > /dev/null 2>&1 || true
  elif [ -n "$SERVER_PID" ]; then
    kill "$SERVER_PID" > /dev/null 2>&1 || true
  fi
  [ -n "$WORK_DIR" ] && rm -rf "$WORK_DIR"
  return 0
}
trap cleanup EXIT INT TERM

command -v oha > /dev/null 2>&1 || die "'oha' not found. Install it with: cargo install oha --locked"
command -v node > /dev/null 2>&1 || die "'node' not found."
command -v curl > /dev/null 2>&1 || die "'curl' not found."

# ---------------------------------------------------------------------------
# 1. Build
# ---------------------------------------------------------------------------
TARGET_DIR="$(target_dir)" || die "Could not resolve the Cargo target directory (cargo metadata failed)."
[ -n "$TARGET_DIR" ] || die "Could not resolve the Cargo target directory (empty answer from cargo metadata)."
EXE="$TARGET_DIR/release/corvis-auth-bench"
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) EXE="$EXE.exe" ;; esac
BUILD_CMD="cargo build --release -p corvis-auth-bench"

if [ "${SKIP_BUILD:-0}" != "1" ]; then
  log "Building corvis-auth-bench (--release) in $TARGET_DIR/release..."
  (cd "$ROOT_DIR" && $BUILD_CMD) || die "Building corvis-auth-bench failed"
fi
[ -f "$EXE" ] || die "$EXE does not exist. Build it with: $BUILD_CMD"

WORK_DIR="$(mktemp -d)"

# ---------------------------------------------------------------------------
# 2. Server and setup
# ---------------------------------------------------------------------------
PORT="$PORT" "$EXE" > "$WORK_DIR/server.log" 2>&1 &
SERVER_PID=$!
SERVER_WINPID="$(cat "/proc/$SERVER_PID/winpid" 2> /dev/null || echo "")"

ready=0
for _ in $(seq 1 100); do
  if curl -s -m 1 -o /dev/null "$BASE/__bench/stats"; then ready=1; break; fi
  kill -0 "$SERVER_PID" 2> /dev/null || break
  sleep 0.2
done
[ "$ready" = 1 ] || die "the server did not answer on $HOST:$PORT (log: $(cat "$WORK_DIR/server.log"))"

json_field() { node -e "let d='';process.stdin.on('data',c=>d+=c).on('end',()=>{const v=JSON.parse(d)[process.argv[1]];if(v===undefined)process.exit(1);console.log(v)})" "$1"; }

TOKENS="$(curl -s -m 5 "$BASE/__bench/tokens")"
HS256_TOKEN="$(printf '%s' "$TOKENS" | json_field hs256)" || die "no HS256 token from /__bench/tokens: $TOKENS"
RS256_TOKEN="$(printf '%s' "$TOKENS" | json_field rs256)" || die "no RS256 token from /__bench/tokens: $TOKENS"
SEED_HEADERS="$(curl -s -m 5 -D - -o /dev/null -X POST "$BASE/session/seed")"
SESSION_COOKIE="$(printf '%s' "$SEED_HEADERS" | tr -d '\r' | sed -n 's/^[Ss]et-[Cc]ookie: \(sid=[^;]*\).*/\1/p' | head -n1)"
[ -n "$SESSION_COOKIE" ] || die "the session seed set no cookie: $SEED_HEADERS"

# The request of a scenario, one argument per line: method, path, then header lines.
scenario_request() {
  case "$1" in
    jwt-hs256) printf '%s\n' GET /jwt/hs256 "authorization: Bearer $HS256_TOKEN" ;;
    jwt-provider-rs256) printf '%s\n' GET /jwt/provider "authorization: Bearer $RS256_TOKEN" ;;
    service-verify-rs256) printf '%s\n' GET /service/verify "authorization: Bearer $RS256_TOKEN" ;;
    service-sign-hs256) printf '%s\n' POST /service/sign ;;
    service-sign-rs256) printf '%s\n' POST /service/sign-rs256 ;;
    session-none) printf '%s\n' GET /session/none ;;
    session-read) printf '%s\n' GET /session/read "cookie: $SESSION_COOKIE" ;;
    session-write) printf '%s\n' POST /session/write "cookie: $SESSION_COOKIE" ;;
    *) die "unknown scenario: $1" ;;
  esac
}

# The status and body a scenario must answer (`*` in the body matches any text).
scenario_expected() {
  case "$1" in
    jwt-hs256|jwt-provider-rs256|service-verify-rs256) printf '%s\n' 200 '{"sub":"7"}' ;;
    service-sign-hs256|service-sign-rs256) printf '%s\n' 201 '{"ok":true}' ;;
    session-none) printf '%s\n' 200 '{"ok":true}' ;;
    session-read) printf '%s\n' 200 '{"n":*}' ;;
    session-write) printf '%s\n' 201 '{"ok":true}' ;;
  esac
}

stats() { curl -s -m 5 "$BASE/__bench/stats"; }

run_oha() {
  local out="$1"; shift
  local extra=()
  [ -n "$OHA_THREADS" ] && extra+=(--worker-threads "$OHA_THREADS")
  oha --no-tui -w "${extra[@]}" --output-format json -o "$out" "$@"
}

# ---------------------------------------------------------------------------
# 3. Measurement
# ---------------------------------------------------------------------------
ROWS="$WORK_DIR/rows.jsonl"
: > "$ROWS"
for sc in $SCENARIOS; do
  mapfile -t req < <(scenario_request "$sc")
  method="${req[0]}"; path="${req[1]}"
  curl_args=(-X "$method")
  oha_args=(-m "$method")
  for header in "${req[@]:2}"; do
    curl_args+=(-H "$header")
    oha_args+=(-H "$header")
  done
  mapfile -t expected < <(scenario_expected "$sc")

  out="$(curl -s -m 5 -w '\n%{http_code}' "${curl_args[@]}" "$BASE$path")"
  code="${out##*$'\n'}"
  body="${out%$'\n'*}"
  # shellcheck disable=SC2053
  if [ "$code" != "${expected[0]}" ] || [[ "$body" != ${expected[1]} ]]; then
    die "scenario $sc: unexpected answer from $method $path: status $code, body $body (expected ${expected[0]} ${expected[1]})"
  fi

  log "[$sc] warming up ${WARMUP}s..."
  run_oha /dev/null -c "$CONNS" -z "${WARMUP}s" "${oha_args[@]}" "$BASE$path"

  log "[$sc] measuring ${DURATION}s with $CONNS connections..."
  before="$(stats)" || die "scenario $sc: the stats endpoint did not answer"
  run_oha "$WORK_DIR/$sc-closed.json" -c "$CONNS" -z "${DURATION}s" "${oha_args[@]}" "$BASE$path"
  after="$(stats)" || die "scenario $sc: the stats endpoint did not answer after the measurement"

  log "[$sc] latency at ${RATE} req/s for ${DURATION}s..."
  run_oha "$WORK_DIR/$sc-rate.json" -c "$CONNS" -q "$RATE" -z "${DURATION}s" "${oha_args[@]}" "$BASE$path"
  stats > /dev/null || die "scenario $sc: the server stopped answering"

  node -e '
    const fs = require("fs");
    const [name, closedFile, rateFile, before, after] = process.argv.slice(1);
    const closed = JSON.parse(fs.readFileSync(closedFile, "utf8"));
    const rate = JSON.parse(fs.readFileSync(rateFile, "utf8"));
    const b = JSON.parse(before), a = JSON.parse(after);
    const requests = a.requests - b.requests;
    const ms = (s) => (s == null ? null : Math.round(s * 1e6) / 1e3);
    const p = rate.latencyPercentiles || {};
    const statuses = Object.assign({}, closed.statusCodeDistribution || {});
    console.log(JSON.stringify({
      scenario: name,
      requests_per_sec: Math.round(closed.summary.requestsPerSec),
      p50_ms: ms(p.p50), p90_ms: ms(p.p90), p99_ms: ms(p.p99), max_ms: ms(rate.summary.slowest),
      server_requests: requests,
      allocations_per_request: requests > 0 ? Math.round((a.allocations - b.allocations) / requests * 10) / 10 : null,
      bytes_per_request: requests > 0 ? Math.round((a.bytes - b.bytes) / requests) : null,
      status_codes: statuses,
    }));
  ' "$sc" "$WORK_DIR/$sc-closed.json" "$WORK_DIR/$sc-rate.json" "$before" "$after" >> "$ROWS"
  log "[$sc] $(tail -n1 "$ROWS")"
done

# ---------------------------------------------------------------------------
# 4. Results
# ---------------------------------------------------------------------------
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*)
    OS_NAME="$(powershell -NoProfile -Command "(Get-CimInstance Win32_OperatingSystem).Caption + ' ' + [Environment]::OSVersion.Version" 2> /dev/null | tr -d '\015')"
    CPU_NAME="$(powershell -NoProfile -Command "(Get-CimInstance Win32_Processor | Select-Object -First 1).Name" 2> /dev/null | tr -d '\015')"
    ;;
  *)
    OS_NAME="$(uname -sr)"
    CPU_NAME="$(sed -n 's/^model name[[:space:]]*: //p' /proc/cpuinfo 2> /dev/null | head -n1)"
    ;;
esac
COMMIT="$(cd "$ROOT_DIR" && git rev-parse --short HEAD)$(cd "$ROOT_DIR" && git diff --quiet HEAD -- crates bench/auth || echo '+changes')"

mkdir -p "$RESULTS_DIR"
node -e '
  const fs = require("fs");
  const [rowsFile, out, compareFile, meta] = process.argv.slice(1);
  const rows = fs.readFileSync(rowsFile, "utf8").trim().split("\n").filter(Boolean).map((l) => JSON.parse(l));
  const result = Object.assign(JSON.parse(meta), { scenarios: rows });
  fs.writeFileSync(out, JSON.stringify(result, null, 2) + "\n");
  const n = (v) => (v == null ? "n/a" : String(v));
  console.log("");
  console.log(`Label ${result.label}, ${result.date}, ${result.os}, ${result.cpu}, rustc ${result.rustc}, ${result.oha}, commit ${result.commit}`);
  console.log(`CONNS=${result.params.conns} DURATION=${result.params.duration}s WARMUP=${result.params.warmup}s RATE=${result.params.rate} OHA_THREADS=${result.params.oha_threads || "auto"}`);
  console.log("");
  let base = null;
  if (compareFile && fs.existsSync(compareFile)) base = JSON.parse(fs.readFileSync(compareFile, "utf8"));
  if (base) {
    const old = Object.fromEntries(base.scenarios.map((r) => [r.scenario, r]));
    console.log("| Scenario | req/s before | req/s after | p99 before | p99 after | allocations / request before → after | bytes / request before → after |");
    console.log("|---|---|---|---|---|---|---|");
    for (const r of rows) {
      const o = old[r.scenario] || {};
      console.log(`| \`${r.scenario}\` | ${n(o.requests_per_sec)} | ${r.requests_per_sec} | ${n(o.p99_ms)} ms | ${n(r.p99_ms)} ms | ${n(o.allocations_per_request)} → ${n(r.allocations_per_request)} | ${n(o.bytes_per_request)} → ${n(r.bytes_per_request)} |`);
    }
  } else {
    console.log("| Scenario | req/s | p50 | p90 | p99 | max | allocations / request | bytes / request |");
    console.log("|---|---|---|---|---|---|---|---|");
    for (const r of rows) {
      console.log(`| \`${r.scenario}\` | ${r.requests_per_sec} | ${n(r.p50_ms)} ms | ${n(r.p90_ms)} ms | ${n(r.p99_ms)} ms | ${n(r.max_ms)} ms | ${n(r.allocations_per_request)} | ${n(r.bytes_per_request)} |`);
    }
  }
' "$ROWS" "$RESULTS_DIR/$LABEL.json" "${COMPARE:+$RESULTS_DIR/$COMPARE.json}" "$(cat << EOF
{
  "label": "$LABEL",
  "date": "$(date '+%Y-%m-%d %H:%M:%S %z')",
  "os": "$OS_NAME",
  "cpu": "$CPU_NAME",
  "rustc": "$(rustc --version 2> /dev/null | awk '{print $2}')",
  "oha": "$(oha --version | head -n1)",
  "commit": "$COMMIT",
  "params": { "conns": $CONNS, "duration": $DURATION, "warmup": $WARMUP, "rate": $RATE, "oha_threads": "${OHA_THREADS}" }
}
EOF
)"
log "Results in bench/auth/results/$LABEL.json"
