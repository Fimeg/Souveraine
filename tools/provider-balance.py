#!/usr/bin/env python3
"""Probe every configured inference provider and report which agent roles are
about to fail.

Why this exists
---------------
Between 2026-08-12 and 2026-08-13 three providers ran out of credit and in
every case the first thing that told us was a failed subconscious pass:

  * glm-5.2 (zai)      -- 429 code 1113 "insufficient balance"
  * deepseek           -- 402 "Insufficient Balance", total_balance -0.04
  * opencode zen       -- 401 CreditsError

Note the codes disagree completely: 429, 402, 401, all for "you are out of
money".  Classifying on status code cannot work; we classify on wording.

Balance exhaustion is invisible until something dies.  Absence of complaint
reads as health.  This is the instrument that speaks first.

Method
------
It does NOT ask whether an endpoint is reachable.  Reachability is not an
answer -- GET /models on opencode returns 200 with a valid key and a bankrupt
workspace.  So the probe sends a real (tiny) completion and classifies the
reply.  Where a provider exposes a native balance endpoint we report the
actual number as well.

Exit status
-----------
  0  every provider backing a configured role can answer
  1  at least one role-backing provider cannot answer
  2  could not run (config unreadable)

Usage
-----
  provider-balance.py            # probe role-backing providers (default)
  provider-balance.py --all      # probe every configured provider
  provider-balance.py --json     # machine-readable
  provider-balance.py --selftest # prove the classifier can reject
"""
from __future__ import annotations

import argparse
import base64
import json
import subprocess
import sys
import time
import tomllib
from pathlib import Path

CONFIG = Path.home() / ".souveraine" / "config.toml"
TIMEOUT = 45

# Logins that live outside config.toml. They back no Souveraine role yet, so a
# dead one never fails the run -- but a refresh token quietly reaching its end
# is a browser trip, and that is worth seeing before the morning it bites.
EXTERNAL_LOGINS = {
    "chatgpt-oauth": Path.home() / ".codex" / "auth.json",
}

OK = "OK"
NO_BALANCE = "NO_BALANCE"
RATE_LIMITED = "RATE_LIMITED"
CAPPED = "CAPPED"
AUTH = "AUTH"
UNREACHABLE = "UNREACHABLE"
SKIPPED = "SKIPPED"

# Substrings that mean "your account is out of money", collected from the real
# error bodies of three different providers.  Matching on wording rather than
# status code because the codes disagree: zai says 429, deepseek 402,
# opencode 401 -- all for the same condition.
BROKE_MARKERS = (
    "insufficient balance",
    "creditserror",
    "out of credit",
    "quota exceeded",
    "billing",
    "payment required",
)

# A time-window cap is NOT bankruptcy.  OpenCode Go is a $10/month
# subscription with $12/5h, $30/week and $60/month ceilings; hitting one means
# "wait", not "pay".  These are checked BEFORE the broke markers because a cap
# message may well also mention billing, and misreporting a throttle as
# bankruptcy would send someone to a payment page for no reason.
#
# Ordering is safe against every real bankruptcy body we hold -- none of them
# contain any of the wording below.  --selftest asserts exactly that.
#
# VERIFIED wording: "FreeUsageLimitError" (opencode zen free tier, captured
# 2026-08-13).  UNVERIFIED: the Go 5h/weekly/monthly cap body -- no one has
# spent $12 in five hours yet, so the remaining markers are a best guess and
# are marked as such rather than presented as observed.
CAP_MARKERS = (
    "freeusagelimiterror",
    "usagelimiterror",
    "usage limit",
    "limit reached",
    "limit exceeded",
)

RATE_MARKERS = (
    "rate limit",
    "too many requests",
)


def curl_json(url: str, key: str, payload: dict | None = None,
              virtual_key: str = "") -> tuple[int, str]:
    """POST/GET via curl.

    curl rather than urllib on purpose: opencode sits behind Cloudflare, which
    answers Python-urllib's user-agent with 403 code 1010.  That failure looks
    exactly like a rejected key and is not one.
    """
    cmd = ["curl", "-sS", "--max-time", str(TIMEOUT), "-w", "\n%{http_code}",
           "-H", f"Authorization: Bearer {key}"]
    if virtual_key:
        cmd += ["-H", f"x-bf-vk: {virtual_key}"]
    if payload is not None:
        cmd += ["-H", "Content-Type: application/json", "-d", json.dumps(payload)]
    cmd.append(url)
    try:
        out = subprocess.run(cmd, capture_output=True, text=True, timeout=TIMEOUT + 10)
    except subprocess.TimeoutExpired:
        return 0, "timed out"
    body = out.stdout
    if "\n" in body:
        body, _, code = body.rpartition("\n")
    else:
        code = "0"
    try:
        return int(code or 0), body
    except ValueError:
        return 0, body


