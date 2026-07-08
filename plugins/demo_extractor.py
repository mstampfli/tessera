#!/usr/bin/env python3
"""A minimal tessera extractor plugin.

Reads raw bytes on stdin, emits NDJSON extraction events on stdout. This example
treats the input as UTF-8 text, emits a title from the first non-empty line, and
one text event per paragraph. It is intentionally simple; a real plugin would
parse a specific format the built-in extractors do not handle.

Runs under the host sandbox (no network, no file writes, cpu/memory/time caps).
"""

import json
import sys


def main() -> None:
    raw = sys.stdin.buffer.read()
    text = raw.decode("utf-8", errors="replace")

    lines = [line.strip() for line in text.splitlines()]
    title = next((line for line in lines if line), None)
    if title:
        print(json.dumps({"event": "meta", "title": title[:200]}))

    paragraph: list[str] = []

    def flush() -> None:
        if paragraph:
            block = " ".join(paragraph).strip()
            if block:
                print(json.dumps({"event": "text", "text": block}))
            paragraph.clear()

    for line in lines:
        if line:
            paragraph.append(line)
        else:
            flush()
    flush()


if __name__ == "__main__":
    main()
