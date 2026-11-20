# wubble as an MCP server: plan

Question: How can any agent harness drive wubble over MCP, in the user's real browser, without handing it the user's whole web life?
Status: plan, not built; decided 2026-11-19 to build after the first release.

Date: 2026-11-19. Plan only, no code. Tags: **[V]** verified from a cited source or from wubble's code, **[I]** inferred
or from background knowledge (check before relying), **[?]** not found. Sources are numbered at the end.
Spec version throughout: MCP **2026-07-28**, the current revision [1]. "Owner" marks direction the owner has decided.

## 1. Summary and recommendation

Build it, after the first release, as **sessions**. The user asks their harness for something; the agent calls
`start_session`; a wubble window opens with a native setup sheet; the user approves the client, picks the scope
(Observer or Actor), the sites it may visit, and the **secrets and variables** it may use by name (owner: like env
vars; the agent sees `GITHUB_PASSWORD`, never the password); then closes the sheet to let it run **headless** or
keeps the window to watch the session's Space live. Each session is bound to one Space and its own container. Actors
get Playwright-level control through accessibility refs (click, type, hover, drag, select, dialogs, uploads, tabs,
waits, screenshots) but no raw script, cookies or request interception. Native Slint prompts ("airlocks") stand at the
boundaries the engine can detect for certain. The server lives inside the browser; a harness reaches it through a
small stdio command, `wubble mcp`, relaying to a private local socket; nothing listens on TCP. Attaching an agent to
a real Chromium is not new: Playwright MCP's extension and CDP modes [5], Chrome DevTools MCP's auto-connect [6] and
browser-use's real-profile mode [7] all do it. wubble's difference has to be **the safety model and the co-working
UX**: the wall between sessions and the user's other Spaces, secrets that never reach the model, prompts a page cannot
draw or spoof, and a human who can watch, pause and take over.

| For | Against |
|---|---|
| The user's own logins and vault, scoped per session: what Playwright's fresh profile lacks | Prompt injection is unsolved: Anthropic reports 1.4% to 2.4% attack success for its best browser agents with safeguards [9]; OpenAI says it may never be fully solved [11] |
| Any harness: one MCP server instead of a per-vendor extension | Site terms often forbid automated access; accounts can be banned, and the user carries that risk |
| Chromium layout and controls agents already know | Bot detection and CAPTCHAs: a real profile helps; CAPTCHAs are a human's job (`request_human`) |
| Native prompts, a container wall and named secrets, which an extension cannot give | Liability: an agent that pays, applies or deletes in the user's name; the prompts must make each such step the user's |
| Co-working: watch live, pause, take over, in the same window | A new local attack surface (socket, token, bridge, a sheet any local process can pop) |
| Reuses Spaces, containers, the ax mirror, the pointer path, the vault's origin rule, the prompt kit | Maintenance: MCP changed its core shape in 2026-07-28 (stateless, no `initialize`) [2] and will move again |

## 2. Threat model

| Actor | What it can do | In scope? |
|---|---|---|
| Malicious page (or ad, comment, mail shown in a page) | Put instructions in text, the ax tree, images or screenshots; steer clicks; overlay elements; ask for a secret on a look-alike site; echo a filled secret into visible text; fire requests, popups, downloads, clipboard writes on the agent's gesture | **Yes, the main threat.** Brave showed hidden text [12] and near-invisible text in screenshots [13] steering agentic browsers |
| Confused agent (honest model, wrong action) | Wrong button, early submit, off-site wandering, a secret in the wrong field | **Yes.** Same defences: scope, allowlist, airlocks, origin-bound secrets |
| Malicious or compromised MCP client | Any tool its token allows, at speed; auto-accept whatever it is asked; forward all it reads | **Yes, bounded by its session.** It never answers an airlock, never leaves its Space, never sees a secret's value, never gets more than the user granted |
| Another local process, other OS user | Connect to the socket, steal a token, pop setup sheets to phish an approval | **Other users: yes** (socket 0600, peer-credential check). **Same-user malware: no**, as for the vault (`wubble-vault` lib.rs [V]): it can read the profile and process memory anyway. Sheet spam: rate-limited, and the sheet names the calling program |
| Remote website reaching a local server | DNS rebinding against an HTTP listener [3] | **Designed out:** no HTTP listener |

Assets: cookies and sessions per container; page content (mail, bank pages, documents); vault secrets and profile
variables; files (uploads); the clipboard; money and irreversible actions.