def classify(code: int, body: str) -> str:
    low = body.lower()
    if code == 0:
        return UNREACHABLE
    # Cap and rate wording first: both are recoverable-by-waiting, and a cap
    # message that also says "billing" must not be reported as bankruptcy.
    if any(m in low for m in CAP_MARKERS):
        return CAPPED
    if any(m in low for m in RATE_MARKERS):
        return RATE_LIMITED
    if any(m in low for m in BROKE_MARKERS):
        return NO_BALANCE
    if code == 200:
        return OK
    if code in (401, 403):
        return AUTH
    return UNREACHABLE


def native_balance(name: str, base_url: str, key: str) -> str | None:
    """Ask for an actual number where the provider exposes one."""
    if "api.deepseek.com" in base_url:
        code, body = curl_json("https://api.deepseek.com/user/balance", key)
        if code == 200:
            try:
                d = json.loads(body)
                info = (d.get("balance_infos") or [{}])[0]
                return f"{info.get('total_balance')} {info.get('currency', '')}".strip()
            except Exception:
                return None
    return None


def _jwt_exp(token: str) -> int | None:
    """Seconds-epoch `exp` from a JWT payload. No signature check — we are
    reading our own stored token to see when it dies, not trusting it."""
    try:
        part = token.split(".")[1]
        part += "=" * (-len(part) % 4)
        return json.loads(base64.urlsafe_b64decode(part)).get("exp")
    except Exception:
        return None


def _when(epoch_s: float | None) -> str:
    if not epoch_s:
        return "unknown"
    delta = epoch_s - time.time()
    if delta < 0:
        return f"expired {_dur(-delta)} ago"
    return f"{_dur(delta)} left"


def _dur(seconds: float) -> str:
    m = seconds / 60
    if m < 90:
        return f"{m:.0f}m"
    h = m / 60
    return f"{h:.0f}h" if h < 48 else f"{h / 24:.1f}d"


def probe_oauth_file(name: str, path: Path) -> dict:
    """An OAuth login is not a key: the access token expires constantly and the
    refresh token is what actually keeps the login alive. Reporting only
    presence would call a dead login healthy — the same 'availability is not an
    answer' trap this whole script exists to close."""
    if not path.exists():
        return {"provider": name, "status": AUTH, "model": "",
                "detail": f"no login at {path.name}"}
    try:
        data = json.loads(path.read_text())
    except Exception as exc:
        return {"provider": name, "status": AUTH, "model": "",
                "detail": f"unreadable: {exc}"}

    # Two shapes in the wild: Claude writes epoch-millis fields, Codex stores
    # JWTs whose own payload carries `exp`.
    oauth = data.get("claudeAiOauth") or {}
    if oauth:
        access = oauth.get("expiresAt")
        access = access / 1000 if access else None
        refresh = oauth.get("refreshTokenExpiresAt")
        refresh = refresh / 1000 if refresh else None
        plan = oauth.get("subscriptionType", "?")
    else:
        tokens = data.get("tokens") or {}
        access = _jwt_exp(tokens.get("access_token", ""))
        refresh = None
        plan = data.get("auth_mode", "?")
        if not tokens.get("refresh_token"):
            return {"provider": name, "status": AUTH, "model": plan,
                    "detail": "no refresh token — re-login required"}

    # An expired access token is normal and self-healing. An expired *refresh*
    # token is the one that needs a human at a browser.
    if refresh and refresh < time.time():
        return {"provider": name, "status": AUTH, "model": plan,
                "detail": f"refresh token {_when(refresh)} — re-login required"}

    detail = f"{plan}; access {_when(access)}"
    if refresh:
        detail += f", refresh {_when(refresh)}"
    return {"provider": name, "status": OK, "model": plan, "detail": detail}


