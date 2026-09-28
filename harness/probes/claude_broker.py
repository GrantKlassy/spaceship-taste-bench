"""Inspect Claude's broker login without emitting tokens or account details."""
import json
import os
import pathlib
import subprocess

placeholder_auth = False
auth_path = pathlib.Path("/home/agent/.claude/.credentials.json")
if auth_path.is_file() and not auth_path.is_symlink() and auth_path.stat().st_size <= 16384:
    try:
        oauth = json.loads(auth_path.read_text()).get("claudeAiOauth", {})
        placeholder_auth = (
            oauth.get("accessToken") == "sk-ant-oat01-proxy-managed"
            and oauth.get("refreshToken") == "sk-ant-ort01-proxy-managed"
        )
    except (ValueError, OSError, AttributeError):
        pass

subscription_login = False
try:
    status = subprocess.run(
        ["claude", "auth", "status", "--json"],
        capture_output=True, text=True, timeout=15,
    )
    auth = json.loads(status.stdout)
    subscription_login = (
        status.returncode == 0 and auth.get("loggedIn") is True
        and auth.get("authMethod") == "claude.ai"
        and auth.get("apiProvider") == "firstParty"
    )
except (ValueError, OSError, AttributeError, subprocess.TimeoutExpired):
    pass

print(json.dumps({
    "placeholder_auth": placeholder_auth,
    "subscription_login": subscription_login,
    "no_api_key_override": not any(os.environ.get(key) for key in (
        "ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "CLAUDE_CODE_OAUTH_TOKEN",
        "ANTHROPIC_BASE_URL", "CLAUDE_CODE_USE_BEDROCK", "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY", "ANTHROPIC_PROFILE",
    )),
}))