Out of scope: a compromised renderer or browser process; XSS on an approved site (the site's own script can always
read what is typed into it); the harness's other tools (shell, web fetch, mail), which can leak whatever a session
returns: wubble limits what it returns; the model provider, which receives what the harness forwards (wubble phones
nothing home, the harness does: the setup sheet says so); external password managers (owner: out of scope).

**The lethal trifecta** [14] (private data, untrusted content, a way out) frames every choice. A browser agent has all
three. wubble cannot remove untrusted content, so it shrinks the other two: the container and the secret grants
decide what private data a session can touch, the allowlist and airlocks decide where it can go.

## 3. Architecture in wubble's terms

### Crates and ownership

| Piece | Crate | Owns |
|---|---|---|
| `agent`: sessions, scopes, grants, secret and variable bindings, pending airlocks; one `enum` per session plus `fn transition(state, event) -> (state, effects)` | `wubble-core` (pure, unit-tested) | The session table: the one owner of what each session may do |
| `agent::policy`: tool table (name, scope, annotations), airlock table, allowlist matcher, as data | `wubble-core` | The allowlist reuses `routing.rs`'s `ContentSettingsPattern` grammar [V]; secret fills reuse `autofill::may_fill` [V] |
| `wubble-mcp`: JSON-RPC framing, `server/discover`, `tools/list`, `tools/call`, schemas, errors | new crate, core only | Protocol only: calls become `agent::Event`s, effect results become tool results. No CEF |
| Effects: snapshot, click at a point, type, navigate, hold or release a request, answer a dialog or file chooser, fill a secret | `wubble-engine` and the `crates/wubble` host | Executes; decides nothing |
| Secret resolution | `wubble-vault` on the store thread | The only place a value exists; it goes straight to the page, never to `wubble-mcp` |
| Setup sheet, prompts, indicator, Sessions page | `wubble-ui` kit components only | Drawn from the kit; shell files lay them out |
| Bridge, `wubble sessions` | `wubble` binary subcommands | Relays bytes; holds no state but its token |

Session lifecycle (core machine, sketch):

    Requested --sheet approved--> Active(scope, grants) --human input / pause--> Paused --resume--> Active
    Active --airlock--> Waiting(airlock) --allow / deny--> Active
    Active --request_human--> HandedOver --Done--> Active
    Active --asks for more (site, secret, scope)--> Requested(delta) --approved / refused--> Active
    any --kill switch / stop / revoke / Space deleted / expiry / bridge gone past grace--> Ended

### The workflow (owner) and transport

1. The harness runs `wubble mcp` (stdio). The bridge connects to `<profile>/mcp.sock` (a named pipe on Windows); if
   wubble is not running it starts it [owner: the window pops up], which is safe because nothing is granted until the
   sheet is approved.
2. The agent calls `start_session` with a purpose and, optionally, the sites and the secret and variable **names** it
   expects to need. wubble opens the setup sheet (section 6), naming the calling program from the socket's peer
   credentials (pid, executable path) [I] and the client's self-reported `clientInfo` [2], marked as unverified.
3. The user approves, edits or refuses. On approval wubble mints a 256-bit token, stored hashed, held by that bridge
   process (and printable for reuse); every later request on that bridge carries it.
4. The user closes the sheet (headless) or opens the session's Space to watch.

Why this shape:

- **stdio for the harness, IPC for the browser.** Clients SHOULD support stdio; a local server should use stdio or a
  token-gated or restricted IPC channel [3][4]. `crates/wubble/src/instance.rs` already binds a profile socket with
  `interprocess` (0BSD OR Apache-2.0) and handles the path-length limit [V]; the MCP socket copies that code but is a
  **second socket** (0600, peer checked), since the instance socket accepts unauthenticated "open this URL".
- **The token carries the scope, not the connection.** 2026-07-28 removed `initialize` and `Mcp-Session-Id`;
  `tools/list` "MUST NOT vary per-connection" but "MAY vary by the authorization presented" [2][4]. Harnesses cache the
  list when they start the server, before any approval. So the **configured** server entry fixes the advertised list
  (`wubble mcp --actor` or `--observer`), the sheet can only grant that scope or less, and every `tools/call` is checked
  against the live grant ("not granted in this session"). Cross-call state is explicit handles (`session`, `tab`) as
  ordinary arguments, as the spec recommends [4].
- **`start_session` blocks** until the sheet is answered, sending progress; under the stateless spec a broken stream
  loses the call [2], so it returns a handle on timeout and `session_status(handle)` polls. The Tasks extension can
  replace this later [2].