def probe_provider(name: str, cfg: dict) -> dict:
    ptype = cfg.get("type", "")
    key = cfg.get("api_key", "")
    base = cfg.get("base_url", "").rstrip("/")
    model = cfg.get("primary_model", "")

    if ptype == "claude-subscription":
        files = cfg.get("credential_files") or [cfg.get("credential_file")]
        files = [f for f in files if f]
        results = [probe_oauth_file(name, Path(f).expanduser()) for f in files]
        alive = [r for r in results if r["status"] == OK]
        # Any one live login carries the provider; name how many so a slow
        # drift from 2/2 to 1/2 is visible before it reaches 0.
        best = alive[0] if alive else (results[0] if results else None)
        if not best:
            return {"provider": name, "status": AUTH, "model": model,
                    "detail": "no credential files configured"}
        return {"provider": name, "status": best["status"], "model": model,
                "detail": f"{len(alive)}/{len(files)} logins live; {best['detail']}"}

    if not key:
        return {"provider": name, "status": SKIPPED, "model": model,
                "detail": "no api_key configured"}

    payload = {"model": model,
               "messages": [{"role": "user", "content": "ping"}],
               "max_tokens": 1}
    code, body = curl_json(f"{base}/chat/completions", key, payload,
                           virtual_key=cfg.get("virtual_key", ""))
    status = classify(code, body)

    detail = f"HTTP {code}"
    bal = native_balance(name, base, key)
    if bal:
        detail += f" | balance {bal}"
    if status != OK:
        snippet = " ".join(body.split())[:160]
        if snippet:
            detail += f" | {snippet}"
    return {"provider": name, "status": status, "model": model, "detail": detail}


def roles(cfg: dict) -> dict[str, str]:
    """Which model each configured role depends on.

    Includes every agent's own primary, not just the global roles: yesterday a
    provider died and the blast radius had to be worked out by hand from seven
    agent.json files. An agent whose primary *and* subconscious share one
    provider has no degraded mode, and that is only visible if both are listed.
    """
    out = {}
    if m := cfg.get("subconscious", {}).get("model"):
        out["subconscious"] = m
    if m := cfg.get("reflection", {}).get("model"):
        out["reflection"] = m
    if m := cfg.get("archivist", {}).get("compression_model"):
        out["archivist"] = m

    agents = Path.home() / ".souveraine" / "server" / "agents"
    for path in sorted(agents.glob("*/agent.json")):
        try:
            data = json.loads(path.read_text())
        except Exception:
            continue
        name = data.get("name") or path.parent.name[:8]
        # The top-level `model` key is always null; the live value is nested.
        if m := (data.get("llm_config") or {}).get("model"):
            out[f"{name}:primary"] = m
        if m := (data.get("_souveraine") or {}).get("subconscious_model"):
            out[f"{name}:sub"] = m
    return out


def selftest() -> int:
    """Prove the classifier can reject, using real captured bodies.

    Every string below was copied out of an actual HTTP response during
    2026-08-12/13.  Nothing here is paraphrased -- a fixture I wrote from
    memory would only test my memory.  The negative cases matter most: a
    context overflow and an overload are NOT bankruptcy, and reporting them
    as such would send someone to a billing page over a full conversation.
    """
    cases = [
        # (label, http code, body, expected)
        ("deepseek 402 bankrupt", 402,
         '{"error":{"message":"Insufficient Balance","type":"unknown_error",'
         '"param":null,"code":"invalid_request_error"}}', NO_BALANCE),
        ("zai 429 code 1113 bankrupt", 429,
         '{"error":{"code":"1113","message":"Insufficient balance or no '
         'resource package. Please recharge."}}', NO_BALANCE),
        ("opencode zen 401 bankrupt", 401,
         '{"type":"error","error":{"type":"CreditsError","message":'
         '"Insufficient balance. Manage your billing here: '
         'https://opencode.ai/workspace/wrk_01KZ/billing"}}', NO_BALANCE),
        ("opencode zen free tier capped", 429,
         '{"type":"error","error":{"type":"FreeUsageLimitError","message":'
         '"Free usage limit reached."}}', CAPPED),
        ("opencode go healthy", 200,
         '{"id":"49dd810d","object":"chat.completion","model":'
         '"deepseek-v4-flash","choices":[{"message":{"content":"alive"}}],'
         '"cost":"0"}', OK),
        # --- negatives: these must NOT read as bankruptcy ---
        ("context overflow is not bankruptcy", 400,
         '{"error":{"message":"This model\'s maximum context length is '
         '1048576 tokens. However, you requested 1377662 tokens (1377662 in '
         'the messages, 0 in the completion). Please reduce the length of '
         'the messages or completion.","type":"invalid_request_error"}}',
         UNREACHABLE),
        ("anthropic 529 overload is not bankruptcy", 529,
         '{"type":"error","error":{"type":"overloaded_error",'
         '"message":"Overloaded"}}', UNREACHABLE),
        ("timeout is unreachable", 0, "timed out", UNREACHABLE),
        ("bad key is auth, not bankruptcy", 401,
         '{"error":{"message":"Invalid API key","type":"authentication_error"}}',
         AUTH),
    ]
    failed = 0
    for label, code, body, expected in cases:
        got = classify(code, body)
        ok = got == expected
        failed += not ok
        print(f"  {'PASS' if ok else 'FAIL'}  {label:<42} -> {got}"
              + ("" if ok else f"  (expected {expected})"))

    # --- OAuth expiry: presence is not liveness -------------------------
    import tempfile
    hour = 3600
    now = time.time()
    oauth_cases = [
        # An expired access token with a live refresh token is the *normal*
        # steady state — both real Claude logins on this box look like this.
        ("claude: access expired, refresh live", OK,
         {"claudeAiOauth": {"expiresAt": int((now - 11 * hour) * 1000),
                            "refreshTokenExpiresAt": int((now + 27 * 24 * hour) * 1000),
                            "subscriptionType": "pro"}}),
        ("claude: refresh expired needs a human", AUTH,
         {"claudeAiOauth": {"expiresAt": int((now - 99 * hour) * 1000),
                            "refreshTokenExpiresAt": int((now - hour) * 1000),
                            "subscriptionType": "pro"}}),
        ("codex: jwt with refresh token is fine", OK,
         {"auth_mode": "chatgpt",
          "tokens": {"access_token": _fake_jwt(now + 10 * 24 * hour),
                     "refresh_token": "rt.1.xyz"}}),
        ("codex: no refresh token needs re-login", AUTH,
         {"auth_mode": "chatgpt",
          "tokens": {"access_token": _fake_jwt(now + hour)}}),
    ]
    for label, expected, blob in oauth_cases:
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as fh:
            json.dump(blob, fh)
            tmp = Path(fh.name)
        got = probe_oauth_file("t", tmp)["status"]
        tmp.unlink()
        ok = got == expected
        failed += not ok
        print(f"  {'PASS' if ok else 'FAIL'}  {label:<42} -> {got}"
              + ("" if ok else f"  (expected {expected})"))

    missing = probe_oauth_file("t", Path("/nonexistent/auth.json"))["status"]
    ok = missing == AUTH
    failed += not ok
    print(f"  {'PASS' if ok else 'FAIL'}  {'absent login is AUTH, not OK':<42} -> {missing}")

    total = len(cases) + len(oauth_cases) + 1
    print(f"\n{total - failed}/{total} passed")
    return 1 if failed else 0


