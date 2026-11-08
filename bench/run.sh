#!/usr/bin/env bash
# Reproducible throughput benchmark: Node.js vs Corvis.
#
# Usage (Git Bash, from any directory):
#   bash bench/run.sh
#
# The prerequisites, the environment variables (CONNS, DURATION, WARMUP, FRAMEWORKS, SCENARIOS, OHA_THREADS,
# SKIP_BUILD, PORT_BASE) and the location of the Corvis binary are described in bench/README.md.
#
# Every server listens on 127.0.0.1 only.

set -euo pipefail

BENCH_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$BENCH_DIR/.." && pwd)"
RAW_DIR="$BENCH_DIR/raw"
LOG_DIR="$BENCH_DIR/logs"

CONNS="${CONNS:-100}"
DURATION="${DURATION:-15}"
WARMUP="${WARMUP:-5}"
FRAMEWORKS="${FRAMEWORKS:-corvis corvis-1t node-express node-fastify}"
SCENARIOS="${SCENARIOS:-hello json param post}"
OHA_THREADS="${OHA_THREADS-8}"
PORT_BASE="${PORT_BASE:-3200}"
HOST="127.0.0.1"

NODE_MAIN="$BENCH_DIR/node-app/dist/main.js"

POST_BODY='{"name":"Ada","email":"ada@example.com","age":36}'

log() { printf '\033[36m[bench]\033[0m %s\n' "$*"; }
die() { printf '\033[31m[bench] ERROR:\033[0m %s\n' "$*" >&2; exit 1; }

# Target directory Cargo uses for the workspace (CARGO_TARGET_DIR, a Cargo config, or <repo>/target),
# printed with forward slashes.
target_dir() {
  (cd "$ROOT_DIR" && cargo metadata --no-deps --format-version 1) |
    node -e "let d='';process.stdin.on('data',c=>d+=c).on('end',()=>console.log(JSON.parse(d).target_directory.replace(/\\\\/g,'/')))"
}

# ---------------------------------------------------------------------------
# Process management: always kill the servers we start.
# ---------------------------------------------------------------------------
SERVER_WINPIDS=()

kill_winpid() {
  local wp="$1"
  [ -n "$wp" ] || return 0
  taskkill //F //T //PID "$wp" > /dev/null 2>&1 || true
}

cleanup() {
  for wp in "${SERVER_WINPIDS[@]:-}"; do
    kill_winpid "$wp"
  done
  # In case a background memory sampler is still running.
  jobs -p | xargs -r kill > /dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

# Working set (KB) of a Windows PID, via tasklist (fast, no PowerShell).
rss_kb() {
  local wp="$1" line
  line="$(tasklist //FI "PID eq $wp" //FO CSV //NH 2> /dev/null | head -n1 || true)"
  # "corvis-app.exe","1234","Console","1","12,345 K" -> 12345
  printf '%s' "$line" | awk -F'","' '{print $5}' | tr -cd '0-9'
}

# ---------------------------------------------------------------------------
# Tools
# ---------------------------------------------------------------------------
command -v oha > /dev/null 2>&1 || die "'oha' not found. Install it with: cargo install oha --locked"
command -v node > /dev/null 2>&1 || die "'node' not found."
command -v curl > /dev/null 2>&1 || die "'curl' not found."

# ---------------------------------------------------------------------------
# 1. Build
# ---------------------------------------------------------------------------
TARGET_DIR="$(target_dir)" || die "Could not resolve the Cargo target directory (cargo metadata failed)."
[ -n "$TARGET_DIR" ] || die "Could not resolve the Cargo target directory (empty answer from cargo metadata)."
CORVIS_EXE="$TARGET_DIR/release/corvis-app.exe"
BUILD_CMD="cargo build --release -p corvis-app"

if [ -d "$BENCH_DIR/target" ]; then
  printf '[33m[bench] NOTICE:[0m bench/target is no longer used (corvis-app builds in %s). It can be deleted: rm -rf bench/target
' "$TARGET_DIR" >&2
fi

if [ "${SKIP_BUILD:-0}" != "1" ]; then
  log "Building corvis-app (--release) in $TARGET_DIR/release..."
  if ! (cd "$ROOT_DIR" && $BUILD_CMD); then
    # It can fail if another crate of the workspace is half-written. If a previous binary
    # exists we continue with it, but we record that in results.md.
    [ -f "$CORVIS_EXE" ] || die "Building corvis-app failed and there is no previous binary at $CORVIS_EXE"
    printf '[33m[bench] WARNING:[0m cargo build failed; using the existing corvis-app binary.
' >&2
    BENCH_NOTES="AVISO: cargo build falló en esta ejecución; se usó un binario de corvis-app compilado previamente ($(date -r "$CORVIS_EXE" '+%Y-%m-%d %H:%M'))."
  fi

  log "Installing dependencies and building node-app..."
  if [ ! -d "$BENCH_DIR/node-app/node_modules" ]; then
    (cd "$BENCH_DIR/node-app" && npm install --no-audit --no-fund)
  fi
  (cd "$BENCH_DIR/node-app" && npm run build --silent)
fi
[ -f "$CORVIS_EXE" ] || die "$CORVIS_EXE does not exist. Build it with: $BUILD_CMD"
[ -f "$NODE_MAIN" ] || die "$NODE_MAIN does not exist. Build it with: (cd bench/node-app && npm run build)"

rm -rf "$RAW_DIR" "$LOG_DIR"
mkdir -p "$RAW_DIR" "$LOG_DIR"

# ---------------------------------------------------------------------------
# Framework definitions
# ---------------------------------------------------------------------------
fw_label() {
  case "$1" in
    corvis) echo "Corvis (Tokio, all cores)" ;;
    corvis-1t) echo "Corvis (Tokio, 1 worker)" ;;
    node-express) echo "Node.js + Express" ;;
    node-fastify) echo "Node.js + Fastify" ;;
    *) die "Unknown framework: $1" ;;
  esac
}

