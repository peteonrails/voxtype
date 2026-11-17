#!/usr/bin/env bash
# Keep apt working on GitHub runners when ubuntu.com DNS is down.
#
# On 2026-11-11 Canonical's DNS went down for hours. Public resolvers kept
# serving cached answers, but the Azure resolver behind GitHub-hosted runners
# did not, so every apt fetch from archive/security/azure.archive.ubuntu.com
# failed on the runner and inside Docker builds alike. The arm64 runners
# use ports.ubuntu.com, which needs the same treatment.
#
# When the runner cannot resolve the Ubuntu mirrors, this resolves them over
# DNS-over-HTTPS (falling back to known addresses), pins them in the runner's
# /etc/hosts, and exports DOCKER_ADD_HOSTS for `docker build`. When DNS works
# it does nothing, so it is safe to leave in place.
set -euo pipefail

if [ "$(uname -s)" != Linux ]; then
  exit 0
fi

HOSTS=(archive.ubuntu.com security.ubuntu.com azure.archive.ubuntu.com ports.ubuntu.com)

# Last-known addresses, used only if DNS-over-HTTPS is unreachable too.
declare -A KNOWN=(
  [archive.ubuntu.com]=91.189.90.13
  [security.ubuntu.com]=91.189.90.12
  [azure.archive.ubuntu.com]=52.147.219.192
  [ports.ubuntu.com]=91.189.91.104
)

export_env() {
  if [ -n "${GITHUB_ENV:-}" ]; then
    echo "DOCKER_ADD_HOSTS=$1" >> "$GITHUB_ENV"
  else
    echo "DOCKER_ADD_HOSTS=$1"
  fi
}

if timeout 15 getent ahostsv4 archive.ubuntu.com >/dev/null 2>&1; then
  echo "Ubuntu mirrors resolve normally; nothing to pin."
  export_env ""
  exit 0
fi

doh() {
  local name=$1 url ip
  for url in "https://1.1.1.1/dns-query" "https://8.8.8.8/resolve"; do
    ip=$(curl -fsS --max-time 10 -H 'accept: application/dns-json' \
      "${url}?name=${name}&type=A" 2>/dev/null \
      | python3 -c 'import json,sys; a=[x["data"] for x in json.load(sys.stdin).get("Answer",[]) if x.get("type")==1]; print(a[0] if a else "")' \
      2>/dev/null || true)
    if [ -n "$ip" ]; then
      echo "$ip"
      return 0
    fi
  done
  return 1
}

echo "Ubuntu mirrors do not resolve; pinning addresses."
args=()
for host in "${HOSTS[@]}"; do
  ip=$(doh "$host" || true)
  ip=${ip:-${KNOWN[$host]}}
  echo "  $host -> $ip"
  echo "$ip $host" | sudo tee -a /etc/hosts >/dev/null
  args+=("--add-host=$host:$ip")
done
export_env "${args[*]}"
