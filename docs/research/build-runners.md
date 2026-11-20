# CI runners for automated CEF builds (research, 2026-11-19)

Question: Where could monthly CEF/Chromium builds for Linux, macOS and Windows run, at what cost, and what do codec patents mean?
Status: Not adopted: builds stay local for now (docs/decisions.md "Build and release"); the options and prices here date from 2026-11-19.

Scope: where to run roughly monthly CEF/Chromium builds (`proprietary_codecs=true ffmpeg_branding=Chrome`) for Linux x64, macOS arm64 (later x64/universal) and Windows x64, driven from GitHub Actions on a public GPL-3.0 repo, without tying up personal machines.

Tags: **[V]** verified from a page fetched in this session (source named), **[I]** inferred or computed by me, **[?]** not found. All prices are USD per the provider's page on 2026-11-19 unless a currency is given. Prices change often; re-check before committing money. Not legal advice.

Baseline used for scaling (measured by the owner): 28-core/31 GB desktop, fetch 8 min, build 2 h 16 min at -j16 without LTO, 38 GB disk, 160 MB output.

---

## 0. Summary

1. **Free GitHub-hosted runners are viable for Linux and Windows on a public repo if the build is split into stages of under 6 h** (the ungoogled-chromium trick, section 4.2). Cost is $0 but wall time is about 12-16 h per platform and the engineering is the heaviest of all options. They are **not** a good fit for macOS (3 vCPU, 7 GB RAM; ungoogled-chromium-macos itself pays for Depot runners).
2. **Cheapest robust path** is self-managed, billed per second: AWS spot via RunsOn (Linux and Windows, RunsOn is free for open source) plus an hourly Apple silicon host or a drop-in macOS runner. About $45-60 per month at 2 builds per platform per month.
3. **Fastest drop-in** is one third-party runner vendor (Depot, Blacksmith, WarpBuild, Namespace) for all three OSes: 1-3 h wall per platform, roughly $45 per build set, about $90-100 per month. GitHub's own larger runners cost more (about $130 per month) and the xlarge Mac has only 14 GB RAM.
4. **Do not try to persist the 29 GB checkout.** GitHub's cache cap is 10 GB per repo; sticky disks cost about $0.50/GB-month (about $15-20 per month for 30-40 GB) while a re-fetch costs 8 min of runner time. Cache compiled output only if/when point-release builds dominate (section 5).
5. **Codec patents are separate from software licences.** Via LA's published AVC terms have a 100,000-unit/year royalty-free tier, but whether a free download counts and who the licensee is needs a lawyer. AAC (Via LA) has no royalty-free tier on the published schedule. Chromium has **no build flag for "use the OS H.264/AAC decoder only"**; `proprietary_codecs=true` pulls in the FFmpeg software decoders under Chrome branding. Details in section 7.

---

## 1. GitHub's own runners

### 1.1 Free standard runners (public repos)

- Standard GitHub-hosted runners in public repos are free and do not use plan minutes. [V] docs.github.com/en/billing/concepts/product-billing/github-actions
- Specs for public repos [V] docs.github.com/en/actions/reference/runners/github-hosted-runners:

| Label | vCPU | RAM | "SSD" per docs | Arch |
|---|---|---|---|---|
| ubuntu-24.04 / latest | 4 | 16 GB | 14 GB | x64 |
| windows-2025 / latest | 4 | 16 GB | 14 GB | x64 |
| macos-latest / macos-26 | 3 (M1) | 7 GB | 14 GB | arm64 |
| macos-15-intel / macos-26-intel | 4 | 14 GB | 14 GB | x64 |

- The 14 GB figure is the docs' number, not the usable free space. Community reports put free space on `/` at about 21-29 GB on Ubuntu, and actions such as `easimon/maximize-build-space` merge `/` and `/mnt` into one volume [V as reported by search results; exact `df` numbers not confirmed]. Chromium (29 GB checkout + out dir + git) needs a volume of 80+ GB, so test `df -h` first. [?]
- ungoogled-chromium-windows builds on `windows-2022` in `C:\ungoogled-chromium-windows`, so Windows has enough disk for a Chromium tree. [V] workflow files (section 4.2)
- Limits [V] docs.github.com/en/actions/reference/limits: job 6 h on GitHub-hosted, **5 days on self-hosted**; cache 10 GB per repo; macOS jobs capped at 5 concurrent on Free/Pro/Team; standard concurrency 20 on Free. Artifact storage by plan is 500 MB (Free) to 50 GB (Enterprise Cloud); the page does not say if public repos are exempt. [?]

### 1.2 Larger runners (paid, also for public repos)

- "Usage in public repositories is also billed at the per-minute rate" and "Larger runners are billed for public repositories." [V] docs.github.com larger-runners concept page and billing page.
- Set up per organisation or enterprise (Settings > Actions > Runners > New GitHub-hosted runner), pick a size, get a label to put in `runs-on`; needs a card on file and a spending limit above zero. [V] The plan requirement (Team/Enterprise vs Free org) is not stated on the pages I fetched. [?]
- Per-minute prices [V] docs.github.com/en/billing/reference/actions-runner-pricing (these include the January 2026 cuts):

| Runner | $/min | $/h |
|---|---|---|
| Linux x64 16-core (64 GB, 600 GB SSD) | 0.042 | 2.52 |
| Linux x64 32-core (128 GB, 1.2 TB) | 0.082 | 4.92 |
| Linux x64 64-core (256 GB, 2 TB) | 0.162 | 9.72 |
| Linux x64 96-core (384 GB, 2 TB) | 0.252 | 15.12 |
| Windows x64 16-core | 0.082 | 4.92 |
| Windows x64 32-core | 0.162 | 9.72 |
| Windows x64 64-core | 0.322 | 19.32 |
| Windows x64 96-core | 0.552 | 33.12 |
| macOS Intel "large" 12-core (30 GB RAM) | 0.077 | 4.62 |
| macOS arm64 "xlarge" 5-core M2 (14 GB RAM, 8 GPU cores) | 0.102 | 6.12 |
| macOS standard 3-4 core (M1/Intel) | 0.062 | 3.72 |