- **Library.** `rmcp` 3.11.0 (Apache-2.0, GPL-3.0-compatible) implements 2026-07-28, stdio and any
  `AsyncRead`/`AsyncWrite`, on tokio [8]; wubble has tokio only transitively [V Cargo.lock]. Recommendation: write
  the small JSON-RPC surface (about 600 lines) in `wubble-mcp` on the host's threads and channels, and test it against
  `rmcp`'s client as a dev-dependency (open question 2).
- **The remote debugging port voids all of this.** `devtools.remote_port` (off by default [V decisions.md]) hands any
  local process CDP over every container. Sessions cannot start while it is on.

### Windowed and headless

wubble has no headless engine mode, and needs none: CEF already renders windowless into textures [V AGENTS.md].
"Headless" means the session's Space has **no window showing it**; the views, container and policy are the same. The
user watches later by opening wubble (a second start forwards `Focus` on the instance socket [V instance.rs]) and
switching to the session's Space, or from the Sessions page. Open issues: hidden views stop painting, so screenshots
of an unshown tab need it painted on demand (CDP `Page.captureScreenshot`, or visible-but-unmapped) [?]; sleep and
freezing (`sleep.rs`, `lifecycle.rs`) must not take a tab a session drives. The **kill switch when headless**: wubble
has no tray [V: no tray code]; offered are `wubble sessions` (list, `stop <id>`, `stop --all`, through the socket), the
Sessions page and command-bar row once the window is open, and an OS notification "An agent session started" with a
Stop action [I]. The agent's own `end_session` is cooperative only, never the safety stop.

### Binding to a Space and container

A session gets a **new Space with a fresh, empty container** by default, where the user signs in themselves (each
Space has a default container; containers share nothing [V decisions.md]). An existing Space is allowed with a
warning naming what is signed in there. The session sees only its Space's tabs; tabs it opens land there; link routing
never moves an agent tab out or a user tab in; "Move to Space" ends the agent's hold on a tab. Whether the container is
kept after the session (logins stay for next time) or wiped is a sheet choice (open question 4).

### Element refs

Reads come from the accessibility mirror already serving screen readers: `View::set_accessibility`,
`wubble_ax::PageTree`, `PageTree::flatten` and `wubble_ax::reach` (centre point, or scroll first); clicks go through
the pointer path and text fields are typed into [V `crates/wubble/src/host/ax.rs`]. A ref is opaque, `e<doc>.<node>`:
the tab's document generation and Chromium's ax node id. A new document bumps the generation, so old refs fail with
"stale ref: take a new snapshot"; a removed node fails the same way. As in Playwright MCP, the snapshot is the main view
and refs its handles [5]. Before a click the engine checks that the topmost node at the point is the ref's node or
inside it (CDP `DOM.getNodeForLocation` through `View::cdp` [I]), refusing otherwise: this stops transparent overlays
stealing agent clicks. Frame offsets and live ax ids across frames still need a first real dump (wubble-ax header [V]).

### Concurrency with the human

