#!/bin/sh
# Supervise lichess-bot for long unattended runs.
#
# Two failure modes have actually happened here, and a bare `nohup python
# lichess-bot.py &` survives neither:
#
#   1. The process exits (crash, or a fatal API error).
#   2. The process stays alive but stops making progress. Seen twice: an
#      event-stream thread that died after a network drop, and a rate-limit
#      retry loop that logged an error every second for six hours without ever
#      reconnecting.
#
# The second case is why this does NOT watch the log's modification time. That
# was the first thing tried and it fails exactly here: a bot stuck in a retry
# loop writes constantly, so mtime looks healthy while nothing happens. What
# gets watched instead is the count of lines that only appear when the bot is
# actually doing something: issuing a challenge, playing a move, finishing a
# game. Errors do not match, so a chattering-but-stuck bot reads as stalled.
#
# Usage: tools/lichess/run-bot.sh [lichess-bot-dir]
set -eu

BOT_DIR="${1:-$HOME/repos/lichess-bot}"
LOG="$BOT_DIR/turox-bot-session.log"
SUP_LOG="$BOT_DIR/supervisor.log"
# Generous next to lichess-bot's own challenge interval, so a genuinely quiet
# stretch (nobody accepting) is never mistaken for a stall.
STALL_SECONDS=1800
POLL_SECONDS=60
# Lines that appear only when the bot is making real progress. Deliberately
# excludes anything an error path can emit.
PROGRESS_RE='Will challenge|Challenge id|Game over|move:|Searching for wtime'
# Long enough for a lichess rate limit to lapse before reconnecting into it.
RESTART_BACKOFF=600
# Lichess rate-limits the event stream for *reconnecting* too often, so a
# restart is the one thing that makes that particular stall worse. When the
# recent log is dominated by rate-limit errors, the bot's own retry loop is
# already doing the right thing and the supervisor must stay out of its way.
RATE_LIMIT_RE='RateLimitedError'
RATE_LIMIT_SAMPLE=200

log() { printf '%s supervisor: %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$1" >>"$SUP_LOG"; }

# How many progress lines the log holds. A count that stops rising is the
# stall signal; the log growing is not, since errors grow it too.
progress_count() {
    [ -f "$LOG" ] || { echo 0; return; }
    grep -cE "$PROGRESS_RE" "$LOG" 2>/dev/null || echo 0
}

cd "$BOT_DIR"
log "starting, bot dir $BOT_DIR"

while true; do
    : >"$LOG"   # truncate so log_age measures this run, not the previous one
    ./venv/bin/python -u lichess-bot.py >>"$LOG" 2>&1 &
    pid=$!
    log "started lichess-bot pid $pid"

    last_progress=$(progress_count)
    quiet_for=0

    while kill -0 "$pid" 2>/dev/null; do
        sleep "$POLL_SECONDS"
        now_progress=$(progress_count)
        if [ "$now_progress" -gt "$last_progress" ]; then
            last_progress=$now_progress
            quiet_for=0
        else
            quiet_for=$((quiet_for + POLL_SECONDS))
        fi

        # Distinguish the two stalls. Restarting cures a dead event-stream
        # thread; it actively prolongs a rate limit, because reconnecting is
        # what the limit is counting.
        if [ "$quiet_for" -ge "$STALL_SECONDS" ] &&
           [ "$(tail -n "$RATE_LIMIT_SAMPLE" "$LOG" | grep -c "$RATE_LIMIT_RE")" -gt 0 ]; then
            log "no progress for ${quiet_for}s but the tail is rate-limit errors; waiting\
 rather than reconnecting, since reconnecting is what the limit counts"
            quiet_for=0
            continue
        fi

        if [ "$quiet_for" -ge "$STALL_SECONDS" ]; then
            log "no progress for ${quiet_for}s (log has $(wc -l <"$LOG") lines, so it is\
 chattering rather than idle); restarting pid $pid"
            kill "$pid" 2>/dev/null || true
            sleep 5
            kill -9 "$pid" 2>/dev/null || true
            break
        fi
    done

    wait "$pid" 2>/dev/null || true
    log "lichess-bot exited; restarting in ${RESTART_BACKOFF}s"
    sleep "$RESTART_BACKOFF"
done