# Starts the server in the background and leaves its Windows PID in $SERVER_WINPID.
start_server() {
  local fw="$1" port="$2" logf="$LOG_DIR/$1.log" pid
  case "$fw" in
    corvis) PORT="$port" "$CORVIS_EXE" > "$logf" 2>&1 & ;;
    corvis-1t) PORT="$port" TOKIO_WORKER_THREADS=1 "$CORVIS_EXE" > "$logf" 2>&1 & ;;
    node-express) PORT="$port" NODE_ENV=production node "$NODE_MAIN" > "$logf" 2>&1 & ;;
    node-fastify) PORT="$port" NODE_ENV=production node "$NODE_MAIN" --fastify > "$logf" 2>&1 & ;;
  esac
  pid=$!
  SERVER_WINPID="$(cat "/proc/$pid/winpid" 2> /dev/null || echo "")"
  [ -n "$SERVER_WINPID" ] || die "Could not get the Windows PID of $fw"
  SERVER_WINPIDS+=("$SERVER_WINPID")
}

wait_ready() {
  local port="$1" i
  for i in $(seq 1 100); do
    if curl -s -m 1 -o /dev/null "http://$HOST:$port/"; then return 0; fi
    sleep 0.2
  done
  return 1
}

# Checks that an endpoint returns exactly the expected status code and body.
check() {
  local name="$1" expected_code="$2" expected_body="$3"; shift 3
  local out code body
  out="$(curl -s -m 5 -w '\n%{http_code}' "$@")"
  code="${out##*$'\n'}"
  body="${out%$'\n'*}"
  if [ "$code" != "$expected_code" ] || [ "$body" != "$expected_body" ]; then
    die "Unexpected response from $name: code=$code body=$body (expected $expected_code $expected_body)"
  fi
}

verify_endpoints() {
  local base="http://$HOST:$1"
  check "GET /" 200 'Hello World!' "$base/"
  check "GET /json" 200 '{"id":1,"name":"Corvis","tags":["a","b","c"],"active":true}' "$base/json"
  check "GET /users/42" 200 '{"id":42}' "$base/users/42"
  check "GET /users/abc" 400 '{"message":"Validation failed (numeric string is expected)","error":"Bad Request","statusCode":400}' "$base/users/abc"
  check "POST /users" 201 "$POST_BODY" -H 'content-type: application/json' -d "$POST_BODY" "$base/users"
  check "POST /users (invalid)" 400 '{"message":["name should not be empty","email must be an email","age must not be less than 0"],"error":"Bad Request","statusCode":400}' \
    -H 'content-type: application/json' -d '{"name":"","email":"nope","age":-1}' "$base/users"
}

# oha arguments for a scenario (URL included).
oha_args() {
  local sc="$1" port="$2" base="http://$HOST:$2"
  case "$sc" in
    hello) echo "$base/" ;;
    json) echo "$base/json" ;;
    param) echo "$base/users/42" ;;
    post) echo "-m POST -T application/json -d $POST_BODY $base/users" ;;
    *) die "Unknown scenario: $sc" ;;
  esac
}