The human always wins: any human pointer or key input on an agent tab pauses its session (the next call gets "paused
by the user"); the indicator offers Resume. Agent input uses the same CEF input path, so to Chromium it is a **trusted
gesture**, which unlocks popups, clipboard writes, file choosers and downloads [I]. The engine therefore tags every
gesture it injects with the session; `popup.rs` and the airlocks treat an agent gesture as the agent's.

## 4. Tool surface

Annotations are hints for the harness's own permission UI, untrusted by spec [4]; wubble enforces nothing through
them. Page tools take `session` and `tab` handles. 2026-07-28 requires `ttlMs`/`cacheScope` on list results [2].

### Always listed

| Tool | Input | Output | Annotations |
|---|---|---|---|
| `start_session` | purpose; optional sites, secret and variable names wanted | session handle once approved, or "pending" | not read-only (opens UI) |
| `session_status` | handle | pending / active / paused / ended, granted scope, sites, names | readOnly |
| `request_more` | handle; sites, names or scope to add | re-opens the sheet for the delta | not read-only |
| `end_session` | handle | ended | destructive |

### Observer (Actor has these too)

| Tool | Input | Output | Annotations | Never |
|---|---|---|---|---|
| `list_tabs` | none | tab handles, title, origin, loading, which tab the human has focused | readOnly, idempotent | Other Spaces; history |
| `read_ax_tree` | `tab`, optional `ref`, `max_items` | flat items (role, name, value, state, ref) with flags (section 5) | readOnly, idempotent, openWorld | Values of password fields or fields filled from secrets; frames outside the list |
| `read_text` | `tab`, optional `ref` | readable text in order, capped, flagged | readOnly, idempotent, openWorld | As above |
| `read_article` | `tab` | the reader built-in's extraction (title, byline, text) [V decisions.md "Built-ins"] | readOnly, idempotent, openWorld | As above |
| `screenshot` | `tab`, optional `ref` crop | PNG, capped | readOnly, openWorld | Secret-filled fields unmasked (section 5); other tabs; the shell |
| `list_variables` | none | granted names, kind, the origins each is bound to; values only of plain variables marked visible | readOnly, idempotent | Any secret's value |
| `list_downloads` | none | this session's downloads: name, size, origin, state | readOnly | Contents |
| `blocker_status` | `tab` | whether the content blocker is on for the site, blocked count | readOnly | Toggling it |

### Actor adds (Playwright-level, by ref)

| Tool | Input | Annotations | Never |
|---|---|---|---|
| `click`, `hover` | `tab`, `ref`; button, count, modifiers | destructive, openWorld | Coordinates without a ref; a node not topmost at its point |
| `drag` | `tab`, from `ref`, to `ref` | destructive, openWorld | |
| `fill` | `tab`, `ref`, one of `{text}`, `{secret: NAME}`, `{var: NAME}` | destructive, openWorld | A secret outside its origin or into the wrong field kind (section 5) |
| `fill_form` | `tab`, list of (ref, value) as above | destructive, openWorld | As `fill` |
| `select` | `tab`, `ref`, option label or value | destructive, openWorld | |
| `press_key` | `tab`, named key or short text | destructive | Shell shortcuts: keys go to the page, never to wubble's key routing |
| `scroll` | `tab`, `ref` or direction and amount | none | |
| `navigate` | `tab`, `url` or back / forward / reload | destructive, openWorld | `javascript:`, `file:`, `data:`, internal pages; outside the list without an airlock |
| `open_tab`, `close_tab` | url / tab | destructive | Closing a tab the session did not open |
| `wait_for` | `tab`; text, ref, load or network idle; timeout | readOnly | |
| `handle_dialog` | `tab`; accept / dismiss, prompt text | destructive | Answer a wubble prompt or airlock; leave-page dialogs answer only with the human present [I] |
| `upload` | `tab`, `ref` of a file input; optional `{var: RESUME}` | destructive, openWorld | Name a path: a file variable was approved in the sheet, or the human picks in the native chooser |
| `download_media` | `tab`, `ref` of a video | destructive | Bypass the download airlock (media downloader built-in [V decisions.md]) |
| `request_human` | reason, optional `tab` | none | Ask for passwords, card numbers or codes |

`request_human` is a **wubble-native hand-over**, not MCP elicitation: the tab glows, a bar shows the agent's reason,
the human does the step (a CAPTCHA, an SMS code, a passkey, a judgement) and presses Done. Elicitation (an
`input_required` result the client answers on retry [2]) can carry a plain question shown **inside the harness**; never
an approval, since the client answering it is the party the airlocks distrust, and form mode MUST NOT ask for passwords
or payment data anyway [10].

### What "Playwright-level" leaves out, and a developer session

Playwright MCP's core list [5] is covered above except: `browser_evaluate` and `browser_run_code_unsafe` (its docs
call the latter RCE-equivalent), cookie and storage tools, `browser_route` interception, network request listing,
console messages, media emulation, window resize and coordinate mouse tools (its opt-in "vision" group). wubble
withholds all of them from sessions: each reads or changes state the container wall and redaction exist to protect,
or lets a page's text become code. Built-ins that inject script (Boosts, dark mode [V decisions.md]) are withheld for
the same reason. A **developer session** (eval, console, network list) is worth having later, but only in an
**ephemeral empty container** that can hold no secrets or variables, is wiped at the end, blocks private and loopback
addresses (the SSRF ranges the MCP security page lists [3]), and wears a different indicator: then it is no riskier
than Playwright's own fresh profile. Recommendation: after phase 3 (open question 7).

## 5. Policy

### The owner's points, refined

1. **Observer and Actor at the protocol level**, enforced on every call. **Read-only is not safe by itself**: an
   Observer in a logged-in Space reads mail and bank pages and the harness can send them anywhere. Point 2 is what
   makes an Observer safe.
2. **The container wall**: one session, one Space, one container, fresh by default. It does not depend on the model
   behaving, so it is built first.
