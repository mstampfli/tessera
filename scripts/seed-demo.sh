#!/usr/bin/env bash
# Seed a small synthetic security dataset so a fresh install has something to
# show: three campaigns that cluster separately, a shared ASN that bridges two of
# them, and indicators (IPs, domains, hashes, CVEs, emails) that correlate.
#
# All indicators use documentation ranges (RFC 5737) and made-up .example
# domains, so nothing here is a real host.
#
# Usage:
#   TESSERA_TOKEN=<ingest-scoped token> [TESSERA_URL=http://127.0.0.1:8400] \
#     scripts/seed-demo.sh
set -euo pipefail

URL="${TESSERA_URL:-http://127.0.0.1:8400}"
TOKEN="${TESSERA_TOKEN:?set TESSERA_TOKEN to an ingest-scoped API token}"

ingest() { # content title source
  curl -sf -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
    -d "$(python3 - "$1" "$2" "$3" <<'PY'
import json, sys
print(json.dumps({"content": sys.argv[1], "media_type": "text/plain",
                  "title": sys.argv[2], "source_name": sys.argv[3]}))
PY
)" "$URL/v1/ingest" >/dev/null
  echo "  + $2"
}

echo "seeding demo dataset into $URL ..."

# Campaign A: Qakbot banking trojan.
ingest "Qakbot loader beacons to command-and-control server 203.0.113.44 over port 443 using TLS." "qakbot-c2" "qakbot-campaign"
ingest "Qakbot C2 infrastructure: the domain qakbot-panel.example resolves to 203.0.113.44." "qakbot-domain" "qakbot-campaign"
ingest "Qakbot dropped a loader with SHA256 hash 9f2a1c3e4b5d6a7f8091a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f7 on the victim endpoint." "qakbot-hash" "qakbot-campaign"
ingest "The Qakbot campaign exploited CVE-2026-31337 for initial access, delivered through phishing emails." "qakbot-cve" "qakbot-campaign"
ingest "Qakbot post-exploitation used Cobalt Strike beacons communicating with 203.0.113.44 for lateral movement across the network." "qakbot-lateral" "qakbot-campaign"

# Campaign B: credential-harvesting phishing kit.
ingest "A phishing campaign hosts a fake Microsoft login page on login-msftsecure.example behind the server 198.51.100.50." "phish-portal" "phishing-campaign"
ingest "The phishing kit at 198.51.100.50 exfiltrates harvested credentials to the collector address mail-drop@evil-collector.example." "phish-exfil" "phishing-campaign"
ingest "Victims received lure emails from noreply@login-msftsecure.example themed as a shared OneDrive document." "phish-lure" "phishing-campaign"
ingest "The same credential harvester on 198.51.100.50 also serves a second kit on the domain paypal-verify-account.example." "phish-second-kit" "phishing-campaign"

# Campaign C: ransomware intrusion.
ingest "A ransomware affiliate gained access to the network through an exposed RDP service on 192.0.2.23." "ransom-access" "ransomware-campaign"
ingest "The ransomware operators exploited CVE-2026-44112 in an unpatched VPN appliance for initial access, then used a leaked credential." "ransom-cve" "ransomware-campaign"
ingest "Data was exfiltrated to 192.0.2.23 before the operators encrypted the environment with the locker payload." "ransom-exfil" "ransomware-campaign"
ingest "The ransom note demanded payment and threatened to leak stolen data on a Tor-based leak site if unpaid." "ransom-note" "ransomware-campaign"

# Bridge: a shared hosting provider links the Qakbot and phishing infrastructure.
ingest "Threat-infrastructure analysis shows that autonomous system AS64500 hosts both 203.0.113.44 and 198.51.100.50, linking the two campaigns to a shared bulletproof hoster." "bridge-asn" "infrastructure"

echo "done. The pipeline is chunking, embedding, extracting entities, correlating,"
echo "and clustering. Insights appear once each cluster forms (within a minute)."
