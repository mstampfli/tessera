# OSINT tool reference

A categorized reference of open-source-intelligence tools, one section per
category, with an honest note wherever a category has no strong or maintained
(especially free) tool. It exists because tessera's flagship showcase is the
security and OSINT domain: these categories map onto the entity kinds the
security extractor pack recognizes (IPs, domains, hashes, certificates,
identities, breach and threat-intel indicators), so this is the menu of sources
that feed, corroborate, or enrich what tessera ingests and correlates.

Each tool is tagged with its access model (free, paid, or freemium), whether it
exposes a usable API, and its maintenance status. Status was verified against
current sources in mid-2026; the "dead or stale" flags matter, because several
tools that still top search-engine "best OSINT tools" lists are abandoned.

This is a reference, not an endorsement. Use every one of these lawfully and
within the terms of service of the target and the tool.

## How to read the gaps

The bottom line up front: the categories where no strong free and maintained
tool exists are ethical face search, phone owner and spam reputation,
people-search automation, social-media scraping at scale, automated deepfake
detection, continuous dark-web monitoring, court-grade blockchain attribution,
and vehicle or license-plate tracking. The full "Where the real gaps are"
summary at the end collects these. Everywhere else the free and open stack is
genuinely strong.

---

## A. Network and infrastructure

### DNS, WHOIS, and domain records

- **SecurityTrails** - freemium, API, maintained. Historical DNS and WHOIS,
  passive DNS, the standard pivot tool. Free tier is roughly 50 queries a month.
- **DNSDumpster** - free, maintained. Fast DNS recon and subdomain map in the
  browser.
- **WhoisXML API** - freemium, API, maintained. Bulk WHOIS, DNS, and IP data.
  DomainTools is the paid heavyweight for the same job.
- GAP: none.

### IP and host scanning and exposure

- **Shodan** - freemium, API, maintained. The internet-device search engine; the
  free InternetDB API gives bulk IP lookups.
- **Censys** - freemium, API, maintained. Cleaner certificate and host data,
  strong for attack-surface mapping.
- **Netlas** - freemium, API, maintained. The best free tier of the scanners
  right now.
- **GreyNoise** - freemium, API. Tells you whether an IP is mass-scanner noise or
  targeted; free tier is 50 lookups a week.
- Dead: **BinaryEdge** shut down in March 2025.
- GAP: none, but free tiers are shrinking across the board.

### Subdomain enumeration

- **subfinder** (ProjectDiscovery) - free and open source, maintained. Fast
  passive enumeration, the current default.
- **Amass** (OWASP) - free and open source, maintained. Most thorough (active and
  passive), heavier to run.
- **theHarvester** - free and open source, maintained. Emails, subdomains, and
  hosts from 40-plus sources.
- Stale: **Sublist3r** is largely unmaintained; use subfinder instead.
- GAP: none.

### Certificate transparency

- **crt.sh** - free, maintained. The go-to CT-log search; queryable over SQL.
- **CertSpotter** (SSLMate) - free plus API, maintained. CT monitoring with
  alerts.
- **MerkleMap** - free, maintained. Newer, fast CT search interface.
- GAP: none.

### Web technology fingerprinting

- **WhatWeb** - free and open source, maintained. CLI fingerprinter.
- **httpx** (ProjectDiscovery) - free and open source, maintained. Fast HTTP
  probing and technology detection at scale.
- **wappalyzergo** and the community Wappalyzer dataset - free and open source.
  The fork path after Wappalyzer's open-source project was closed in 2023.
- **BuiltWith** - paid, API. Deepest technology profile and history.
- GAP: none, but the once-canonical open Wappalyzer went proprietary in 2023.

---

## B. Identity and people

### Email discovery, validation, and enrichment

- **theHarvester** - free and open source, maintained. Best free option for
  "emails for a domain."
- **Hunter.io** - freemium, best-in-class API, maintained. Domain to email with a
  confidence score; free tier is 25 lookups a month.
- **Epieos** - free web tool, maintained. Email to linked accounts across 140-plus
  services; the maintained successor to holehe.
- **EmailRep.io** - free API (no key required), maintained. Reputation and risk
  enrichment.
- Dead: **holehe** is abandoned and broken by platform hardening; use Epieos.
- GAP: none.

### Username and account enumeration

The healthiest category in this cluster.

- **Sherlock** - free and open source, very active (around 400 sites). Best quick
  sweep.
- **Maigret** - free and open source, maintained (3000-plus sites, extracts
  profile metadata, not just hit or miss).
- **WhatsMyName** - free, community-maintained (700-plus hand-vetted sites, lowest
  false-positive rate; the dataset many other tools consume).