3. **The allowlist is mandatory for Actors**: a click, fill or navigation can carry data out in a URL.
4. **Airlocks** at deterministic boundaries (below), drawn from the kit (`a-prompt.slint`'s family [V]), outside the
   page's pixels, never focused (as the permission prompt: no keyjacking [V decisions.md]).
5. **Hidden-content flags are a signal, not a defence**: items with `opacity:0`, font-size 0, off-screen, clipped
   away, colour on its own background, or `aria-hidden` but visible. The ax tree already drops `display:none` and
   `visibility:hidden` [I]; the rest needs computed style read in an isolated world through `View::cdp` [I]. Brave's
   screenshot attack used faint text in pixels [13], which no DOM flag sees.

### Secrets and variables (owner)

What the vault holds today: **logins only** (origin, form-action origin, username, password; AES-256-GCM, key in the OS
keyring) [V `wubble-vault` record.rs, lib.rs]. No TOTP, passkeys or cards: the autofill parser only recognises OTP
fields so it does not mistake them for passwords [V autofill.rs]; passkeys are future work
([apple-passwords.md](apple-passwords.md)). External managers (Bitwarden and others) are out of scope (owner;
Chromium extensions are off until M6 [V decisions.md]).

- **Names, not values.** The sheet lists vault logins; each one ticked becomes two names, e.g. `GITHUB_USER` and
  `GITHUB_PASSWORD`, bound to that login's exact origin. The agent sees names and origins only (`list_variables`).
- **Resolved at the moment of typing**, in the host: `fill(ref, {secret})` sends the name; the host asks the vault on
  the store thread and types the value into the field through the same path as autofill. The value never reaches
  `wubble-core::agent`, `wubble-mcp`, the activity log or any tool result.
- **Origin binding** is `autofill::may_fill`: the field's frame origin must equal the top-level origin and both must
  equal the login's exact origin (https never fills on http, a subdomain never matches) [V]. Also checked: the form's
  action origin against the record's `form_action_origin` when present [V record.rs] [I as a rule]; the field kind (a
  password only into a password input or `current-password`/`new-password`, a username only into a text or email
  input); one fill per approved login per document unless the human allows more. A look-alike site fails the origin
  check whatever the agent or page asks.
- **Redaction.** Chromium draws password inputs as bullets, so screenshots of an untouched password field are masked,
  and its ax tree does not expose a password field's value [I: check in the first live ax dump]. That is not enough:
  a "show password" toggle turns the field into plain text, and a page can echo the value elsewhere. So: (a) the host
  records each fill (tab, document, field ref, the value's hash); reads redact the field's value whatever its type now
  is, and redact **any occurrence of the value's exact string** in ax names, values and text before they leave the
  browser, flagging "this page copied a secret into visible text"; (b) **screenshots of a document a secret was filled
  into are refused** until it navigates, since pixels cannot be checked for an echo; a login page usually navigates on
  submit, so the cost is small (open question 5); (c) fields filled from secrets cannot be read back with `read_*`.
- **Profile variables** (owner): name, email, phone, address, a resume file. Stored in a new profile table, picked in
  the sheet like secrets and resolved the same way. Plain ones (name, email) may be marked **visible to the agent**
  (it needs them to write a cover letter); sensitive ones (address, phone, ID numbers) stay names only. A file
  variable is a path the user picked in the sheet; `upload(ref, {var})` uses it without a chooser, only on approved
  origins, with the upload airlock once per origin.
- **Codes and passkeys today**: SMS, mail and TOTP codes and WebAuthn prompts go to the human (`request_human`).
  Future: TOTP seeds in the vault as a secret kind, filled into `one-time-code` fields under the same origin rule.

### Airlocks

| Boundary | How the engine detects it | The prompt shows | Default | Pre-approve? |
|---|---|---|---|---|
| File upload | `on_file_dialog` (welding `DialogHandler`) [V] | origin, field label, file | the human picks; a file variable asks once per origin | File variables only |
| Download, media download | `can_download`/`on_before_download` [V]; the media downloader's job | origin, name, size, type | asks | Per origin |
| Clipboard write | Wayland: the helper's `clipboard.js` bridge reports every page copy [V]; X11, macOS, Windows: Chromium writes itself [V clipboard.rs] | text (truncated), origin | asks | Yes |
| Navigation outside the list | `on_before_browse` and the resource filter on main-frame requests, redirects included [V handlers.rs] | from, to, the link's name | blocks; asks if allowed | Add the site |
| Popup / new window | `popup.rs` with the agent-gesture tag | target origin | as navigation | As navigation |
| Secret fill outside its binding | the origin and kind checks above | refusal, no prompt | refused | No |
| Payment Request | `PaymentRequest.show()` [?: whether CEF 156 offers it is unverified] | merchant, total | always asks, or off in agent containers | No |
| Permission prompts | `on_show_permission_prompt` [V] | as today, labelled "during an agent session" | the human answers | No |
| Leave-page dialog, JS prompt | welding `OnJSDialog`/`OnBeforeUnloadDialog` [V] | the page's text | agent may answer alerts and confirms | n/a |
| Non-GET after an agent action (phase 3) | resource request handler, below | method, URL, body summary | per write mode | Per origin |

Clipboard off Wayland is not gateable until either the bridge is forced on in agent containers' renderers (with
Chromium's own writes suppressed) or `clipboard-write` is denied there [I] (open question 6).

### "This click submits a form" is not detectable

Any click can run script that sends a request now or later, by `fetch`, `sendBeacon`, a form, a WebSocket or a
service worker. The real mechanisms and their cost:

| Mechanism | Catches | False positives | False negatives |
|---|---|---|---|
| **Write window**: hold every non-GET from the tab's frames for N seconds after each agent action and ask (CEF's `on_before_resource_load` can answer later through its callback [V welding signature]) | Form posts, fetch/XHR writes, beacons the action fires | Analytics beacons (the blocker cancels many), POST-for-reads (GraphQL, search), autosave | GET with side effects, late requests, WebSocket frames, service-worker requests (`browser_id` 0 [V surface.rs]) |
| **Per-origin grants**: writes to `jobs.example.com` allowed, asked once | Fewer prompts for a known task | n/a | Everything an allowed origin does |
| **Review writes**: every non-GET held for the session's life | All request-shaped writes | Many | As above minus timing |
| **Intent prompt** before clicking a node named like submit, pay, buy, delete, send, apply | Obvious buttons, before the fact | Wording is a guess | Unlabelled and icon buttons |

