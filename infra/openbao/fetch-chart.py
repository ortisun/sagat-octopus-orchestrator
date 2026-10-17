#!/usr/bin/env python3
"""Fetch the reviewed official chart into an operator-chosen directory; verify before use."""
import hashlib
import pathlib
import sys
import urllib.request

VERSION = "0.30.0"
SHA256 = "ee6c2642f63efa06edc1f8b70b7652fc01f82c832c6e3a272f93427b0dfe26bf"
url = f"https://github.com/openbao/openbao-helm/releases/download/openbao-{VERSION}/openbao-{VERSION}.tgz"
if len(sys.argv) != 2:
    raise SystemExit("usage: fetch-chart.py OUTPUT.tgz")
with urllib.request.urlopen(url, timeout=30) as response:
    data = response.read(8 * 1024 * 1024 + 1)
if hashlib.sha256(data).hexdigest() != SHA256:
    raise SystemExit("chart checksum mismatch")
with pathlib.Path(sys.argv[1]).open("xb") as output:
    output.write(data)
print(f"verified OpenBao chart {VERSION}")