def _fake_jwt(exp: float) -> str:
    """Only the payload segment is ever read, so a signature is unnecessary."""
    payload = base64.urlsafe_b64encode(
        json.dumps({"exp": int(exp)}).encode()).decode().rstrip("=")
    return f"header.{payload}.sig"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--all", action="store_true",
                    help="probe every provider, not just role-backing ones")
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--selftest", action="store_true",
                    help="run the classifier against real captured bodies")
    args = ap.parse_args()

    if args.selftest:
        return selftest()

    try:
        with CONFIG.open("rb") as fh:
            cfg = tomllib.load(fh)
    except Exception as exc:
        print(f"cannot read {CONFIG}: {exc}", file=sys.stderr)
        return 2

    providers = cfg.get("providers", {})
    models = cfg.get("models", {})
    role_models = roles(cfg)

    # model alias -> provider name
    needed = {}
    for role, model in role_models.items():
        entry = models.get(model)
        if not entry:
            needed.setdefault("<unregistered:" + model + ">", []).append(role)
        else:
            needed.setdefault(entry.get("provider", "?"), []).append(role)

    targets = list(providers) if args.all else [p for p in needed if p in providers]
    results = [probe_provider(n, providers[n]) for n in targets]

    for r in results:
        r["roles"] = needed.get(r["provider"], [])

    # External logins are reported, never fatal — nothing routes through them.
    for name, path in EXTERNAL_LOGINS.items():
        if args.all or path.exists():
            ext = probe_oauth_file(name, path)
            ext["roles"] = []
            results.append(ext)

    unregistered = [k for k in needed if k.startswith("<unregistered:")]
    failing = [r for r in results if r["roles"] and r["status"] != OK]

    if args.json:
        print(json.dumps({"results": results, "unregistered": unregistered,
                          "failing": [r["provider"] for r in failing]}, indent=2))
    else:
        width = max((len(r["provider"]) for r in results), default=8)
        for r in results:
            tag = ",".join(r["roles"]) or "-"
            print(f"{r['provider']:<{width}}  {r['status']:<13} {tag:<28} {r['detail']}")
        for u in unregistered:
            print(f"{u}  NOT IN [models.*] -- role(s): {','.join(needed[u])}")
        if failing:
            print()
            for r in failing:
                print(f"!! {','.join(r['roles'])} will fail: "
                      f"{r['provider']} is {r['status']}")

    return 1 if (failing or unregistered) else 0


if __name__ == "__main__":
    sys.exit(main())