Recommended: write window plus per-origin grants by default, the intent prompt as UX on top, review writes as the
"high stakes" choice in the sheet. Engine changes: `ResourceRequest` carries no method or body today [V surface.rs]
(a welding change); service workers off in agent containers (as Playwright's `--block-service-workers` [5]) so
requests stay attributable; WebSocket and WebRTC coverage by the handler unverified [?], so WebRTC's non-proxied UDP is
off in agent containers [I]. Honest limit, for the allowlist too: an allowed site with user content (a comment box, a
job form) is itself a way out. The airlock shows the payload; it cannot know it is private.

### Allowlist semantics

Patterns in `routing.rs`'s grammar, matched on the top-level origin of every main-frame request. Redirects: each hop
checked [I], unlike Playwright's list, which its docs say does not cover redirects [5]. Subframes load but are read
only when their origin is listed, and secrets never fill into them (`may_fill` requires frame origin = top origin
[V]). Subresources: not checked by default (sites break); "strict" checks them. Link clicks and popups count as
navigation. The sites the agent asked for are pre-filled in the sheet, unticked.

### Reads, logging, audit

Size caps per call (text and tree in characters, screenshots in pixels and bytes) with continuation by ref;
cross-origin frames outside the list summarised as "frame from <origin>"; nothing from the shell. Every call, its
arguments (secret names, never values), its outcome and every airlock answer go to a per-session activity log in the
profile store, shown on the Sessions page; retention as decided (open question 9); nothing leaves the machine.

## 6. UX

- **Enable**: Settings, a Sessions section, off: the switch, the warning (what an agent sees; that the harness sends
  it to its model provider; site terms), the remote-debugging conflict.
- **Setup sheet** (kit `sheet.slint`, `check-box`, `toggle`, `segmented`, `select`, `list-editor` [V]): who is asking
  (program, client name, purpose, the unverified mark); scope; Space (new by default) and whether its container is
  kept; allowed sites; secrets ticked from the vault, each with its origin; profile variables, each with "visible to
  the agent"; write mode; windowed or headless; expiry. Approve, Refuse.
- **Lifetime and re-approval**: ends on expiry, idle timeout, `end_session`, stop, or the bridge gone past a short
  grace. Re-approval is needed for a new client, more scope, a new site, secret or variable (`request_more` opens the
  sheet on the delta only), and after expiry.
- **Indicator**: the design's border beam (`beam.rs`, the kit's `BorderBeam` [V]) round an agent tab's page card in
  the session's colour, a mark on its tab row, "Agent: <client>" on the Space header. A steady "driving" state needs
  the owner.
- **Approvals**: the airlock prompt under the address pill, never focused: Allow once, Allow for this session (where
  allowed), Deny. Headless, an airlock raises a notification and the session waits; it does not open the window by
  itself [I] (open question 8).
- **Take-over**: any human input pauses; a bar offers Resume and End. `request_human` uses the same bar with the reason
  and Done.
- **Kill switch**: "Stop all agents" in the command bar and as a shortcut, `wubble sessions stop --all`, and the
  notification's Stop.
- **Sessions page**: an `InternalPage` like Downloads [V pages.rs]: sessions, grants, calls, airlocks, revoke.

