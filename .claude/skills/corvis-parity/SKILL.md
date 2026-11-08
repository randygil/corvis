---
name: corvis-parity
description: Build the three Corvis port apps (consi, facten, houseplan) and run their apidiff suites against the Node originals, with the expected results and cleanup. Use to check that a framework change keeps behavioral parity, before archiving a change or committing framework work.
---

# Parity check of the three port apps

The ports live in `C:\Users\Usuario\Devel\corvis-tests\corvis\{consi-backend,facten-api,houseplan-api}` and use the
framework through `path =` dependencies. Each suite sends the same requests to the Node original (A) and to the port
(B), compares status, headers and bodies, then compares the final database contents.

Run the suites **one at a time** (shared Docker container and fixed ports), and always finish with the cleanup step.

## 0. Prerequisites

- Docker Desktop running and the Postgres container up:
  ```bash
  docker ps >/dev/null 2>&1 || powershell -NoProfile -Command "Start-Process 'C:\Program Files\Docker\Docker\Docker Desktop.exe'"
  # wait until `docker ps` works, then:
  docker start corvis-pg && docker exec corvis-pg pg_isready -U postgres
  ```
- The project settings set `CARGO_TARGET_DIR` to the framework's target dir. **The ports keep their own target
  dirs** (their scripts expect `./target/...`), so override it when building them, as below.
- **Testing a branch in a worktree:** back up each port's `Cargo.toml` and `Cargo.lock`, rewrite the `corvis`,
  `corvis-orm` and `corvis-swagger` path dependencies to `<worktree>/crates/...`, and restore both files byte for byte
  at the end (`cmp` against the backup; the ports are not git repos). Before building, clean only the framework
  crates each port actually depends on (`corvis-cli` is in none, `corvis-swagger` is not in houseplan), for example
  `CARGO_TARGET_DIR="$PWD/target" cargo clean --release -p corvis -p corvis-macros -p corvis-orm -p corvis-orm-macros`;
  otherwise a build may reuse artifacts compiled from another checkout with the same crate names.

## 1. Build and unit-test each port (release)

```bash
for p in consi-backend facten-api houseplan-api; do
  cd /c/Users/Usuario/Devel/corvis-tests/corvis/$p
  CARGO_TARGET_DIR="$PWD/target" cargo build --release --bins 2>&1 | grep -E "^(warning|error)" -A6 | head -40
  CARGO_TARGET_DIR="$PWD/target" cargo test 2>&1 | grep -E "^test result|FAILED|panicked"
done
```
Expected: no warnings; consi 93 tests, facten 254 (+2 ignored), houseplan 70 + 10, all passing.

## 2. consi (Nest on 4000, port on 4100, webhook sink on 4191)

```bash
cd /c/Users/Usuario/Devel/corvis-tests/tmp/consi
./restart-all.sh                                   # resets consi_nest / consi_corvis from consi_tpl, starts all
./run-diff.sh http://127.0.0.1:4100 > full.log 2>&1; tail -1 full.log; grep -n "✗" full.log | head
cd /c/Users/Usuario/Devel/corvis-tests/corvis/consi-backend/apidiff
node ../../../tools/apidiff/apidiff.mjs dbdiff --db-a consi_nest --db-b consi_corvis | tail -2
```
Expected: `832 checks: 832 equal, 0 warnings, 0 different` and `databases are equivalent`.
Cleanup: `bash /c/Users/Usuario/Devel/corvis-tests/tmp/consi/kill-port.sh 4000 4001 4100 4191`.

Note: `tmp/consi/*.sh` are helper scripts outside version control; `start-corvis.sh` runs
`consi-backend/target/release/consi-backend.exe`.

## 3. facten (Nest on 3001, port on 3101)

```bash
cd /c/Users/Usuario/Devel/corvis-tests/corvis/facten-api
bash scripts/apidiff.sh > ../../tmp/facten/final.txt 2>&1
grep -n "✗" ../../tmp/facten/final.txt | head; grep -E "checks:|routes|identical|equivalent" ../../tmp/facten/final.txt | tail -4
```
Expected: `727 checks: 727 equal`, `A: 74 routes, B: 74 routes`, `route tables are identical`,
`databases are equivalent`. Cleanup: `bash /c/Users/Usuario/Devel/corvis-tests/tmp/facten/run.sh --stop`.

## 4. houseplan (Nest on 3200, port on 3300, mocks on 3399)

```bash
cd /c/Users/Usuario/Devel/corvis-tests/corvis/houseplan-api
PROFILE=release bash scripts/diff.sh > ../../tmp/houseplan/final.txt 2>&1
grep -n "✗\|!!" ../../tmp/houseplan/final.txt | head; grep -E "checks:|routes|identical|equivalent" ../../tmp/houseplan/final.txt | tail -4
```
`PROFILE=release` matters: without it the script runs a possibly stale debug binary.
Expected: `636 checks: 636 equal`, `A: 31 routes, B: 31 routes`, identical routes, equivalent databases.

The check count depends on the time of day: `13-nudges` exercises the per-minute dispatcher, which stays silent during
`QUIET_HOURS` (default `22:00-08:00`, Caracas time = UTC-4). Outside quiet hours the run gives 636 checks; during them
the dispatcher sends nothing, the `buttons` group of `13-nudges` loses its 38 checks and the run gives 598 (still all
equal, same routes, equivalent databases). Both numbers are valid; for full coverage run between 08:00 and 22:00
Caracas time.

## 5. Cleanup (always)

```powershell
taskkill /F /IM houseplan-api.exe 2>$null
Get-NetTCPConnection -State Listen -LocalPort 3001,3101,3200,3300,3399,4000,4001,4100,4191 -ErrorAction SilentlyContinue |
  Where-Object { $_.OwningProcess -gt 4 } |
  ForEach-Object { try { Stop-Process -Id $_.OwningProcess -Force -ErrorAction Stop } catch {} }
# consi's helper scripts keep running after their server dies (restart-all.sh, start-corvis.sh, the webhook sink);
# a leftover consi-backend.exe locks the release binary for the next build ("Acceso denegado", os error 5).
Get-CimInstance Win32_Process |
  Where-Object { $_.CommandLine -match 'restart-(all|nest)\.sh|start-corvis\.sh|start-nest2?\.sh|sink\.mjs|consi-backend\.exe' } |
  ForEach-Object { try { Stop-Process -Id $_.ProcessId -Force -ErrorAction Stop } catch {} }
```
Then check that nothing listens on those ports and that no process command line mentions `consi-backend`,
`facten-api` or `houseplan-api`.

## Reading a difference

- **Regression:** the port's response changed because of the framework change. Report it; do not patch the scenario.
- **Nondeterministic order:** the original does not define an order (for example a `findMany` without `orderBy`).
  The scenario should compare it as a set or by count, as `consi-backend/apidiff/99-final.mjs` does for webhook
  deliveries. Confirm by checking the original's code before calling it nondeterministic.
- **Intended difference:** documented in the port's README under justified differences (for example consi's
  evidence uploads, which only accept PDF/PNG/JPG for security). The scenario must use inputs both sides accept.
