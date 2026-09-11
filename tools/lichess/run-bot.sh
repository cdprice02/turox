#!/bin/sh
# Supervise lichess-bot for long unattended runs.
#
# Three failure modes have actually happened here, and a bare
# `nohup python lichess-bot.py &` survives none of them:
#
#   1. The process exits (crash, or a fatal API error).
#   2. The process stays alive but stops making progress. Seen twice: an
#      event-stream thread that died after a network drop, and a rate-limit
#      retry loop that logged an error every second for six hours without ever
#      reconnecting.
#   3. The process is signalled but its children are not, so it keeps talking
#      to lichess long after the supervisor believes it is gone. lichess-bot
#      plays each game in a `multiprocessing` pool, so signalling only the
#      parent pid orphans the workers. Over one week that leaked 150 processes.
#      Each one retried a failed connection every 100ms, and together they dug
#      deep enough that lichess's host stopped answering this address at all:
#      not an HTTP 429 but a null route, which took the whole household off
#      lichess until the processes were found and killed by hand.
#
# Case 2 is why this does NOT watch the log's modification time. That was the
# first thing tried and it fails exactly here: a bot stuck in a retry loop
# writes constantly, so mtime looks healthy while nothing happens. What gets
# watched instead is the count of lines that appear only when the bot is doing
# something real. Errors do not match, so a chattering-but-stuck bot reads as
# stalled.
#
# Case 3 is why the bot is launched into its own process group and is always
# signalled as a group, why the supervisor refuses to start while any earlier
# process survives, and why every exit path runs the same cleanup. A supervisor
# that leaks children is worse than no supervisor, because it turns each
# restart into another permanent talker.
#
# The broader lesson from case 3 is that this script cannot assume the failure
# is on lichess's side and safe to retry into. Being unable to reach lichess is
# sometimes a punishment for how hard we retried, in which case retrying harder
# is the one response guaranteed to prolong it. So reachability is checked once
# per cycle before anything is started, the backoff grows instead of repeating,
# and a bot that floods its log is stopped rather than left to run.
#
# Usage: tools/lichess/run-bot.sh [lichess-bot-dir]
set -eu

# Give every background job its own process group, so the bot and its whole
# worker tree can be signalled with one `kill -- -PGID`. Without this the bot
# joins the supervisor's group and there is no group to signal that does not
# also include the supervisor itself.
set -m

BOT_DIR="${1:-$HOME/repos/lichess-bot}"
# Resolve to a physical path before anything else is derived from it. A process
# command line shows where the interpreter really lives, so if the bot directory
# is reached through a symlink the pool workers will not string-match the path we
# were handed, and exactly the half of the tree that leaks would look like it had
# already exited. A detection check that silently sees half the processes is the
# failure this whole script exists to prevent.
BOT_DIR=$(cd "$BOT_DIR" 2>/dev/null && pwd -P) || {
    echo "bot directory not found: ${1:-$HOME/repos/lichess-bot}" >&2
    exit 1
}
LOG="$BOT_DIR/turox-bot-session.log"
SUP_LOG="$BOT_DIR/supervisor.log"
LOCK="$BOT_DIR/.supervisor.lock"

: "${POLL_SECONDS:=60}"
# Generous next to lichess-bot's own challenge interval, so a genuinely quiet
# stretch (nobody accepting) is never mistaken for a stall.
: "${STALL_SECONDS:=1800}"

# Lines that appear only when the bot is making real progress. Deliberately
# excludes anything an error path can emit.
PROGRESS_RE='Will challenge|Challenge id|Game over|move:|Searching for wtime'

# Two different "cannot talk to lichess" shapes, which need opposite responses
# and used to be conflated. A 429 is lichess answering and asking us to slow
# down, so the bot's own retry logic is the right thing to leave alone. A
# connect timeout is lichess not answering at all, which during the incident
# meant the address was null-routed; reconnecting into that is what kept it
# null-routed. Matching only the 429 shape is why the old guard stopped firing
# once the block escalated, and the supervisor cheerfully restarted into a
# black hole every ten minutes for a day.
RATE_LIMIT_RE='RateLimitedError|Too Many Requests|429'
CONNECT_FAIL_RE='ConnectTimeout|ConnectionError|Max retries exceeded|NewConnectionError|Failed to establish'
: "${TAIL_SAMPLE:=200}"