- Specs for Linux/Windows sizes [V] docs.github.com/en/actions/reference/runners/larger-runners: 4 vCPU/16 GB/150 GB up to 96 vCPU/384 GB/2040 GB. macOS xlarge is 5 vCPU, 14 GB RAM; nested virtualisation is not supported on arm64 macOS.
- Caching: only `actions/cache` (10 GB per repo) and artifacts. No persistent disks. Nothing for a 29 GB checkout. [V]
- Self-hosted runner platform fee: announced Dec 2025 ($0.002/min from March 2026), **suspended on 2025-12-17, still suspended per RunsOn's page updated Nov 2026**; GitHub's billing page lists self-hosted as free. [V] runs-on.com/blog/github-self-hosted-runner-fee-2026, docs.github.com billing page. It could return; keep orchestration portable.
- Electron's own CI uses `macos-15-xlarge` for macOS builds, but with remote execution (siso + their RBE cluster), not a full local compile. [V] electron/electron `build.yml` and `pipeline-segment-electron-build.yml`.

---

## 2. Third-party drop-in runner providers

Drop-in means: install a GitHub App, change `runs-on:` to the vendor label. Self-managed means you run the runner fleet in your own cloud account.

| Provider | Linux | Windows | macOS arm64 | Max size seen | Disk | Time limit | OSS / free tier |
|---|---|---|---|---|---|---|---|
| **Depot** | x64 EPYC Genoa and arm64 Graviton4, 2-64 vCPU: 2/$0.006, 4/$0.012, 8/$0.024, 16/$0.048, 32/$0.096, 64/$0.192 per min [V] | 2-64 vCPU, 16 vCPU $0.064, 32 vCPU $0.128, 64 $0.256 [V] | 1 size: 8 vCPU, 24 GB, 400 GB, $0.08/min, macOS 14/15/26 [V] | 64 vCPU/256 GB | 100 GB (2 vCPU) up to 250 GB (64 vCPU) [V] | not stated [?] | Plans: Developer $20/mo (2,000 min), Startup $200/mo (20,000 min) [V]; no OSS discount stated, contact them [V]. Sponsors Helium's runners (Linux, Windows, macOS) [V] |
| **Blacksmith** | x64 2-32 vCPU; 2 vCPU $0.004/min, so 16 vCPU about $0.032, 32 vCPU about $0.064 [V for 2 vCPU rate, I for scaling]; 16 vCPU = 64 GB, 750 GB; 32 vCPU = 128 GB, 1.5 TB [V] | Server 2025 public beta, 2-32 vCPU, 130 GB fixed disk, 2 vCPU = 2x Linux rate so 16 vCPU about $0.064 [V ratio, I price] | **M4**, 6 vCPU/24 GB/150 GB ($0.08/min [V]) and 12 vCPU/48 GB/250 GB (about $0.16/min [I from 20x ratio]) [V specs] | 32 vCPU Linux | see left | not stated; one third-party repo doc suggests 240 min input limit for them, unconfirmed [?] | 3,000 free min/month [V]. OSS sponsorship program for "actively maintained public repos with permissive licenses" (no discount stated, apply) [V]. Zen browser's release jobs run on Blacksmith [V]; note GPL-3.0 may not count as "permissive" [I] |
| **Namespace** | 1 vCPU/2 GB $0.001/min, 4/8 $0.004, 64/512 $0.32 [V]; shapes up to 64 vCPU/512 GB [V] | 1 vCPU $0.002, 16 vCPU/64 GB $0.064/min, up to 32 vCPU [V] | 4 vCPU/7 GB $0.04, **16 vCPU/56 GB $0.20/min**, "Apple M5 Max (or M4 Pro on some configurations)" [V] | 64 vCPU Linux, 16 vCPU macOS | not stated [?] | **Max instance duration: 3 h Developer, 5 h Team, up to 24 h Business/Enterprise** [V] | 30-day trial, OSS programme exists (terms [?]) [V]. Cache volumes: snapshots $0.002/GB-h, storage $0.0048/GB-day (about $0.14/GB-month); `nscloud-checkout-action` has a git mirror cache [V] |
| **WarpBuild** | x64 2/$0.004, 4/$0.008, 8/$0.016, 16/$0.032, 32/$0.064 per min [V] | 4/$0.016, 8/$0.032, 16/$0.064, 32/$0.128 [V] | M4/M5 Pro 6 vCPU/14 GB $0.08, 12 vCPU/28 GB $0.16 [V] | 32 vCPU cloud; **BYOC** on AWS/GCP/Azure at $0.002/min, up to larger instances [V] | not stated [?] | not stated [?] | no OSS discount listed; cache $0.20/GB-month; snapshots $0.025/h each [V] |
| **Ubicloud** | x64 2/$0.0012, 4/$0.0025, 8/$0.005, 16/$0.010 per min (Standard); Premium 16 vCPU $0.016 [V]; 1,250 free min/month [V] | not listed [V says Linux only] | not listed | **16 vCPU/64 GB** max on that page | not stated [?] | not stated [?] | Open source, can self-host; 30 GB cache free (Standard) [V] |
| **RunsOn** (self-hosted on **your** AWS) | any EC2 type, spot or on-demand, per-second AWS billing, no markup [V] | supported (EC2 Windows) [V] | not mentioned, so no [V that page does not list it] | any EC2 size (e.g. c7a.16xlarge 64 vCPU) | EBS you choose | GH self-hosted 5 days [V] | **Licence €0 for OSS/non-profit/educational/personal use with a public link back**; commercial from €300/year; AWS control plane from about $7/month [V] |
| **Cirrus Runners / Cirrus CI** | | | | | | | **Shut down**: Cirrus CI ended 2026-06-01, Cirrus Labs joined OpenAI, Cirrus Runners takes no new customers, Tart/Orchard being re-licensed permissively [V via search results, MacStadium and vendor blogs]. Do not build on it. |
| **BuildJet** | | | | | | | **Shut down 2026-03-31** [V via search results] |
| **Actuated** | Firecracker microVMs on your own bare metal; Linux only | no | no | your hardware | | | about $250/month in third-party listings; I did not reach its own pricing page [?] |
| **Tenki** | | | M4 Pro macOS at $0.025/vCPU/min per a third-party snippet; the docs URL redirected to another domain [?] | | | | not verified |

