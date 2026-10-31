#!/usr/bin/env bash
# Part of the APK.audio project — http://APK.audio — made by Anthony Kuzub
# MIT Licence. Free, for everyone, for ever. Full text in LICENSE at the root.
#
# The live smoke check: post one probe job to the RUNNING Worker-SAMPLE_ANALYZER
# and wait for its answer on incoming/result, matched by job_id.
#   SRC/probe_job.sh                       broker on localhost:1883
#   MQTT_HOST=… MQTT_PORT=… SRC/probe_job.sh
# Credentials, when the broker wants them: APK_MQTT_USERNAME / APK_MQTT_PASSWORD.
# Exit 0 = the worker answered ok:true with a .PEAK record; 1 = it answered with
# an error; 2 = no answer within $TIMEOUT seconds (default 30).
set -euo pipefail
HOST="${MQTT_HOST:-localhost}"; PORT="${MQTT_PORT:-1883}"; TIMEOUT="${TIMEOUT:-30}"
BASE="APK.audio/System/Protocols/sample_analyzer"
JOB_ID="probe-$$-$(date +%s)"
AUTH=()
if [[ -n "${APK_MQTT_USERNAME:-}" ]]; then AUTH=(-u "$APK_MQTT_USERNAME" -P "${APK_MQTT_PASSWORD:-}"); fi

# A 0.5 s 440 Hz mono 16-bit WAV, built here so the probe needs no fixture file.
WAV_B64="$(python3 -I -c '
import base64, io, math, struct, wave
sr = 44100; buf = io.BytesIO()
with wave.open(buf, "wb") as w:
    w.setnchannels(1); w.setsampwidth(2); w.setframerate(sr)
    w.writeframes(b"".join(struct.pack("<h", int(12000 * math.sin(2 * math.pi * 440 * n / sr))) for n in range(sr // 2)))
print(base64.b64encode(buf.getvalue()).decode())')"

ANSWERS="$(mktemp)"; trap 'rm -f "$ANSWERS"' EXIT
mosquitto_sub -h "$HOST" -p "$PORT" "${AUTH[@]}" -t "$BASE/incoming/result" -W "$TIMEOUT" > "$ANSWERS" 2>/dev/null &
SUB=$!
sleep 0.5
printf '{"job_id":"%s","filename":"probe.wav","folder":"smoke","content_b64":"%s"}' "$JOB_ID" "$WAV_B64" \
  | mosquitto_pub -h "$HOST" -p "$PORT" "${AUTH[@]}" -t "$BASE/outgoing/job" -s
for _ in $(seq 1 "$((TIMEOUT * 4))"); do
  if grep -q "\"$JOB_ID\"" "$ANSWERS"; then
    kill "$SUB" 2>/dev/null || true
    LINE="$(grep "\"$JOB_ID\"" "$ANSWERS" | head -1)"
    if python3 -I -c 'import json,sys; a=json.loads(sys.argv[1]); sys.exit(0 if a.get("ok") and a["result"]["metadata"].get("name") else 1)' "$LINE"; then
      echo "probe_job.sh: PASS — $JOB_ID answered ok with a .PEAK record"; exit 0
    fi
    echo "probe_job.sh: FAIL — $JOB_ID answered: ${LINE:0:300}" >&2; exit 1
  fi
  sleep 0.25
done
kill "$SUB" 2>/dev/null || true
echo "probe_job.sh: FAIL — no answer to $JOB_ID on $BASE/incoming/result within ${TIMEOUT}s" >&2; exit 2