- **Blackbird** - free and open source, maintained; searches by username and by
  email.
- GAP: none. The caveat is that you run these yourself; there is no hosted API.

### People search and background lookups (mostly US)

- **TruePeopleSearch** - free, active, no account or limits. Name to addresses,
  phones, relatives, and associates.
- **FastPeopleSearch** - free, active. Solid for straightforward lookups, but
  freshness is mixed.
- **ThatsThem** and **IDCrawl** - free, active; ThatsThem exposes a limited API.
- Dead as a free tool: **Pipl** pivoted fully to an enterprise identity API.
- GAP: partial. Free web lookups are fine, but there is no good free API or
  automation: everything programmatic or bulk is paywalled, and the one strong
  identity API (Pipl) went enterprise-only.

### Phone number intelligence

- **numverify** - freemium API, maintained. Carrier, line type, and region.
- **Veriphone.io** - freemium API, maintained. A good second source.
- **PhoneInfoga** - free and open source but officially unmaintained; its scanners
  have decayed. Legacy use only.
- **Truecaller** - no legitimate free OSINT API; owner name and spam reputation
  live in the app, and the unofficial libraries ride your account token and get
  banned.
- GAP: yes. Metadata (carrier, line type, region) is free and easy, but owner
  identity and spam reputation have no strong maintained free tool. Phone OSINT
  has degraded to "free equals metadata only."

### Social media analysis and scraping

The most degraded category in this cluster.

- **Instaloader** - free and open source, maintained. Best free Instagram option.
- **Telepathy** - free and open source, maintained. Telegram, currently the most
  scrape-friendly platform.
- **Scweet** - free and open source; still works against X's current GraphQL but
  is brittle and needs proxies and multiple accounts.
- Paid at scale: **Apify**, **Bright Data**, and **Maltego with Social Links**
  transforms.
- Graveyard: **snscrape**, **Twint**, and **Nitter** are dead; **Proxycurl**
  (LinkedIn) was sued and shut down in 2025.
- GAP: yes. No reliable free all-platform scraper exists anymore. Free tools
  survive only per platform and fragile; LinkedIn, Facebook, and TikTok at any
  scale are effectively paid-only.

---

## C. Media, geolocation, and metadata

### Reverse image search and facial recognition

- **Google Lens** - free (the Cloud Vision Web Detection API is the paid
  programmatic equivalent). Best all-round; deliberately weak on faces by policy.
- **Yandex Images** - free, maintained but geopolitically degraded since 2022.
  Still the best free proxy for face-adjacent matching.
- **TinEye** - free web tool (exact-match focus) plus a robust paid API. Best for
  provenance and earliest appearance.
- Face-specific: **PimEyes** (paid, legally embattled) and **FaceCheck.ID**
  (freemium, with crypto-only paid unblurring).
- GAP (general reverse image): none. GAP (dedicated face search): yes. There is
  no reliable ethical free tool; the purpose-built engines are paywalled,
  privacy-invasive, and increasingly illegal in the EU, where the AI Act's
  high-risk rules become enforceable in August 2026.

### Geolocation and GEOINT

- **Google Earth Pro** (desktop) - free, maintained. Historical-imagery time
  slider plus a built-in sun and shadow tool; the core of most geolocation work.
- **Copernicus Browser** - free. It replaced the Sentinel Hub EO Browser (shut
  down around February 2025). Free Sentinel-2 imagery is about 10 m resolution;
  sub-meter fresh imagery is paid.
- **SunCalc**, **Stellarium**, and **ShadowCalculator** - free. Chronolocation
  from shadows and star or moon positions.
- **Overpass Turbo** - free, API. Query OpenStreetMap by feature to narrow
  candidate locations.
- Street level: **Google Street View** and **Mapillary** (free, with an API),
  plus Yandex Panoramas and KartaView. Bellingcat's toolkit aggregates all of it.
- GAP: none. The only genuine paywall is fresh sub-meter commercial satellite
  imagery.

### Metadata and document forensics

- **ExifTool** - free and open source, actively maintained. The reference
  standard across 300-plus formats: GPS, timestamps, camera make and serial, lens,
  and software.
- Browser EXIF viewers (**Jimpl**, **metadata2go**, **exif.tools**) - free, for
  quick triage.
- **FOCA** and **metagoofil** - free and open source, aging. Domain-level document
  metadata harvesting (authors, software versions, internal paths).
- GAP: none; ExifTool anchors it. The caveat is that the domain harvesters are
  aging.

### Video and audio verification and deepfake detection

- **InVID and WeVerify** - free browser extension, actively maintained. Gold
  standard for manual video verification: keyframe extraction, per-frame reverse
  search, and metadata.