Other notes:
- None of the drop-in vendors documents a single 6 h+ guarantee. For Depot, Blacksmith, WarpBuild, Ubicloud, Tenki I found no stated max job duration. Run a 7-hour dummy job before committing. [?]
- Namespace's limit matters: a Windows 32 vCPU build (about 2.5-3 h by my estimate below) fits Developer's 3 h only barely; plan for Team (5 h) or use 32 vCPU+ sizes. [I]
- Helium (a Chromium fork) says Depot "sponsors our runners, which handle the Windows builds" and "macOS builds", and that this makes builds "at least 8 times faster than with GitHub-hosted runners". [V] github.com/imputnet/helium-windows and helium-macos READMEs.
- ungoogled-chromium-macos runs its build on `os: depot-macos-latest` (arm64 and x86_64 matrix) [V] workflow `build.yml`.

---

## 3. Rent-by-the-hour machines with an on-demand self-hosted runner

### 3.1 How the "start on demand" pattern works [I]
A cheap workflow (free `ubuntu-slim`/`ubuntu-latest`) job: (1) calls the cloud API to create the host, passing a user-data script that downloads the Actions runner, registers it with `--ephemeral` and a unique label using a short-lived registration token minted by the workflow; (2) the real build job has `runs-on: [self-hosted, wubble-mac-<run_id>]` and `needs:` the first job; (3) a final `if: always()` job deletes the host. Add a scheduled janitor that terminates any leaked host (important for the 24 h-minimum hosts and for cost safety). Restrict self-hosted runners to `workflow_dispatch`, tags and `schedule`, never to fork PRs. GitHub docs also tell you to use runner groups with selected repositories. [V for the runner-group guidance, I for the rest]
RunsOn and WarpBuild BYOC implement this for you on AWS/GCP/Azure; Scaleway, Hetzner and MacStadium need your own script.

### 3.2 Linux and Windows

| Option | Spec | Price | Notes |
|---|---|---|---|
| **AWS c7a.8xlarge** | 32 vCPU, 64 GB, EBS | On-demand $1.642/h, **spot $0.612/h** (us-east-1) [V instances.vantage.sh] | Windows adds about $0.046 per vCPU-hour licence [V AWS page states this figure as the Windows Server vCPU rate] so about $3.11/h on-demand [I] |
| **AWS c7a.16xlarge** | 64 vCPU, 128 GB | $3.284/h on-demand, spot $0.966/h [V] | |
| AWS c7i.12xlarge | 48 vCPU, 96 GB | $2.142/h, spot $1.070 [V] | |
| AWS c7i.24xlarge | 96 vCPU, 192 GB | $4.284/h, spot $2.096 [V] | |
| EC2 billing | per second, 60 s minimum for Linux/Windows [V aws.amazon.com/ec2/pricing/on-demand] | | EBS gp3 for a few hours is cents; a 30 GB snapshot costs about $1.50/month [I] |
| **Hetzner Cloud CCX33/43/53/63** | 8/32 GB, 16/64 GB, 32/128 GB, **48/192 GB, 960 GB NVMe** | €0.1891, €0.3766, €0.7267, **€1.1626 per hour**, capped at €117.99/€234.99/€453.49/€725.49 per month (excl. VAT) [V hetzner.com/cloud/general-purpose] | Billed hourly, pay only the hours the server existed. **Linux only in practice** (no bundled Windows or macOS) [I]. EU cheapest; US +20% [V] |
| Hetzner AX dedicated | AX102 (16-core Ryzen) from €159/mo, €129 setup; AX162 (48-core EPYC option) from €319/mo, €304 setup [V hetzner.com matrix-ax] | | A setup fee makes ephemeral use pointless; only worthwhile as a permanent box (which defeats "no personal machine tied up" less than a laptop, but is a standing cost) |
| **Azure Standard_F64s_v2** | 64 vCPU, 128 GB | about $2.706/h Linux in East US per third-party listings [V third-party only] | Windows price [?] |
| GCP | c3/c2 families | not fetched [?] | |

### 3.3 macOS

Apple's licence facts: macOS may be virtualised on Apple hardware with at most **two** macOS instances per host, and the licence limits virtualisation to development, testing during development, macOS Server, and personal use; leased Apple software must be used for "Permitted Developer Services" with each lessee agreeing to the licence [V via search snippets of Apple developer forums and eclecticlight.co; read the actual current SLA]. Cloud Mac providers are set up around this: AWS says the **24-hour minimum host allocation exists "to comply with the Apple macOS Software License Agreement"** [V aws.amazon.com/ec2/instance-types/mac].

