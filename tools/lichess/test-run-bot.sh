#!/bin/sh
# Check that run-bot.sh cannot leak a bot process.
#
# The supervisor's real job is not restarting the bot, which is easy, but
# guaranteeing that nothing survives it, which is not. A leaked `multiprocessing`
# worker keeps talking to lichess with nothing watching it, and enough of them
# got this machine's address null-routed for a day. That failure is invisible in
# a read-through of the script and invisible in production until the damage is
# already done, so it gets a test.
#
# The fixture is a throwaway bot directory with a real virtualenv, because the
# behaviour under test depends on details a simpler stub gets wrong: lichess-bot
# launches itself by a relative path while its pool workers appear under the
# absolute venv path, and only a real venv reproduces both spellings.
#
# Nothing here signals a process by name alone. Every kill is scoped to this
# fixture's unique temporary path or to a pid this script started, so running
# the test can never disturb a real bot on the same machine.
#
# Usage: tools/lichess/test-run-bot.sh
set -eu
# Each background job needs its own process group, for the same reason the
# supervisor does: without it the supervisor under test shares this script's
# group and stopping it as a group kills the test itself.
set -m

SUPERVISOR=$(cd "$(dirname "$0")" && pwd -P)/run-bot.sh
[ -x "$SUPERVISOR" ] || { echo "cannot find run-bot.sh next to this script" >&2; exit 1; }
command -v python3 >/dev/null 2>&1 || { echo "SKIP: python3 not available" >&2; exit 0; }

PROBE_PORT_TEST="${PROBE_PORT_TEST:-18443}"
WORK=$(mktemp -d)
BOT="$WORK/fakebot"
probe_srv=""
passed=0
failed=0

# Only ever matches this fixture, never a real bot.
fixture_procs() { ps ax -o pid=,command= 2>/dev/null | grep -F "$BOT" | grep -F python | grep -v -F 'run-bot' | awk '{print $1}'; }
fixture_count() { fixture_procs | grep -c . || true; }
kill_fixture()  { for p in $(fixture_procs); do kill -9 "$p" 2>/dev/null || true; done; }

# Compare against this script's real process group, not its pid: the two differ
# whenever the test is itself run as a background job, and the pid comparison
# then fails open and signals the group the test is running in.
MY_PGID=$(ps -o pgid= -p $$ 2>/dev/null | tr -d ' ' || echo "")

# Stop the supervisor the way an operator does, with SIGTERM, and then wait for
# it. Escalating to SIGKILL on a short fuse would defeat the whole test: SIGKILL
# cannot be trapped, so it prevents exactly the cleanup being verified. The
# escalation below is a backstop for a supervisor that has genuinely wedged.
stop_pid() {
    [ -n "${1:-}" ] || return 0
    pgid=$(ps -o pgid= -p "$1" 2>/dev/null | tr -d ' ' || echo "")
    kill -TERM "$1" 2>/dev/null || true
    waited=0
    while [ "$waited" -lt 30 ] && kill -0 "$1" 2>/dev/null; do
        sleep 1
        waited=$((waited + 1))
    done
    if kill -0 "$1" 2>/dev/null && [ -n "$pgid" ] && [ "$pgid" != "$MY_PGID" ]; then
        echo "  note   supervisor did not exit within ${waited}s; forcing"
        kill -KILL -"$pgid" 2>/dev/null || true
    fi
    return 0
}

cleanup() {
    stop_pid "${sup:-}"
    kill_fixture
    [ -n "$probe_srv" ] && kill -9 "$probe_srv" 2>/dev/null || true
    rm -rf "$WORK"
}
trap cleanup EXIT INT TERM

ok()  { passed=$((passed + 1)); echo "  ok     $1"; }
bad() { failed=$((failed + 1)); echo "  FAIL   $1"; }
is()  { if [ "$2" = "$3" ]; then ok "$1 ($2)"; else bad "$1: expected $3, got $2"; fi; }

echo "building fixture in $WORK"
mkdir -p "$BOT"
python3 -m venv "$BOT/venv" >/dev/null 2>&1 || { echo "SKIP: could not create a virtualenv" >&2; exit 0; }
BOT=$(cd "$BOT" && pwd -P)

cat > "$BOT/lichess-bot.py" <<'PY'
import multiprocessing, os, time

def worker():
    while True:
        time.sleep(0.5)

if __name__ == "__main__":
    mode = os.environ.get("FAKE_MODE", "PLAY")
    for _ in range(3):
        multiprocessing.Process(target=worker).start()
    while True:
        if mode == "PLAY":
            print("Game over: fixture", flush=True)
            time.sleep(1)
        elif mode == "FLOOD":
            for _ in range(400):
                print("requests.exceptions.ConnectTimeout: timed out", flush=True)
            time.sleep(0.2)
        else:
            time.sleep(1)