run_oha() {
  local secs="$1" fmt="$2" out="$3"; shift 3
  local extra=()
  [ -n "$OHA_THREADS" ] && extra+=(--worker-threads "$OHA_THREADS")
  # -w: wait for in-flight requests when the deadline passes (avoids false "aborted due to deadline" errors).
  oha --no-tui -w -c "$CONNS" -z "${secs}s" "${extra[@]}" --output-format "$fmt" -o "$out" "$@"
}

# ---------------------------------------------------------------------------
# 2. Measurement
# ---------------------------------------------------------------------------
RSS_JSON=""
FW_JSON=""
idx=0
for fw in $FRAMEWORKS; do
  label="$(fw_label "$fw")"
  port=$((PORT_BASE + idx))
  idx=$((idx + 1))

  log "== $label (port $port) =="
  start_server "$fw" "$port"
  wait_ready "$port" || die "$label did not respond on $HOST:$port (see $LOG_DIR/$fw.log)"
  verify_endpoints "$port"
  log "Endpoints verified with curl."

  sleep 1
  idle_kb="$(rss_kb "$SERVER_WINPID")"
  log "Idle RSS: ${idle_kb:-n/a} KB"

  sc_json=""
  for sc in $SCENARIOS; do
    # shellcheck disable=SC2046
    read -r -a args <<< "$(oha_args "$sc" "$port")"

    log "[$sc] warming up ${WARMUP}s..."
    run_oha "$WARMUP" quiet /dev/null "${args[@]}"

    log "[$sc] measuring ${DURATION}s with $CONNS connections..."
    peak_file="$LOG_DIR/$fw-$sc.rss"
    echo 0 > "$peak_file"
    (
      peak=0
      while true; do
        sleep 2
        v="$(rss_kb "$SERVER_WINPID")"
        if [ -n "$v" ] && [ "$v" -gt "$peak" ]; then peak="$v"; echo "$peak" > "$peak_file"; fi
      done
    ) &
    sampler=$!
    run_oha "$DURATION" json "$RAW_DIR/$fw-$sc.json" "${args[@]}"
    kill "$sampler" > /dev/null 2>&1 || true
    wait "$sampler" 2> /dev/null || true
    peak_kb="$(cat "$peak_file")"

    # The server must still be alive after each scenario.
    curl -s -m 2 -o /dev/null "http://$HOST:$port/" || die "$label stopped responding after scenario $sc"

    rps="$(node -e "const r=require(process.argv[1]);console.log(Math.round(r.summary.requestsPerSec))" "$RAW_DIR/$fw-$sc.json")"
    log "[$sc] ${rps} req/s (peak RSS ${peak_kb} KB)"
    sc_json="$sc_json${sc_json:+,}\"$sc\":{\"peak_kb\":\"$peak_kb\"}"
  done

  RSS_JSON="$RSS_JSON${RSS_JSON:+,}\"$fw\":{\"idle_kb\":\"$idle_kb\"${sc_json:+,}$sc_json}"
  FW_JSON="$FW_JSON${FW_JSON:+,}{\"id\":\"$fw\",\"label\":\"$label\"}"

  kill_winpid "$SERVER_WINPID"
  sleep 1
done

# ---------------------------------------------------------------------------
# 3. Report
# ---------------------------------------------------------------------------
SC_JSON="$(printf '"%s",' $SCENARIOS)"
SC_JSON="[${SC_JSON%,}]"
OS_NAME="$(powershell -NoProfile -Command "(Get-CimInstance Win32_OperatingSystem).Caption + ' ' + [Environment]::OSVersion.Version" 2> /dev/null | tr -d '\015' || uname -s)"
cat > "$RAW_DIR/meta.json" << EOF
{
  "date": "$(date '+%Y-%m-%d %H:%M:%S %z')",
  "os": "$OS_NAME",
  "node": "$(node --version)",
  "rustc": "$(rustc --version 2> /dev/null | awk '{print $2}')",
  "loadgen": "$(oha --version | head -n1) (${OHA_THREADS:-auto} hilos)",
  "conns": $CONNS,
  "duration": $DURATION,
  "warmup": $WARMUP,
  "frameworks": [$FW_JSON],
  "scenarios": $SC_JSON,
  "rss": {$RSS_JSON},
  "notes": "${BENCH_NOTES:-}"
}
EOF

node "$BENCH_DIR/report.js" "$RAW_DIR" "$BENCH_DIR/results.md"
log "Done. Results in bench/results.md"
