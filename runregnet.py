#!/usr/bin/env python3
"""Compatibility wrapper for the clearer `regnet.py` launcher."""

from runtime_launcher import main


if __name__ == "__main__":
    raise SystemExit(
        main(
            "regnet",
            prog="runregnet.py",
            compatibility_note="runregnet.py is kept for compatibility; prefer regnet.py.",
        )
    )
