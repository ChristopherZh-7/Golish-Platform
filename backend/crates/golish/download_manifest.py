#!/usr/bin/env python3
import urllib.request
import sys

url = "http://8.138.179.62:8080/.vite/manifest.json"
output = "./vite_manifest.json"

try:
    with urllib.request.urlopen(url) as response:
        data = response.read()
        with open(output, 'wb') as f:
            f.write(data)
        print(f"Downloaded {url} to {output}, status: {response.status}")
except Exception as e:
    print(f"Failed to download: {e}", file=sys.stderr)
    sys.exit(1)