| Provider | Hardware | Price | Billing minimum |
|---|---|---|---|
| **AWS EC2 Mac** `mac-m4pro.metal` | M4 Pro, 14 cores (10P+4E), 48 GB, 1.9 TB local NVMe | **$1.97/h** [V vantage] | **24 h per Dedicated Host** [V AWS], so **$47.3 per allocation** [I] |
| AWS `mac2-m2pro.metal` | M2 Pro 12 cores, 32 GB | $1.56/h [V] | 24 h, **$37.4** per allocation [I] |
| AWS `mac-m4max.metal` | M4 Max 16 cores, 128 GB | $6.25/h [V] | 24 h, **$150** [I] |
| AWS `mac-m4.metal` | M4 10 cores, 24 GB | price not fetched [?] | 24 h |
| **Scaleway** Mac mini M4 Pro (M4-XL: 14-core CPU, 20-core GPU, 64 GB per its FR page) | | **€0.49/h** or about €335/month [V via search result quoting the French pricing page; direct pages returned 403] | 24 h minimum stated for the M2 Pro runners page and third-party comparison for M4; **assume 24 h also for M4 Pro**, so about **€11.8 per allocation** [I] |
| **MacStadium** dedicated Mac mini | M4 Pro 12-core 48 GB 1 TB = $349/month; M4 24 GB = $249/month; M2 Pro 12-core 32 GB $349/month [V macstadium.com/pricing] | monthly only on that page, hourly not mentioned; Orka (on-demand) needs a sales quote | monthly prepaid [V] |
| **Oakhost**, MacinCloud, RentAMac | | RentAMac M4 from $3.30/day, no hourly [V via search snippet]; Oakhost and MacinCloud prices not found [?] | |

Takeaway [I]: a 24 h minimum is irrelevant when a build takes about 1.5 h and the host also does signing, notarisation, the x64/universal build and smoke tests in the same window. Scaleway (about €12) is far cheaper than EC2 Mac ($47); EC2 Mac is the better-supported, scriptable choice (API, IAM, tags, SSM) and has the 1.9 TB local disk.

---

## 4. What comparable projects do

### 4.1 Summary table

| Project | How builds are produced | Source |
|---|---|---|
| **CEF (official, Spotify-hosted cef-builds)** | The index page only shows builds; I found no documentation of the build farm. Official builds ship **without** proprietary codecs (default `proprietary_codecs=false`); users wanting H.264/AAC set `GN_DEFINES="proprietary_codecs=true ffmpeg_branding=Chrome"` and run `automate-git.py` themselves. [V for the default and the GN args, via CEF issues/forums in search results; farm details ?] | cef-builds.spotifycdn.com, search results |
| **Electron** | GitHub Actions. Linux builds on self-hosted ARC runners (`electron-arc-centralus-linux-amd64-32core`, 32 cores) in a build container; Windows on `garm-windows-x64-32core` (self-hosted); macOS on `macos-15-xlarge` (GitHub-hosted M2). Remote execution via **siso + a reclient-compatible RBE backend** restricted to maintainers (cache-only tokens for others); a patched siso binary per host OS; cross-instance cache mounts for the checkout. No `timeout-minutes` set in those files. Historically goma then reclient. | [V] electron `build.yml`, `pipeline-segment-electron-build.yml`, electronjs.org docs reclient page |
| **Brave** | Private Jenkins (13 pipelines, VPN-only), sccache in PR builds; EngFlow case study cites about 120 min per build down to about 15 min with remote execution. | [V] PR Builder wiki, EngFlow case study snippet |
| **ungoogled-chromium** | The core repo ships patches only; per-platform repos exist. Windows and macOS binaries are built on GitHub Actions with the **multi-stage resume trick** (4.2). macOS runs on **Depot** runners; Windows on `windows-2022` (free). Binaries are published to GitHub releases / "contributor binaries". | [V] workflow files |
| **Helium** | Linux, Windows, macOS builds on runners sponsored by **Depot**; claims 8x faster than GitHub-hosted. | [V] READMEs |
| **Zen** (Firefox fork) | GitHub Actions, Linux jobs on **Blacksmith** `blacksmith-16vcpu-ubuntu-2404` (x64 release) and 8 vCPU (aarch64), no self-hosted runners in those files, sccache switch, PGO profile cache via `actions/cache`, MAR signing from secrets. | [V] zen-browser/desktop workflows |
| **Thorium** | No evidence of CI; docs describe manual builds; separate repos per platform. | [?] |
| **Vivaldi, Floorp** | not researched (closed pipeline / not found) | [?] |

Pattern: **nobody builds full Chromium on free GitHub-hosted runners for macOS**; the successful ones use sponsored or paid fast runners (Depot, Blacksmith) or their own clusters with remote execution (Electron, Brave).

### 4.2 The ungoogled-chromium multi-stage trick (precise)

Source: ungoogled-chromium-windows `.github/workflows/reusable-build.yml`, `.github/actions/stage/{action.yml,index.js}`, `build.py`; ungoogled-chromium-macos `building.yml`, `github_build.sh`. [V]

**Windows (free `windows-2022`, 4 vCPU):**
1. Workflow has `build-1` ... `build-16`, a strict `needs:` chain, all on `windows-2022`; final `complete` job (ubuntu) fails with "Build did not finish after 16 stages" unless the last stage reports `finished == true`. No `timeout-minutes` set.
2. Each job: checkout, `prepare` action (toolchain/python), then the `stage` action. If a previous stage already reported `finished`, the stage exits immediately.
3. `build.py --ci -j 2`. In `--ci` mode every expensive step (source download/prune/patch, GN bootstrap, `gn gen`, bindgen, crubit) is skipped if its output already exists, so a rerun just resumes. Ninja is run through a wrapper with a **hard-coded 3.5 h timeout**. At the limit it sends CTRL_BREAK to the Ninja process group three times, waits 10 s, kills it, and exits **124**. Exit 124 means "not finished" and skips packaging.
4. On exit 124 the stage action waits 5 s then runs `7z` (compression level 3, **timestamps preserved**) over only `build\src` into `artifacts.zip` and uploads it as an artifact named `build-artifact` (4-day retention, no extra compression, retried up to 5 times, delete-then-upload).
5. The next job downloads that artifact into the build dir, extracts it with 7z (overwrite), deletes the zip, and runs `build.py --ci` again. Ninja resumes from `.ninja_log`/`.ninja_deps` and file mtimes inside `build\src\out\Default`. Preserving mtimes is what makes Ninja not rebuild.
6. On exit 0 the final packages (zip, installer) are uploaded as a separate artifact and `finished=true`.

