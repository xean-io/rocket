#!/usr/bin/env python3
"""Captures golden responses from a reference (Go) rocketd.

usage: capture.py /path/to/go/rocket   (writes ./<name>.golden next to this file)

Each golden file is `<status>\n<raw body bytes>`. The scenario is shared with
tests/golden.rs (scenario.tsv). Uses a throw-away ROCKET_HOME under /tmp and a
copy of testdata/fixture; never touches the real ~/.rocket.
"""
import http.client, json, os, shutil, socket, subprocess, sys, tempfile, time

here = os.path.dirname(os.path.abspath(__file__))
rocket = sys.argv[1]
fixture = os.path.join(here, "..", "..", "..", "..", "testdata", "fixture")
tmp = os.path.realpath(tempfile.mkdtemp(prefix="rkg", dir="/tmp"))
home, proj = os.path.join(tmp, "home"), os.path.join(tmp, "proj")
shutil.copytree(fixture, proj)
env = dict(os.environ, ROCKET_HOME=home)

class Unix(http.client.HTTPConnection):
    def __init__(self, path):
        super().__init__("rocketd")
        self.path = path
    def connect(self):
        self.sock = socket.socket(socket.AF_UNIX)
        self.sock.connect(self.path)

def request(conn, method, path, body):
    conn.request(method, path, body=body.encode() if body is not None else None)
    r = conn.getresponse()
    return r.status, r.read()

subprocess.run([rocket, "daemon", "start"], env=env, check=True, capture_output=True)
try:
    sock = os.path.join(home, "rocketd.sock")
    info = json.load(open(os.path.join(home, "daemon.json")))
    saved = {"PROJ": proj}
    for line in open(os.path.join(here, "scenario.tsv")):
        line = line.rstrip("\n")
        if not line or line.startswith("#"):
            continue
        name, transport, method, path, body, save = line.split("\t")
        for k, v in saved.items():
            path, body = path.replace("{%s}" % k, v), body.replace("{%s}" % k, v)
        body = None if body == "-" else body
        if transport == "unix":
            conn = Unix(sock)
        else:
            host, port = info["http"][len("http://"):].split(":")
            conn = http.client.HTTPConnection(host, int(port))
        status, data = request(conn, method, path, body)
        open(os.path.join(here, name + ".golden"), "wb").write(("%d\n" % status).encode() + data)
        if save != "-":
            key, field = save.split("=")
            saved[key] = json.loads(data)[field]
        print(name, status, len(data))
finally:
    subprocess.run([rocket, "daemon", "stop"], env=env, capture_output=True)
    shutil.rmtree(tmp, ignore_errors=True)