- **Reality Defender**, **Sensity**, and **Hive** - paid, with APIs (Reality
  Defender has a small free developer tier). The credible detection engines.
- **Deepware Scanner** - free web screening aid, explicitly not forensic-grade.
- GAP (verification): none. GAP (automated deepfake detection): yes. There is no
  reliable free tool; free options are noisy with high false-positive rates, the
  paid engines are imperfect, and a 2026 comparative study found human reviewers
  outperformed every automated tool. Treat any automated verdict as one weak
  signal, never as proof.

---

## D. Threat, data, and specialized

### Breach and leaked-credential lookup

- **Have I Been Pwned** - freemium, API (v3), maintained. The free password check
  (k-anonymity) is best-in-class; email and domain search now needs a paid key.
- **DeHashed** - paid (roughly 0.02 USD per query), API, maintained. Exposes the
  record fields HIBP deliberately withholds.
- **Intelligence X** - freemium, API. Breaches, paste sites, and dark web with
  historical snapshots.
- **LeakCheck** - freemium, API. Cheap programmatic lookups. Hudson Rock's
  Cavalier is a useful free adjacent tool for infostealer-log exposure.
- GAP: none for a yes-or-no breach signal, but the actual leaked-password content
  is paywalled across DeHashed, IntelX, and LeakCheck.

### Threat intelligence and IOC enrichment

- **VirusTotal** - freemium, API (around 500 requests a day on the free tier). The
  default file-hash, URL, domain, and IP reputation lookup.
- **abuse.ch** (URLhaus, MalwareBazaar, ThreatFox, Feodo Tracker) - free, APIs,
  maintained (Spamhaus is the primary licensee). The best free open feeds of
  malicious URLs, malware samples, IOCs, and C2 or botnet trackers.
- **AlienVault OTX** (now **LevelBlue OTX**) - free, API. A large community IOC
  exchange with subscribable pulses.
- **GreyNoise** - freemium, API. Separates internet-wide scanner noise from
  targeted activity and kills false positives.
- GAP: none; the free stack is genuinely good here.

### Dark web, onion, and paste-site monitoring

- **Ahmia** - free, maintained. The cleanest maintained onion search engine; it
  only indexes reachable sites.
- **Intelligence X** - freemium, API. Best for retrieving cached or deleted pastes.
- **Hudson Rock Cavalier** - free. Infostealer and stealer-log exposure lookups.
- Dead: **OnionScan** has been unmaintained since roughly 2016 to 2019, and the
  **Pastebin** public scraping API was removed.
- GAP: yes. Genuine continuous dark-web monitoring with alerting is
  enterprise-paywalled (Flare, DarkOwl, SpyCloud, and similar, from a few hundred
  dollars a month up to six figures a year). Free tools do ad-hoc search, not
  monitoring, because indexing closed and invite-only forums needs human-operated
  access that free crawlers cannot reach.

### Code and secret leakage

- **TruffleHog** - free and open source (plus an enterprise tier), maintained.
  Recognizes 800-plus secret types and verifies which leaked credentials are still
  live; best for full git-history scans.
- **Gitleaks** - free and open source (MIT), maintained. The fastest scanner and
  the standard for pre-commit and CI diff blocking.
- **GitHub Secret Scanning with Push Protection** - free on all public repos; paid
  (Advanced Security) for private repos.
- **GitGuardian (ggshield)** - freemium, API. Adds governance and dashboards for
  organizations.
- Dead: **gitrob** and **shhgit** are abandoned and superseded by the two above.
- GAP: none; a strong, fully usable free stack.

### Cryptocurrency and blockchain tracing and attribution

- **Arkham** - freemium, API. The best free-ish named-entity attribution
  interface. Arkham winding down its separate exchange product does not affect the
  intel and explorer product.
- **MetaSleuth** (BlockSec) - freemium, API. The most polished free visual
  fund-flow tracer across multiple chains.
- **Etherscan**, **mempool.space**, and **Chainabuse** - free (Etherscan has an
  API). The raw-data backbone plus crowdsourced scam and ransomware reports.
- **GraphSense** (Iknaio) - free, open source, self-hostable. The only serious
  open-source clustering analytics platform.
- GAP: yes. Court-grade attribution (linking addresses to real-world identities,
  exchange KYC, admissible reports) is enterprise-paywalled (Chainalysis,
  Elliptic, TRM Labs, all five or six figures). Free tools give raw transaction
  flow and limited labels, not defensible attribution.

### Corporate and business records

- **OpenCorporates** - freemium, API (500 calls a month free). The largest open
  company database, 200 million-plus entities across 140-plus jurisdictions.
- **OpenSanctions** - free for non-commercial use, API. The best free sanctions,
  PEP, and beneficial-ownership screening.