**macOS (`depot-macos-latest`):** a matrix of arm64 and x86_64 calls `building.yml` with chained jobs `retrieve-resources` then `build_job_01` ... `build_job_20`. Each build job runs only if the previous job's `status` output is `running`. `github_build.sh` computes the time left against a 5 h budget (18,000 s, reserving about 1 h of the 6 h limit for setup and artifact transfer), runs `timeout -k 7m -s SIGTERM <remaining> ninja`, and on timeout (124) exits 0 with `status=running`; success sets `status=finished`. State moves in artifacts `github_build_artifact_<arch>` and `github_build_logs_<arch>`, uploaded with `overwrite: true`. Each job also deletes `$ANDROID_HOME` and disables Spotlight.

**What it means for wubble [I]:**
- CEF's build is a Chromium build plus libcef, so the same scheme applies: stage 0 `automate-git.py --no-build --no-distrib` style sync (the script has flags for these), then stages that run `autoninja` on the CEF targets with a time-boxed `timeout`, then a final stage that creates the distrib. Keep the whole `src/` (or at least `src/out/Release_GN_x64` plus everything it depends on) in the hand-off artifact.
- The hand-off artifact is the problem: the Windows stage uploads the whole `build\src` tree. For a Chromium tree that is tens of GB. I did **not** find a documented per-artifact size limit or whether public repos are exempt from the artifact storage quota (Free plan quota is 500 MB; per-artifact figures in community threads (500 MB, 5 GB) are unofficial). ungoogled-chromium evidently succeeds, so it works in practice, but measure it. [?] An alternative hand-off that avoids artifacts is object storage (R2 at $0.015/GB-month, free egress [V]) via `rclone`/`aws s3 cp`, which also removes the 4-day retention concern.
- 4 vCPU free runners: by my scaling (section 6) Linux needs about 3 stages and Windows 4-5. The 16-stage maximum in ungoogled shows what the worst case looks like. [I]
- RAM 16 GB on free x64 runners is marginal for linking `libcef`/`chrome`; add swap (`maximize-build-space` creates swap). On macOS (7 GB) it is not viable. [I]

---

## 5. Remote caching and execution

- **siso** is Chromium's default build tool for external developers (reclient removed at end of September 2025; Ninja unsupported afterwards). Remote execution targets any REAPI-compatible backend: `use_siso=true`, `use_remoteexec=true`, plus `reapi_address`, `reapi_instance`, `reapi_backend_config_path`, and mTLS/gcloud/credential-helper auth. Google's own RBE is invite-only and "best-effort" for external contributors. [V] chromium build_instructions.md and chromium-dev PSA.
- **EngFlow** documents Siso with Chromium 145+ on Linux, macOS and Windows (mTLS cert; `custom_vars` in `.gclient`). No pricing or cache-only mode in that doc. [V]
- **NativeLink** (open source, FSL-1.1-Apache-2.0 for most files) lists Siso as a supported integration; self-hosting is free, but production use of some features (metrics, persistent workers) is BSL; enterprise licence is quoted. [V nativelink.com pricing; the Siso integration claim from search result]
- **Buildbarn** is mentioned in a 2025 Chromium-dev thread as what the Electron project is said to run for Linux workers while cross-compiling Windows/Mac [search result; Electron docs only say "reclient-compatible RBE", so treat as unconfirmed [?]].
- **BuildBuddy**: Free (Personal) tier "for individual developers, small teams and open source projects" includes 100 GB cache transfer and up to 80 Linux cores of remote execution; Team is pay-as-you-go (rate not shown); no hard caps; MIT-licensed OSS version for self-hosting. [V buildbuddy.io/pricing] Whether Siso works well against it for Chromium: no benchmarks found. [?]
- **sccache**: supports S3, R2, GCS, Azure, Redis, GitHub Actions cache (10 GB limit makes that backend unusable for Chromium) and local disk. Chromium has `cc_wrapper` plumbing and had an Electron-tested sccache change for Windows; Brave uses sccache in PR builds. [V] No published CEF/Chromium cache-hit numbers found. [?]
- **Realistic for a small project [I]:**
  - A monthly milestone bump changes essentially every file, so a compiler cache buys little. Point releases change a small fraction of files, so a cache can cut a 2 h build to maybe 20-40 min if the build path, flags and toolchain are byte-identical (use a fixed container path).
  - Storage is cheap: R2 holds 40 GB for about $0.60/month plus operations (Class A $4.50 per million). [V R2 pricing]
  - The siso remote execution route needs Linux workers with the Chromium sysroot, and cross-compiling Windows/macOS from Linux workers (Electron does) or native Mac/Windows workers (EngFlow). That is weeks of work; skip for v1.
  - Do the simplest thing first: bigger machine (linear cost, sublinear time) and, later, an R2 sccache for point releases.
- **Persisting the checkout**: Namespace cache volumes about $0.14/GB-month, Blacksmith sticky disks $0.50/GB-month, WarpBuild $0.20/GB-month, Depot cache $0.20/GB-month, GH cache 10 GB max. For a 30-40 GB tree that is $5-20 per month, versus re-fetching 29 GB in about 8-12 min of runner time (about $0.10-0.80 on any option here). **Re-fetch every time.** [I]

---

## 6. Cost and wall-time estimates for one full CEF build

### 6.1 Model (assumptions, all [I])

