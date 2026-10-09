#!/bin/sh
set -eu
# Restore the already-configured tunnel after every container start.
# Credentials stay in private stdin/files, never in process arguments.
(
    umask 077
    config="${ABV_DATA_DIR:-/root/.antigravity_tools}/gui_config.json"
    attempt=0
    while [ "$attempt" -lt 60 ]; do
        attempt=$((attempt + 1))
        if curl -fsS --max-time 2 http://127.0.0.1:8045/health >/dev/null 2>&1 && [ -r "$config" ]; then
            if ! jq -e '.cloudflared.enabled == true' "$config" >/dev/null 2>&1; then exit 0; fi
            payload=$(mktemp /tmp/antigravity-tunnel.XXXXXX)
            trap 'rm -f "$payload"' EXIT HUP INT TERM
            if jq -c '{config: .cloudflared}' "$config" > "$payload"; then
                if jq -r '"header = " + ("Authorization: Bearer " + (if (.proxy.admin_password // "") != "" then .proxy.admin_password else .proxy.api_key end) | @json)' "$config" |
                    curl --config - --fail --silent --max-time 45 --output /dev/null \
                    -H 'Content-Type: application/json' --data-binary "@$payload" \
                    http://127.0.0.1:8045/api/proxy/cloudflared/start; then
                    echo 'Configured Cloudflare tunnel restored.' >&2
                    exit 0
                fi
            fi
            rm -f "$payload"
        fi
        sleep 2
    done
    echo 'Cloudflare tunnel did not start; check administrator tunnel status.' >&2
) &
exec /app/antigravity-tools --headless "$@"
