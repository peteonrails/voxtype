#!/usr/bin/env python3
"""xdg-open for the actions-real scenario: plays the browser for Todoist's
consent page. Given the authorize URL a real voxtype opens, it follows
Todoist's redirect back to voxtype's loopback listener with a code, as a
user who clicks Allow would; anything else is just recorded. Sends nothing
off the machine (the API voxtype talks to is the scenario's fake)."""
import os
import subprocess
import sys
import time
import urllib.parse
import urllib.request

url = sys.argv[1] if len(sys.argv) > 1 else ""
with open(os.environ["FAKE_BROWSER_LOG"], "a") as f:
    f.write(url + "\n")
if "/oauth/authorize?" in url and os.environ.get("FAKE_BROWSER_CHILD") != "1":
    # Answer after voxtype has gone back to waiting, from a child so
    # xdg-open itself returns at once, as the real one does.
    subprocess.Popen([sys.executable, "-I", __file__, url], env=dict(os.environ, FAKE_BROWSER_CHILD="1"),
                     start_new_session=True)
    sys.exit(0)
if "/oauth/authorize?" in url:
    q = urllib.parse.parse_qs(urllib.parse.urlsplit(url).query)
    back = q["redirect_uri"][0].replace("localhost", "127.0.0.1") + "?" + urllib.parse.urlencode(
        {"code": "c0de", "state": q["state"][0]})
    for _ in range(50):
        try:
            urllib.request.urlopen(back, timeout=5).read()
            break
        except OSError:
            time.sleep(0.1)