- **Linux work** W_L = about 45 vCPU-hours (range 36-55). Basis: 136 min x 16 jobs = 36 thread-hours on the desktop; cloud vCPUs are SMT threads and slower, so x1.25. Add about 0.3 h fixed (fetch 8-12 min, GN, link, packaging).
- Parallel efficiency about 85% to 32 vCPU, lower beyond (long link tail, and the fixed 0.3 h).
- **Windows** about 1.4x Linux (MSVC-ABI clang-cl, larger headers, SDK) so W_W about 63 vCPU-hours plus about 0.5 h fixed.
- **macOS**: public data points for a full `chrome` build: **M2 Max (12 cores) 1 h 30 min**, **M2 Ultra (24 cores) 1 h 06 min**, M2 debug 1 h 49 [V medium.com/@solomonkinard posts, 2023, unknown GN args]. A chromium-dev poster claims **under 20 min on an M5 Max** [unverified claim from search result [?]]. CEF adds libcef on top of the chrome target; assume the same order of magnitude. Scaled by core count and generation: GitHub xlarge (5-core M2) 3.5-5 h; Depot 8 vCPU 2.5-3.5 h; M4 12 vCPU 1.5-2 h; Namespace 16 vCPU (M5 Max/M4 Pro) 1-1.5 h; EC2 M4 Pro 14-core about 1-1.5 h.

### 6.2 Linux x64

| Option | Size | Wall | Cost per build |
|---|---|---|---|
| GitHub free, staged | 4 vCPU | about 12 h in 3 stages x about 4.5 h plus 2 artifact hops | $0 |
| GitHub larger | 16-core | about 3.3 h | about $8 |
| GitHub larger | 32-core | about 1.9 h | about $9 |
| GitHub larger | 64-core | about 1.1 h | about $11 |
| Depot | 16 vCPU / 32 vCPU | 3.3 h / 1.9 h | $9.5 / $11 |
| Blacksmith | 16 / 32 vCPU | 3.3 h / 1.9 h | $6.3 / $7.3 (price scaled from 2 vCPU rate) |
| WarpBuild | 16 / 32 vCPU | 3.3 h / 1.9 h | $6.3 / $7.3 |
| Ubicloud | 16 vCPU Standard | 3.3 h | about $2 (Premium about $3.2); disk and job limit unknown [?] |
| Namespace | 32 vCPU | 1.9 h | price for that shape not on page [?] |
| AWS c7a.8xlarge via RunsOn | 32 vCPU | 1.9 h | spot about $1.2, on-demand about $3.1 |
| AWS c7a.16xlarge via RunsOn | 64 vCPU | about 1.1 h | spot about $1.1, on-demand about $3.8 |
| Hetzner CCX63 | 48 vCPU | about 1.5 h | about €1.7 (needs own runner script) |

### 6.3 Windows x64

| Option | Size | Wall | Cost per build |
|---|---|---|---|
| GitHub free, staged | 4 vCPU | about 16-20 h in 4-5 stages | $0 |
| GitHub larger | 16 / 32 / 64-core | 4.4 / 2.8 / 1.6 h | $21.6 / $27 / $31 |
| Depot | 16 / 32 vCPU | 4.4 / 2.8 h | $17 / $21.5 |
| WarpBuild | 16 / 32 vCPU | 4.4 / 2.8 h | $17 / $21.5 |
| Blacksmith (beta) | 16 vCPU | 4.4 h | about $17 |
| Namespace | 16 vCPU | 4.4 h | $16.9 (needs Team plan for 5 h limit if over 3 h) |
| AWS c7a.8xlarge Windows via RunsOn | 32 vCPU | 2.8 h | spot about $5.8, on-demand about $8.7 |

### 6.4 macOS arm64 (and x64 as a second build in the same window)

| Option | Hardware | Wall | Cost per build |
|---|---|---|---|
| GitHub free `macos-latest` | M1, 3 vCPU, 7 GB | about 10+ h, 7 GB RAM | $0, **not viable** [I] |
| GitHub `macos-15-xlarge` | M2, 5 vCPU, 14 GB | 3.5-5 h, near the 6 h cap | $21-31 |
| Depot | 8 vCPU / 24 GB / 400 GB | 2.5-3.5 h | $12-17 |
| Blacksmith / WarpBuild | M4 12 vCPU | 1.5-2 h | about $14-19 |
| Namespace | 16 vCPU / 56 GB | 1-1.5 h | $12-18 |
| AWS EC2 `mac-m4pro.metal` | M4 Pro 14-core | 1-1.5 h | **$47 per 24 h allocation**, can include arm64 + x64 + signing |
| Scaleway M4 Pro | 14-core, 64 GB | 1-1.5 h | **about €12 per 24 h allocation** (if 24 h applies) |
| MacStadium dedicated M4 Pro | 12-core, 48 GB | 1.5-2 h | $349/month flat |

### 6.5 Monthly cost, 2 builds per platform per month (6 builds; time-of-use prices above; storage extra)

| Setup | Linux | Windows | macOS | Orchestration | **Total / month** |
|---|---|---|---|---|---|
| A. Free hosted staged for Linux + Windows, hourly cheap Mac | $0 | $0 | 2 x about $12-18 (Namespace/Depot) or about €24 (Scaleway) | free | **about $25-40** |
| B. RunsOn on AWS spot for Linux + Windows, hourly Mac | about $2.4 | about $12 | about $24-36 | about $7-15 AWS control plane, RunsOn free for OSS | **about $45-65** |
| C. One drop-in vendor for all (Depot or Blacksmith or WarpBuild, 32/32/12 vCPU) | $15-22 | $43-55 | $28-38 | Depot $20 plan | **about $90-135** |
| D. GitHub larger runners (32-core / 32-core / xlarge) | $18 | $54 | $42-62 | none | **about $115-135** |
| E. Monthly dedicated Mac mini + others | as B | as B | $349 | | not recommended, about $400/month |

Storage: checkout not cached (above). Optional sccache/Siso cache on R2: 40 GB about $0.60/month plus a few cents of operations. Release assets (160 MB x 3 platforms x 2 per month) are free on GitHub Releases. R2 would only be a hand-off store for option A: transient, pennies.

---

## 7. Codec patent licensing (not legal advice)

