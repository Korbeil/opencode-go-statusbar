# opencode-go-statusbar

A [COSMIC desktop](https://github.com/pop-os/cosmic-epoch) panel applet that shows the
remaining usage quota of one or more **OpenCode Go** or **Claude** accounts.

## Features

- **Panel button**: the applet icon followed by one label per configured account
  - the label shows the **worst remaining %** across the account's windows
    (5-hour and weekly, plus monthly on OpenCode Go) — the true "time until I'm
    blocked"
  - turns **orange** when fewer than 20% remain, and shows a red **`blocked`**
    label as soon as any window is exhausted or rate-limited
- **Popup**: one section per account with the quota bars (**5 hours / Weekly /
  Monthly** on OpenCode Go; **5 hours / Weekly** on Claude), showing remaining %,
  reset countdown, `rate-limited` badge, and per-account error reporting
  (invalid key, missing subscription, missing OAuth token, network failure)
- **Providers**: each account is either an **OpenCode Go** subscription or a
  **Claude** subscription (Pro/Max), chosen per account in the settings
- **Settings** (footer button in the popup): add / remove accounts (name +
  credentials + provider) and pick a refresh interval (30 s – 10 min)
- Configuration is persisted with `cosmic-config`
  (`~/.config/dev.korbeil.opencode-go-statusbar/`) and hot-reloaded when the file
  changes on disk

## Screenshots

The applet button in the panel (one label per account, `|`-separated) with the
quota popup open — bars show the used quota, numbers what remains:

![Applet button and quota popup](assets/panel-popup.png)

The settings popup (accounts and refresh interval):

![Settings popup](assets/settings.png)

## The OpenCode Go API

The applet queries OpenCode's first-party (but undocumented) usage endpoint:

```
GET https://opencode.ai/zen/go/v1/usage
Authorization: Bearer <api key>
```

which returns, per window, the percent of the budget already used, a
`ok | rate-limited` status, and an ISO-8601 `resetsAt` timestamp:

```json
{ "usage": {
    "rolling":  { "status": "ok", "percent": 0,   "resetsAt": "2026-09-03T18:41:15Z" },
    "weekly":   { "status": "ok", "percent": 64,  "resetsAt": "2026-09-07T00:00:00Z" },
    "monthly":  { "status": "ok", "percent": 12,  "resetsAt": "2026-09-28T12:41:54Z" }
} }
```

Go plan limits: **$12 per 5 hours, $30 per week, $60 per month**.
An API key comes with an [OpenCode Go](https://opencode.ai/docs/go/) subscription —
sign in at [opencode.ai/auth](https://opencode.ai/auth) and copy the key.
The key is stored locally in the applet's config file (plaintext, user-scoped,
standard for COSMIC applet configs).

## Providers

Each account targets one of two subscription kinds, selected in the settings
with a segmented control:

**OpenCode Go** (default) uses the API documented above.

**Claude** queries the same (undocumented) endpoint that powers `/usage` in
Claude Code:

```
GET https://api.anthropic.com/api/oauth/usage
Authorization: Bearer <oauth access token>
anthropic-beta: oauth-2025-04-20
```

which returns a `five_hour` and a `seven_day` window (there is **no monthly
window**), each with a 0–100 `utilization` percent and an ISO-8601 `resets_at`:

```json
{ "five_hour": { "utilization": 37.0, "resets_at": "2026-03-10T04:59:59.000000+00:00" },
  "seven_day": { "utilization": 26.0, "resets_at": "2026-03-15T14:59:59.771647+00:00" } }
```

The endpoint requires a subscription OAuth token (Claude Pro/Max), not an
Anthropic API key. Leave the token field **empty** and the applet reads the
live token fresh on every refresh from Claude Code's credentials file —
`~/.claude/.credentials.json`, or `$CLAUDE_CONFIG_DIR/.credentials.json` — so
it never goes stale as long as you stay logged in to Claude Code. A token
pasted into the field overrides the file, but OAuth access tokens expire
within hours (Claude Code rotates them itself), so pasting is only useful for
edge cases; an expired token shows a `no usable Claude OAuth token` error
until you run `claude login` or clear the field. The endpoint may change
without notice as it is undocumented.

## Building

Requires Rust ≥ 1.85 (edition 2024) and the usual COSMIC build dependencies
(`build-essential`, `libxkbcommon-dev`, `libwayland-dev`, `cmake` for rustls).

```sh
just            # build-release (default)
just check      # clippy with pedantic lints
just test       # unit tests (no network needed)
just run        # run the applet standalone for testing
```

## Installing

System-wide:

```sh
sudo just install
```

or into your home directory (no root needed):

```sh
just install-user
```

Then add the applet to the panel: **COSMIC Settings → Panel → Applets → Add →
"OpenCode Go Statusbar"** (a re-login may be needed for the panel to discover a
freshly installed applet).

Uninstall with `sudo just uninstall` / `just uninstall-user`.

## Usage

1. Click the applet icon in the panel to open the popup.
2. Click **Settings**, then **Add account**.
3. Enter a display name, pick the provider (**OpenCode Go** or **Claude**),
   and enter the matching credentials — an API key for OpenCode Go, or leave
   the token empty for Claude to reuse Claude Code's login.
4. Quotas refresh on the configured interval, when the popup opens, and via the
   **Refresh** button.

## License

MPL-2.0