# Restart pacing. The old fixed ten minutes was far too eager for a failure
# that lasts hours, so this doubles up to a ceiling and only resets once a run
# actually makes progress. Jitter keeps a restarted machine from re-dialing on
# a perfectly regular beat.
: "${BACKOFF_MIN:=600}"
: "${BACKOFF_MAX:=14400}"

# After this many consecutive cycles where lichess is unreachable before we
# even start, stop starting the bot at all and just probe once per cycle. One
# TCP connect every few hours cannot dig the hole deeper, while a running bot
# makes hundreds of attempts per minute.
: "${COLD_AFTER_STRIKES:=3}"
: "${PROBE_TIMEOUT:=10}"
: "${PROBE_HOST:=lichess.org}"
: "${PROBE_PORT:=443}"

# A runaway retry loop is visible as log volume long before it is visible as
# anything else. Normal play writes a handful of lines per move, so a rate this
# high means the bot is spinning on a failure rather than playing, and the
# right move is to stop it instead of letting it keep dialing. During the
# incident the log grew by roughly 2000 lines a minute for hours.
: "${MAX_ERROR_LINES_PER_POLL:=600}"
: "${MAX_LOG_BYTES:=67108864}"

# This script's own process group, which must never be signalled: `$$` is not a
# substitute, because a supervisor started as a background job has a process
# group that differs from its pid, and the comparison would then fail open.
MY_PGID=$(ps -o pgid= -p $$ 2>/dev/null | tr -d ' ' || echo "")