- **OCCRP Aleph** - free and open source, API, self-hostable. Cross-references
  company records with leaks and public records; the investigative-journalism
  standard.
- **GLEIF LEI** - free and open, API. Authoritative legal-entity identifiers with
  parent and ownership links.
- GAP: none. Deep credit and risk enrichment (Dun and Bradstreet and similar) runs
  25,000 USD a year and up, but the core registry, ownership, and sanctions data
  is free.

### Transportation tracking

- **ADS-B Exchange** - freemium, API (paid). Shows unfiltered military and
  government aircraft, which is its differentiator; most competitors filter
  sensitive tail numbers.
- **OpenSky Network** - free for research and non-commercial use, REST API. The
  clean, academically friendly ADS-B source.
- **Flightradar24** - freemium. Best UI and coverage for casual tracking, but a
  restrictive and expensive API.
- **AISstream.io** (ships) - free real-time global AIS over WebSocket, API. The
  best genuinely free live AIS source; AISHub is a contributor-based free
  alternative. MarineTraffic (now Kpler-owned) and VesselFinder are freemium with
  paid credit-based APIs.
- GAP: none for aircraft (OpenSky's free API plus ADS-B Exchange's unfiltered
  feed) or ships (AISstream's free live feed). Vehicle or license-plate tracking
  has no strong legitimate free OSINT tool.

### All-in-one frameworks and aggregators

- **SpiderFoot** - free and open source. 200-plus modules, still the most complete
  free automation aggregator; watch the open-source project for drift after the
  Intel 471 acquisition.
- **IntelOwl** (Honeynet Project) - free and open source, API, actively
  maintained. The best-maintained modern analyzer and aggregator right now.
- **Maltego Community Edition** - free tier of a commercial tool (capped at 12
  results per transform), maintained. The best link-analysis and graphing tool;
  full power is paid.
- **Recon-ng** - free and open source, functional and shipped in Kali, but
  low-velocity (last major release in 2019). **theHarvester** is the maintained
  companion for email, subdomain, and host enumeration.
- GAP: none. SpiderFoot for breadth, IntelOwl for a maintained modern aggregator,
  and Maltego CE for graphing cover it free.

### Wireless, wardriving, and IoT geolocation

- **WiGLE** - free, API, maintained. The definitive crowdsourced database of
  Wi-Fi, Bluetooth, and cell-tower locations; look up a BSSID's physical location.
- **Kismet** - free and open source, maintained, REST API. The reference wireless
  sniffer and detector; logs directly to WiGLE's format.
- **Bettercap** - free and open source, maintained. Active Wi-Fi, BLE, and HID
  recon and attack framework.
- GAP: none. WiGLE plus Kismet is a strong, fully free, actively maintained combo,
  and WiGLE has no serious free competitor for the crowdsourced-location database.

---

## Where the real gaps are

The categories where no strong free and maintained tool exists, worth knowing
before you build a workflow around them:

1. **Ethical face search** - free options are either not purpose-built (Yandex) or
   legally embattled and paywalled (PimEyes, FaceCheck.ID).
2. **Phone owner and spam reputation** - free gets you carrier and line type only;
   owner identity is locked inside Truecaller's app.
3. **People-search automation and API** - free web lookups are fine, but anything
   programmatic or bulk is paywalled, and Pipl went enterprise-only.
4. **Social-media scraping at scale** - the biggest collapse; robust multi-platform
   scraping is now paid-only (Apify, Bright Data), LinkedIn especially.
5. **Automated deepfake detection** - no reliable free tool, and the paid ones are
   imperfect; detection is losing the arms race to generation.
6. **Continuous dark-web monitoring** - ad-hoc search is free; monitoring with
   alerts is enterprise-only.
7. **Court-grade blockchain attribution** - raw tracing is free; identity
   attribution is enterprise-only (Chainalysis, Elliptic, TRM).
8. **Vehicle or license-plate tracking** - no legitimate free OSINT tool.

Everywhere else (network and infrastructure, subdomain enumeration, certificate
transparency, username enumeration, geolocation and GEOINT, metadata forensics,
threat-intel feeds, secret leakage, corporate records, flight and ship tracking,
and wireless) the free and open stack is genuinely strong and actively
maintained.

## Dead or stale, do not reach for these

These still appear on "best OSINT tools" lists but are abandoned or superseded:
BinaryEdge (shut down March 2025), Sublist3r, holehe, Pipl's free tier, snscrape,
Twint, Nitter, Proxycurl, PhoneInfoga, OnionScan, gitrob, shhgit, the Pastebin
scraping API, and the open-source Wappalyzer. Recon-ng still works but is
low-velocity.
