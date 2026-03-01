#!/usr/bin/env python3
# FILE: tests/gen_tests_badge.py
"""
Generate a JSON badge (Shields.io format) from a JUnit report produced by pytest.

Goal: automatically link the "X/Y tests" badge display to what was actually
executed by CI.

Example:
    pytest --junitxml=test-report.xml && python tests/gen_tests_badge.py test-report.xml tests.json
"""

from __future__ import annotations

import argparse
import json
import xml.etree.ElementTree as ET
from pathlib import Path
from typing import Tuple


def summarize_junit(path: Path) -> Tuple[int, int, int, int]:
    """Return (total, passed, failed, skipped) from a JUnit report."""
    tree = ET.parse(path)
    root = tree.getroot()

    if root.tag == "testsuite":
        suites = [root]
    elif root.tag == "testsuites":
        suites = list(root.findall("testsuite"))
    else:
        suites = list(root.findall(".//testsuite"))

    if not suites:
        return 0, 0, 0, 0

    total = 0
    failed = 0
    skipped = 0

    for suite in suites:
        tests = int(suite.attrib.get("tests", 0))
        failures = int(suite.attrib.get("failures", 0))
        errors = int(suite.attrib.get("errors", 0))
        skips = int(suite.attrib.get("skipped", 0))

        total += tests
        failed += failures + errors
        skipped += skips

    passed = max(total - failed - skipped, 0)
    return total, passed, failed, skipped


def make_badge(total: int, passed: int, failed: int, skipped: int) -> dict:
    """Build the JSON structure expected by Shields.io."""
    if total <= 0:
        message = "0/0"
        color = "lightgrey"
    else:
        message = f"{passed}/{total}"
        if failed:
            color = "red"
        elif skipped:
            color = "yellow"
        else:
            color = "brightgreen"

    return {
        "schemaVersion": 1,
        "label": "tests",
        "message": message,
        "color": color,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="Generate a 'tests' JSON badge from a pytest JUnit report.")
    parser.add_argument("junit_xml", type=Path, help="JUnit XML file produced by pytest.")
    parser.add_argument("output", type=Path, help="Output path for the badge JSON.")
    args = parser.parse_args()

    total, passed, failed, skipped = summarize_junit(args.junit_xml)
    badge = make_badge(total, passed, failed, skipped)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(badge))

    print(
        f"Generated tests badge: {badge['message']} "
        f"(failed={failed}, skipped={skipped}, color={badge['color']}) -> {args.output}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