### 7.1 Software licence vs patent licence (fact)
- Chromium's source is BSD-licensed; its FFmpeg is built as LGPL. These grant **copyright** permissions. They say nothing about **patents** held by codec pool members. The Chromium docs mark H.264/AVC, H.265/HEVC, AAC and xHE-AAC as "**Proprietary ... (Limited to Google Chrome)**"; `ffmpeg_branding=Chrome` "includes additional proprietary codecs (MP3, etc..) for use with Google Chrome"; `proprietary_codecs=false` is the default. [V] chromium.org/audio-video
- CEF's default builds do not include them; anyone enabling them via `proprietary_codecs=true ffmpeg_branding=Chrome` is told by CEF/Chromium community material that it requires licences with the patent holders. [V via search results of CEF issues/forums; no official legal statement]

### 7.2 AVC / H.264 (Via LA) [V from via-la.com AVC licence summary page]
- A "unit" is a decoder, an encoder, or one product with both.
- Products for end users and PCs/OS: **first 100,000 units per year are $0.00** (per legal entity in an affiliated group); 100,001-5,000,000: **$0.20/unit**; above 5,000,000: **$0.10/unit**; resets annually. Enterprise cap **$9.75M/year** (2017 onwards).
- Streaming and distribution fees (title-by-title, subscriber/MAU-tiered participation fees with a flat **$100K** small/nascent tier for OTT, FAST, social media, cloud gaming) apply to the **content service**, not to a browser.
- Separately, MPEG LA announced in August 2010 that **internet video that is free to end users** (Internet Broadcast AVC Video) stays royalty-free for the life of the licence; devices that decode and encode AVC "remain licensed" and royalty-bearing under the unit schedule. [V engadget and design-reuse reports of the press release] So the "free internet video" provision protects the **content distributor**, not the **decoder maker**. The decoder in a browser is a "unit"; for a free download the first 100,000 units per year are free.
- **Needs a lawyer:** whether each download counts as a "unit"; whether the project/foundation or each distributor is the licensee; whether the free tier requires signing the licence anyway (to be licensed, you normally must sign up); how "affiliated group" applies; whether an open-source project that only publishes source is a licensee at all; patent expiry timing; jurisdictions (US vs elsewhere).

### 7.3 AAC (Via LA) [V from via-la.com/licensing-2/aac]
- Per-unit schedule from **$0.98** (units 1-500,000) down to **$0.10** (75,000,001+); multichannel counts as 1.5 units; **one-time initial fee $15,000, or $1,000 for small entities** (15 or fewer employees and under $1M revenue); five-year term. **No royalty-free tier** is listed for end-user encoders or decoders. Fees are due on the sale of encoder/decoder products; AAC bitstreams carry no fee. Another Via LA page showed a $0.64 first tier, so the schedules may differ by version [?]. Via LA says Android OEMs need their own AAC licences because Google's licence does not cover them. [V]
- Needs a lawyer: whether free distribution counts as "sale"; whether a small-entity $1,000 fee plus per-unit rates apply to a free software download.

### 7.4 HEVC [mixed]
- HEVC pools: Access Advance (HEVC Advance) licenses decoders and encoders in consumer products, including software and cloud; per-category caps. Via LA's HEVC/VVC programme was **acquired by Access Advance as of 2025-12-15** and rates were to be re-aligned on **2026-01-01**. [V from search-result snippets of accessadvance.com and via-la.com; pages not fully read]
- Chromium: `enable_hevc_parser_and_hw_decoder` enables the parser and **hardware** decode on Windows, Apple, Android, Linux; `enable_platform_hevc` enables demuxing but "Actual decoding must come from the platform". [V] media_options.gni. So Chromium only ever decodes HEVC through OS/hardware, which is why browsers rely on platform decoders (the OS vendor or GPU maker holds the licence). The reason most browsers do so is my inference [I].