Labels are drafts; wording is wubble's. Nothing here has a web design yet. For
[docs/fidelity/derived-review.md](../fidelity/derived-review.md): the setup sheet, the session indicator (colour,
steady state, tab mark), the airlock prompt variant (payload block, origin emphasis), the take-over bar, the Sessions
settings section, the Sessions page, the notification text. The indicator's steady glow may need a new primitive and
waits for the owner.

## 7. Phases

| Phase | Scope | Exit criteria and tests | Size [I] | Reuses |
|---|---|---|---|---|
| 0. Threat model | This plan reviewed, section 8 answered, tool and airlock tables frozen as data | Decisions in `docs/decisions.md` | 1 to 2 days | |
| 1. Observer and sessions | `agent` machine and policy; `wubble-mcp`; socket, peer check, token; bridge that starts wubble; `start_session` and the setup sheet (sites, scope, headless); read tools, `read_article`, redaction; indicator; `wubble sessions` and the kill switch | Core: every state x event; allowlist matcher. MCP: golden JSON for discover, list, call, errors, an `rmcp` client test. Hand-run `--smoke mcp`: a session starts, reads, screenshots, cannot see another Space | 3 weeks | instance socket, ax mirror and `flatten`, screenshot worker, routing matcher, kit sheet and prompt |
| 2. Actor, secrets, airlocks | the Actor tools; hit-test check; gesture tagging; allowlist on navigation and popups; secret and variable fills with origin and kind checks, string-match redaction, screenshot refusal; airlocks for upload, download, navigation, permissions, dialogs (clipboard on Wayland) | Core: fill decisions (origin, kind, frame), airlock decisions, stale refs. Smoke: a bundled login and form page; a secret fills on its origin and is refused on a look-alike; human input pauses | 4 weeks | screen-reader click and field paths, `may_fill`, vault, file chooser, download machine, permission prompt |
| 3. Write gating, flags, audit | method and body in `ResourceRequest` (welding), held requests, write window, grants, review mode, service workers and WebRTC off in agent containers; hidden-content flags; Sessions page; profile variables store | Core: write-window machine on a fake clock (states, never timings); smoke: a POST after a click is held | 3 weeks | blocker filter, store |
| 4. Docs, harness configs, developer session | user docs and configs for common harnesses; ephemeral developer sessions if chosen | A harness on each OS completes a scripted task by hand | 1 to 2 weeks | |

No test asserts timings (AGENTS.md). TOTP and passkeys follow the vault's own roadmap.

## 8. Open questions for the owner

1. **Name in the UI?** (a) "Sessions", (b) "Agent sessions", (c) "Agents". Recommend **(b)**: "session" alone collides
   with session restore (`wubble-core::session` [V]) and with a site's login session.
2. **Protocol library?** (a) `rmcp`, (b) our own small surface tested against `rmcp`. Recommend **(b)**.
3. **Default Space?** (a) always new, (b) new by default, an existing one with a warning, (c) the current one.
   Recommend **(b)**.
4. **Keep the session's container afterwards?** (a) always wipe, (b) the sheet asks, default keep, (c) always keep.
   Recommend **(b)**: logins made for an agent are worth keeping, but the user decides.
5. **Screenshots after a secret fill?** (a) refused until navigation, (b) allowed with fields masked and a flag, (c)
   allowed. Recommend **(a)**.
6. **Clipboard off Wayland?** (a) force the bridge in agent containers, (b) deny clipboard writes there, (c) leave
   ungated and say so. Recommend **(b)** first.
7. **Developer session (eval, console, network) in an ephemeral empty container?** (a) never, (b) after phase 3, (c)
   in phase 2. Recommend **(b)**.
8. **Airlock while headless?** (a) notification, the session waits, (b) the window opens on the prompt, (c) refuse at
   once. Recommend **(a)**.
9. **Activity log retention?** (a) 7 days, (b) 30 days, (c) until the session is deleted. Recommend **(b)**.
10. **Default write mode?** (a) deterministic airlocks only, (b) write window plus grants, (c) review writes.
    Recommend **(b)**.
11. **Plain variables visible to the agent?** (a) never, (b) per variable in the sheet, name and email ticked, (c)
    always. Recommend **(b)**.
12. **Lend one tab of the user's own Space to an Observer?** (a) never, (b) one tab until the user switches away, (c)
    any Space. Recommend **(a)** for now: (b) breaks the wall and needs its own review.
13. **Bot detection and site terms?** (a) nothing, (b) a line in the sheet, (c) per-site warnings. Recommend **(b)**.

## Where this plan differs from the positions it started from