PY
printf 'rate_limiting_delay: 0\n' > "$BOT/config.yml"

# Short timers so the suite finishes in a minute rather than in hours, and a
# local listener so the reachability probe succeeds without touching lichess.
export POLL_SECONDS=2 STALL_SECONDS=6 BACKOFF_MIN=2 BACKOFF_MAX=4 \
       COLD_AFTER_STRIKES=2 PROBE_TIMEOUT=2 TAIL_SAMPLE=50 \
       MAX_ERROR_LINES_PER_POLL=100 \
       PROBE_HOST=127.0.0.1 PROBE_PORT="$PROBE_PORT_TEST"

set +m   # untracked: this one is stopped by pid, never as a group
python3 -m http.server "$PROBE_PORT_TEST" --bind 127.0.0.1 >/dev/null 2>&1 &
probe_srv=$!
set -m
sleep 2

# The fixture spawns three workers plus multiprocessing's resource tracker, and
# all four carry the absolute venv path. The parent uses a relative path and so
# is counted by the supervisor rather than here.
WORKERS=4

echo "1. stopping the supervisor takes the whole process tree with it"
FAKE_MODE=PLAY sh "$SUPERVISOR" "$BOT" >/dev/null 2>&1 & sup=$!
sleep 8
is "workers running under supervision" "$(fixture_count)" "$WORKERS"
stop_pid "$sup"; sup=""
sleep 2
is "workers surviving the supervisor" "$(fixture_count)" "0"
if grep -q "supervisor stopped; 0 lichess-bot" "$BOT/supervisor.log" 2>/dev/null
then ok "supervisor confirmed a zero survivor count on the way out"
else bad "supervisor did not confirm zero survivors"; fi

echo "2. a bot left over from a previous run is cleared, not stacked on"
kill_fixture; rm -f "$BOT/.supervisor.lock" "$BOT/supervisor.log"
set +m   # untracked for the same reason: the supervisor, not this test, clears it
( cd "$BOT" && FAKE_MODE=STALL ./venv/bin/python -u lichess-bot.py >/dev/null 2>&1 & )
set -m
sleep 4
is "stray workers planted" "$(fixture_count)" "$WORKERS"
FAKE_MODE=PLAY sh "$SUPERVISOR" "$BOT" >/dev/null 2>&1 & sup=$!
sleep 8
if grep -q "already running before startup" "$BOT/supervisor.log" 2>/dev/null
then ok "strays detected before starting"
else bad "strays not detected"; fi
stop_pid "$sup"; sup=""
sleep 2
is "workers surviving" "$(fixture_count)" "0"

echo "3. a bot flooding its log is stopped instead of left dialing"
kill_fixture; rm -f "$BOT/.supervisor.lock" "$BOT/supervisor.log"
FAKE_MODE=FLOOD sh "$SUPERVISOR" "$BOT" >/dev/null 2>&1 & sup=$!
sleep 12
if grep -q "flooding:" "$BOT/supervisor.log" 2>/dev/null
then ok "flood tripwire fired"
else bad "flood tripwire did not fire"; fi
stop_pid "$sup"; sup=""
sleep 2
is "workers surviving" "$(fixture_count)" "0"

echo "4. a second supervisor refuses to start alongside the first"
kill_fixture; rm -f "$BOT/.supervisor.lock" "$BOT/supervisor.log"
FAKE_MODE=PLAY sh "$SUPERVISOR" "$BOT" >/dev/null 2>&1 & sup=$!
sleep 6
if (FAKE_MODE=PLAY sh "$SUPERVISOR" "$BOT" 2>&1; true) | grep -q "already running"
then ok "second supervisor rejected"
else bad "second supervisor was allowed to start"; fi
stop_pid "$sup"; sup=""
sleep 2
is "workers surviving" "$(fixture_count)" "0"

echo "5. an unreachable lichess is waited out, not retried into"
kill_fixture; rm -f "$BOT/.supervisor.lock" "$BOT/supervisor.log"
kill -9 "$probe_srv" 2>/dev/null || true; probe_srv=""
sleep 1
FAKE_MODE=PLAY sh "$SUPERVISOR" "$BOT" >/dev/null 2>&1 & sup=$!
sleep 14
is "workers started while unreachable" "$(fixture_count)" "0"
if grep -q "Holding cold" "$BOT/supervisor.log" 2>/dev/null
then ok "held cold after repeated failures"
else bad "did not hold cold"; fi
if grep -q "started lichess-bot pid" "$BOT/supervisor.log" 2>/dev/null
then bad "started the bot even though lichess was unreachable"
else ok "never started the bot"; fi
stop_pid "$sup"; sup=""

echo
echo "$passed passed, $failed failed"
[ "$failed" -eq 0 ]