### 7.5 What other projects do
- **Firefox/Mozilla**: downloads Cisco's precompiled **OpenH264** binary (BSD source; Cisco pays the royalties for that specific binary; anyone building from source takes on the obligation); used for WebRTC and, per Mozilla's 2013 post, also for `<video>` on platforms without an OS decoder; otherwise Firefox uses the OS decoder where available. Fedora only enables it for WebRTC. [V via search results, 2013-14 sources; current behaviour on Windows Media Foundation not re-verified [?]]
- **Bromite / Cromite**: `proprietary_codecs=true` default; the Cromite issue discusses the risk without conclusion; the suggested "safest" approach was to use libraries supplied by the OS distributor. [V cromite issue #710]
- **ungoogled-chromium, Brave, Vivaldi, Thorium, Electron apps, Linux distro Chromium**: I found no official statement from Brave or Vivaldi on who pays for H.264/AAC [?]. Common practice (my recollection, not verified here): ungoogled-chromium and Thorium enable proprietary codecs in their flags; Debian/Ubuntu ship `chromium-codecs-ffmpeg-extra` separately; Electron ships Chromium-branded ffmpeg by default and apps swap `libffmpeg`. [I]
- **Chromium debate**: Chrome 2011 blog and "H264 licensing and royalties with Chromium" threads treat the codec patents as the responsibility of whoever distributes the build. [V title/snippet level only]

### 7.6 "Use only OS decoders" via build flags [mostly I]
- **There is no `enable_platform_h264` flag in `media/media_options.gni`** (the file lists `media_use_openh264`, HEVC, AC3/EAC3, MPEG-H, DTS, Dolby Vision platform flags). [V] H.264 playback in Chromium is gated by `proprietary_codecs=true` (which also controls `canPlayType()`), and `ffmpeg_branding=Chrome` switches the in-tree FFmpeg to the config that includes the H.264/AAC/MP3 software decoders. [V chromium.org/audio-video]
- On macOS and Windows Chromium has hardware H.264 decoders via VideoToolbox and D3D11/Media Foundation, and platform audio decoders exist (`AudioToolboxAudioDecoder`, `MediaFoundationAudioDecoder`) that are used for some formats such as AC3/EAC3, and xHE-AAC "requires native OS support". [V partial, search results] I could **not** confirm that Chromium decodes regular AAC-LC through the OS on macOS/Windows rather than FFmpeg. [?]
- So to ship "OS decoders only" you would likely need: `proprietary_codecs=true` (to enable the H.264/AAC code paths and parsers), **plus a custom FFmpeg config/patch that removes the software H.264/AAC decoders** while leaving the hardware/OS paths. That is a local patch to `third_party/ffmpeg` configs, not a documented flag. Linux has no OS-supplied licensed decoder, so on Linux this means VA-API hardware only (still H.264-capable via the GPU driver) or loading the user's system FFmpeg. [I]
- This is a project-sized investigation of its own (and affects what `canPlayType` reports); put it on the roadmap, do not assume the flags above exist.

---

## 8. Ranked recommendation

### 8.1 Cheapest fully automated from GitHub Actions for all three OSes: Option A (about $25-40/month)
- **Linux and Windows: free GitHub-hosted runners with the ungoogled-style staged build** (4 vCPU, 6 h cap, resume via 7z/tar of `src` plus mtimes preserved). Wall about 12 h (Linux) and 16-20 h (Windows) per build; both run in parallel on a monthly cron or tag.
- **macOS: pay per build** with a drop-in vendor with Apple silicon (Namespace 16 vCPU about $0.20/min, Depot 8 vCPU $0.08/min, Blacksmith/WarpBuild M4 12 vCPU about $0.16/min) at about $12-18 per build, or Scaleway M4 Pro (about €12 per 24 h allocation, self-managed runner).
- One-off setup:
  1. Staged build driver (`stage` composite action: time-boxed Ninja, tar/7z of `src` with mtimes preserved, artifact or R2 upload/download, exit-124 protocol, `finished` output). Port from ungoogled-chromium-windows (BSD-3-Clause; check licence and add to THIRD_PARTY_NOTICES.md). 2-4 days plus iteration.
  2. Verify real free disk and per-artifact size on both free runners; likely need `maximize-build-space` and swap; 1 day.
  3. macOS: either vendor account plus label (hours) or the Scaleway lifecycle workflow (allocate, register ephemeral runner, build, sign, notarise, delete, janitor; 1-2 days). Store Developer ID cert (p12), password and App Store Connect API key as repo secrets; import into a temporary keychain in the job.
  4. Release step: upload to GitHub Releases from a small hosted job.
- Caveat: slowest and fiddliest; Linux still needs `--no-sandbox` style CI quirks to be worked out; a 4-5 stage Windows chain has more failure points. A cheap fallback is option B for Windows only.

### 8.2 More robust, still cheap: Option B (about $45-65/month)
RunsOn (free for OSS, link back) on your AWS account: c7a.8xlarge or 16xlarge spot for Linux and Windows (about $1-6 per build), single-job builds well under any limit (self-hosted cap is 5 days), 600+ GB EBS, no artifact hand-off. macOS as in A. Setup: AWS account and CloudFormation/Terraform for RunsOn (about 1 day), runner configs for 64 GB/600 GB Linux and Windows images with VS 2022/2026, Windows SDK and depot_tools, and spot-interruption handling (retry the job). Watch the spot interruption risk on 1-3 h jobs and have an on-demand fallback label.

### 8.3 Faster paid alternative: Option C (about $90-135/month)
One drop-in vendor for everything: **Depot** (Linux 16-64 vCPU up to 250 GB, Windows 16-64 vCPU, macOS 8 vCPU/400 GB; used by Helium and ungoogled-chromium-macos), or **Blacksmith** (Linux 32 vCPU 1.5 TB, M4 macOS 12 vCPU, Windows beta) or **WarpBuild**. Wall about 1-3 h per platform. Setup: install the GitHub App, change `runs-on`, run a 7 h dummy job to confirm no hidden limit, apply to the vendor's OSS/sponsorship programmes first (Blacksmith's asks for permissive licences and may not accept GPL; Depot sponsors Helium; Namespace has an OSS programme; none state terms). GitHub larger runners are the no-new-vendor alternative at about $115-135/month; the Mac xlarge (14 GB) is the weak point.

### 8.4 Before committing
1. Measure real vCPU-hours of the CEF build on a 16 or 32 vCPU cloud runner once (one dry run on each OS) and replace the model in section 6.1.
2. Ask the vendors (Depot, Namespace, Blacksmith) about OSS sponsorship and the maximum job time.
3. Get legal advice on shipping `proprietary_codecs=true ffmpeg_branding=Chrome` binaries before the first public release (section 7); consider the OS-decoder-only patch path (7.6) as a mitigation project.

---

## 9. Sources (fetched this session)

GitHub: docs.github.com (actions-runner-pricing, larger-runners concept and reference, github-hosted-runners reference, actions limits, billing for Actions, manage-larger-runners); runs-on.com (pricing, github-self-hosted-runner-fee-2026). Providers: blacksmith.sh/pricing, docs.blacksmith.sh/blacksmith-runners/overview, namespace.so/pricing and docs, depot.dev/pricing and /docs/github-actions/runner-types, warpbuild.com/pricing, ubicloud.com/use-cases/github-actions, cirrus-runners.app, buildbuddy.io/pricing, nativelink.com/pricing, developers.cloudflare.com/r2/pricing. Clouds: instances.vantage.sh (c7i, c7a, mac-m4pro, mac-m4max, mac2-m2pro), aws.amazon.com/ec2/instance-types/mac and /pricing/on-demand, hetzner.com/cloud/general-purpose and /dedicated-rootserver/matrix-ax, macstadium.com/pricing; Scaleway pages returned 403 so figures come from search-result snippets. Projects: ungoogled-chromium-windows and -macos workflow/action/script files, electron/electron workflows and docs, zen-browser/desktop workflows, imputnet/helium-windows and -macos READMEs, brave-browser wiki via search, chromium build_instructions.md and siso PSA, docs.engflow.com siso page, chromium.org/audio-video, media_options.gni, cromite issue 710. Patents: via-la.com AVC and AAC pages, MPEG LA 2010 press-release coverage, Access Advance/Via LA HEVC pages (snippets).