- **Stateless MCP moves the binding to the token**, and because harnesses cache the tool list before approval, the
  configured entry (`--observer`/`--actor`) fixes the list and the sheet grants that or less [2][4].
- **`request_human` is not elicitation**: elicitation is answered by the distrusted client.
- **Clipboard writes are only detectable on Wayland** today.
- **Agent clicks are trusted gestures**: popups, downloads and clipboard writes must be attributed to the session.
- **The remote debugging port voids the wall**: no sessions while it is on.
- **Service workers and WebSockets escape request attribution**: off in agent containers, or write gating is weaker.
- **Bullets are not redaction**: show-password toggles and echoes need string-match redaction and no screenshots of a
  document holding a secret.
- **Owner direction folded in**: named secrets and variables, harness-started sessions with a native setup sheet,
  windowed or headless, Playwright-level control by ref. **Kept**: container wall first, mandatory Actor allowlist,
  fresh container, deterministic airlocks only, flags as signals, socket plus stdio bridge, build after release.

## Sources

1. MCP specification, latest (2026-07-28): https://modelcontextprotocol.io/specification/latest [V]
2. MCP 2026-07-28 changelog (stateless, `server/discover`, multi round-trip requests, roots deprecated, Tasks extension, no stream resumption): https://modelcontextprotocol.io/specification/2026-07-28/changelog [V]
3. MCP transports (Origin validation, DNS rebinding, bind to localhost): https://modelcontextprotocol.io/specification/2025-06-18/basic/transports [V]; security best practices (local server compromise, stdio or restricted IPC, SSRF ranges): https://modelcontextprotocol.io/docs/2026-07-28/tutorials/security/security_best_practices [V]
4. MCP tools (list varies by authorization, not connection; annotations untrusted; state handles): https://modelcontextprotocol.io/specification/2026-07-28/server/tools [V]. Annotation defaults (readOnlyHint false, destructiveHint true, idempotentHint false, openWorldHint true) not re-read in the 2026-07-28 schema [I]
5. Playwright MCP (tools, snapshot refs, `--extension`, `--cdp-endpoint`, origins "not a security boundary" and not applied to redirects, `--block-service-workers`, `browser_run_code_unsafe`, opt-in groups): https://github.com/microsoft/playwright-mcp [V]
6. Chrome DevTools MCP and auto-connect (Allow dialog per connection, banner, Chrome 144+; remote debugging on the default profile blocked since Chrome 136): https://github.com/ChromeDevTools/chrome-devtools-mcp, https://developer.chrome.com/docs/devtools/agents/use-cases/auto-connect [V via search summary]
7. browser-use (MIT, raw CDP, real profile, unauthenticated local debugging port, `allowed_domains` "not a sandbox", `sensitive_data` placeholders that model-written code can still read): https://github.com/browser-use/browser-use [V]
8. Official Rust SDK `rmcp` 3.11.0, Apache-2.0: https://github.com/modelcontextprotocol/rust-sdk, https://crates.io/crates/rmcp [V]
9. Anthropic, prompt injection defenses for browser use (Nov 2025): https://www.anthropic.com/research/prompt-injection-defenses [V]; Claude in Chrome (site permissions, confirmations for publishing, purchasing, sharing personal data, blocked categories, 23.6% to 11.2%): https://claude.com/blog/claude-for-chrome [V]
10. MCP elicitation (form and URL modes, no secrets in form mode): https://modelcontextprotocol.io/specification/2026-07-28/client/elicitation [V]
11. OpenAI on Atlas and prompt injection (Dec 2025): https://techcrunch.com/2025/12/22/openai-says-ai-browsers-may-always-be-vulnerable-to-prompt-injection-attacks/ [V secondary; OpenAI's own post returned 403]; https://simonwillison.net/2025/Oct/22/openai-ciso-on-atlas/
12. Brave, indirect prompt injection in Perplexity Comet (Aug 2025): https://brave.com/blog/comet-prompt-injection/ [V via search summary]
13. Brave, unseeable prompt injections (Oct 2025; faint text in screenshots; Fellou navigation): https://brave.com/blog/unseeable-prompt-injections/ [V]
14. Simon Willison, the lethal trifecta: https://simonwillison.net/2025/Jun/16/the-lethal-trifecta/ [V]. CaMeL, "Defeating Prompt Injections by Design" (control and data flow from the trusted query; capabilities on values): https://arxiv.org/abs/2503.18813 [V abstract]. wubble does not run the planner, so CaMeL is a model for the harness; what wubble copies is its rule that data from untrusted content never authorises an action by itself, which named secrets and airlocks enforce