log() { printf '%s supervisor: %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$1" >>"$SUP_LOG"; }

# Every lichess-bot process, whether or not this supervisor started it, and
# whichever supervisor started it. Both spellings are needed: lichess-bot
# launches itself as `./venv/bin/python -u lichess-bot.py`, a relative path that
# does not contain the bot directory at all, while its pool workers appear under
# the absolute venv path with no script name. Matching only one of the two finds
# only half the tree, which is worse than useless for a check whose whole job is
# proving that nothing survived. The count is supposed to be zero whenever no
# bot is running.
bot_pids() {
    ps ax -o pid=,command= 2>/dev/null \
        | grep -F -e "$BOT_DIR" -e 'lichess-bot.py' \
        | grep -F 'python' \
        | grep -v -F 'run-bot.sh' \
        | awk -v self="$$" '$1 != self { print $1 }'
}

count_bot_pids() { bot_pids | grep -c . || true; }

# Signal a whole process group, then escalate. SIGTERM alone is not enough:
# multiprocessing's resource tracker is written to survive it so it can clean
# up shared memory, and a worker wedged in a socket timeout can miss it too.
stop_group() {
    pgid=$1
    kill -TERM -"$pgid" 2>/dev/null || true
    waited=0
    while [ "$waited" -lt 10 ]; do
        if ! kill -0 -"$pgid" 2>/dev/null; then break; fi
        sleep 1
        waited=$((waited + 1))
    done
    kill -KILL -"$pgid" 2>/dev/null || true
}

# Last line of defence. Anything still alive after the group kill gets found by
# name and killed individually, because a process that has been re-parented or
# has called setsid is no longer in the group we just signalled.
reap_strays() {
    remaining=$(count_bot_pids)
    [ "$remaining" -eq 0 ] && return 0
    log "$remaining lichess-bot process(es) survived the group kill; killing by name"
    for p in $(bot_pids); do kill -TERM "$p" 2>/dev/null || true; done
    sleep 3
    for p in $(bot_pids); do kill -KILL "$p" 2>/dev/null || true; done
    sleep 2
    remaining=$(count_bot_pids)
    if [ "$remaining" -gt 0 ]; then
        log "WARNING: $remaining lichess-bot process(es) still alive after SIGKILL;\
 refusing to start another. Investigate with: ps ax | grep lichess-bot"
        return 1
    fi
    return 0
}

# One TCP connect, no HTTP request, so this costs nothing against the API quota
# and tells us the only thing we need to know: whether packets to lichess are
# being answered at all. Run before every start, because starting the bot into
# an unreachable lichess is precisely how a short block becomes a long one.
reachable() {
    if command -v nc >/dev/null 2>&1; then
        nc -z -w "$PROBE_TIMEOUT" "$PROBE_HOST" "$PROBE_PORT" >/dev/null 2>&1
    elif command -v curl >/dev/null 2>&1; then
        curl -sS -o /dev/null --connect-timeout "$PROBE_TIMEOUT" \
            --max-time "$PROBE_TIMEOUT" -I "https://$PROBE_HOST:$PROBE_PORT/" >/dev/null 2>&1
    else
        # No probe available. Assume reachable rather than refusing to ever
        # run; the group kill and the flood tripwire still bound the damage.
        return 0
    fi
}

matches_in_tail() { tail -n "$TAIL_SAMPLE" "$LOG" 2>/dev/null | grep -cE "$1" || true; }
count_in_log()    { [ -f "$LOG" ] && { grep -cE "$1" "$LOG" 2>/dev/null || true; } || echo 0; }
log_bytes()       { wc -c <"$LOG" 2>/dev/null | tr -d ' ' || echo 0; }

cleanup() {
    trap - INT TERM HUP EXIT
    if [ -n "${bot_pgid:-}" ]; then
        log "supervisor exiting; stopping bot process group $bot_pgid"
        stop_group "$bot_pgid"
    fi
    reap_strays || true
    rm -f "$LOCK"
    log "supervisor stopped; $(count_bot_pids) lichess-bot process(es) remain"
}
# Without this, interrupting the supervisor left the bot and its workers
# running with nothing watching them, which is the same leak by another route.
trap 'cleanup; exit 130' INT
trap 'cleanup; exit 143' TERM HUP
trap 'cleanup' EXIT

cd "$BOT_DIR"

# Refuse to run two supervisors at once. Two of them means two bots, double the
# request rate, and each one reaping the other's children out from under it.
if [ -f "$LOCK" ]; then
    other=$(cat "$LOCK" 2>/dev/null || echo "")
    if [ -n "$other" ] && kill -0 "$other" 2>/dev/null; then
        echo "another supervisor is already running (pid $other); refusing to start" >&2
        trap - EXIT
        exit 1
    fi
    rm -f "$LOCK"
fi
printf '%s\n' "$$" >"$LOCK"

log "starting, bot dir $BOT_DIR"

# Refuse to start on top of a dirty slate. If anything from a previous run is
# still alive it is still making requests, and adding a fresh bot on top is how
# the leak compounded unnoticed for a week.
stale=$(count_bot_pids)
if [ "$stale" -gt 0 ]; then
    log "found $stale lichess-bot process(es) already running before startup; clearing them"
    reap_strays || { log "could not clear them; aborting"; exit 1; }
fi

# A zero delay after each move leaves no headroom when lichess is slow, and the
# retries that fills the log with are what the rate limiter counts.
if grep -qE '^[[:space:]]*rate_limiting_delay:[[:space:]]*0[[:space:]]*($|#)' config.yml 2>/dev/null; then
    log "NOTE: config.yml sets rate_limiting_delay: 0. Consider 100 (ms) to leave\
 headroom against Too Many Requests."
fi

backoff=$BACKOFF_MIN
strikes=0

while true; do
    # Sleep first when we are backing off from a previous failure, so that the
    # reachability probe below reflects the state after the wait, not before.
    if [ "${need_backoff:-0}" -eq 1 ]; then
        # Vary the wait by the clock rather than by the pid, which is fixed for
        # the life of the supervisor and would otherwise make every retry land
        # on the same beat.
        jitter=$(( $(date +%s) % (backoff / 5 + 1) ))
        wait_for=$((backoff + jitter))
        log "waiting ${wait_for}s before the next attempt"
        sleep "$wait_for"
    fi
    need_backoff=1

    if ! reachable; then
        strikes=$((strikes + 1))
        if [ "$strikes" -ge "$COLD_AFTER_STRIKES" ]; then
            log "lichess unreachable ($strikes consecutive checks). Holding cold: not\
 starting the bot, probing once per cycle. If this persists for a day, the address is\
 likely blocked and the fix is to stay off lichess, not to retry."
        else
            log "lichess unreachable (check $strikes of $COLD_AFTER_STRIKES); not starting"
        fi
        backoff=$(( backoff * 2 ))
        [ "$backoff" -gt "$BACKOFF_MAX" ] && backoff=$BACKOFF_MAX
        continue
    fi
    if [ "$strikes" -gt 0 ]; then
        log "lichess reachable again after $strikes failed check(s)"
        strikes=0
    fi

    : >"$LOG"   # truncate so the counters below measure this run, not the last
    ./venv/bin/python -u lichess-bot.py >>"$LOG" 2>&1 &
    bot_pid=$!
    # With job control on, the job leads its own group and the group id equals
    # the leader's pid. Read it back anyway rather than assuming, since
    # signalling the wrong group would hit the supervisor.
    bot_pgid=$(ps -o pgid= -p "$bot_pid" 2>/dev/null | tr -d ' ' || echo "")
    if [ -z "$bot_pgid" ]; then
        log "could not read the bot's process group; stopping pid $bot_pid and retrying"
        kill -KILL "$bot_pid" 2>/dev/null || true
        reap_strays || true
        continue
    fi
    if [ "$bot_pgid" = "$MY_PGID" ]; then
        log "ERROR: the bot shares the supervisor's process group, so it cannot be\
 signalled as a group without killing this script. Aborting rather than risking\
 another orphaned worker leak."
        kill -KILL "$bot_pid" 2>/dev/null || true
        exit 1
    fi
    log "started lichess-bot pid $bot_pid (process group $bot_pgid)"

    last_progress=$(count_in_log "$PROGRESS_RE")
    last_errors=$(count_in_log "$CONNECT_FAIL_RE|$RATE_LIMIT_RE")
    made_progress=0
    quiet_for=0
    stop_reason=""

    while kill -0 "$bot_pid" 2>/dev/null; do
        sleep "$POLL_SECONDS"

        now_progress=$(count_in_log "$PROGRESS_RE")
        now_errors=$(count_in_log "$CONNECT_FAIL_RE|$RATE_LIMIT_RE")
        # Truncation resets both counters, so treat a decrease as a new
        # baseline rather than as impossible negative progress.
        [ "$now_progress" -lt "$last_progress" ] && last_progress=0
        [ "$now_errors" -lt "$last_errors" ] && last_errors=0

        # Stop a bot that is flooding, whatever the cause. This is the tripwire
        # that bounds how much traffic a single bad run can generate, and it
        # fires in one poll instead of waiting out the stall timeout.
        if [ "$((now_errors - last_errors))" -gt "$MAX_ERROR_LINES_PER_POLL" ]; then
            stop_reason="flooding: $((now_errors - last_errors)) error lines in\
 ${POLL_SECONDS}s, over the ${MAX_ERROR_LINES_PER_POLL} ceiling"
            break
        fi
        last_errors=$now_errors

        if [ "$(log_bytes)" -gt "$MAX_LOG_BYTES" ]; then
            log "log passed $MAX_LOG_BYTES bytes; truncating"
            : >"$LOG"
            last_progress=0
            last_errors=0
            # Re-read both counters next tick rather than comparing this tick's
            # pre-truncation numbers against the reset baseline, which would
            # otherwise register the truncation itself as progress.
            continue
        fi

        if [ "$now_progress" -gt "$last_progress" ]; then
            last_progress=$now_progress
            made_progress=1
            quiet_for=0
            continue
        fi

        quiet_for=$((quiet_for + POLL_SECONDS))
        [ "$quiet_for" -lt "$STALL_SECONDS" ] && continue

        # Distinguish the two stalls. Restarting cures a dead event-stream
        # thread; it actively prolongs a rate limit, because reconnecting is
        # what the limit counts.
        if [ "$(matches_in_tail "$RATE_LIMIT_RE")" -gt 0 ]; then
            log "no progress for ${quiet_for}s but the tail is rate-limit errors;\
 waiting rather than reconnecting, since reconnecting is what the limit counts"
            quiet_for=0
            continue
        fi
        if [ "$(matches_in_tail "$CONNECT_FAIL_RE")" -gt 0 ]; then
            stop_reason="no progress for ${quiet_for}s and lichess is not answering;\
 stopping so the address gets a rest instead of a faster retry"
            break
        fi
        stop_reason="no progress for ${quiet_for}s (log has $(wc -l <"$LOG") lines, so\
 it is chattering rather than idle)"
        break
    done

    if [ -n "$stop_reason" ]; then
        log "$stop_reason; stopping process group $bot_pgid"
    else
        log "lichess-bot exited on its own"
    fi
    stop_group "$bot_pgid"
    wait "$bot_pid" 2>/dev/null || true
    reap_strays || { log "aborting: cannot guarantee the previous run is stopped"; exit 1; }
    bot_pgid=""

    # Only a run that actually played earns a reset. Without this a bot that
    # fails instantly forever keeps retrying at the floor, which is the pattern
    # that generated the flood.
    if [ "$made_progress" -eq 1 ]; then
        backoff=$BACKOFF_MIN
    else
        backoff=$((backoff * 2))
        [ "$backoff" -gt "$BACKOFF_MAX" ] && backoff=$BACKOFF_MAX
    fi
done
