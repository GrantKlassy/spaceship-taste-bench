"""Fixed non-billable destinations, tested with and without proxy environment use."""
import urllib.error
import urllib.request
import socket
import json
import http.client

direct = urllib.request.build_opener(urllib.request.ProxyHandler({}))
observations = {"proxy_http_denials": 0, "direct_http_denials": 0,
                "direct_dns_denials": 0, "direct_transport_failures": 0}
for target in (
    "https://example.com", "https://github.com", "http://169.254.169.254",
    "http://host.docker.internal:80", "https://api.anthropic.com",
    "https://api.openai.com", "https://index.crates.io/config.json",
):
    for open_url in (urllib.request.urlopen, direct.open):
        try:
            open_url(target, timeout=3)
        except urllib.error.HTTPError as error:
            assert error.code == 403
            observations["direct_http_denials" if open_url == direct.open else "proxy_http_denials"] += 1
            continue
        except urllib.error.URLError as error:
            if open_url == direct.open and isinstance(error.reason, socket.gaierror):
                assert error.reason.errno in (socket.EAI_NONAME, socket.EAI_NODATA)
                observations["direct_dns_denials"] += 1
                continue
            if open_url == direct.open and isinstance(error.reason, ConnectionRefusedError):
                # Record unreachability, not proof that policy caused it. The
                # controller keeps comprehensive raw-IP acceptance unresolved.
                observations["direct_transport_failures"] += 1
                continue
            raise
        except http.client.RemoteDisconnected:
            if open_url == direct.open:
                observations["direct_transport_failures"] += 1
                continue
            raise
        raise SystemExit("expected an explicit proxy denial")
assert observations["proxy_http_denials"] == 7
print(json.dumps(observations))
