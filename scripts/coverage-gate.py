#!/usr/bin/env python3
"""Coverage gate: every production line, function and region must be covered.

Usage: coverage-gate.py <cov.json> <lcov.info> <workspace-root> [<path-prefix>...]

Reads a `cargo llvm-cov` JSON export (functions and regions) and its lcov export
(lines) and checks the production sources under `crates/*/src`, or only those
under the given path prefixes. Two kinds of code are left out of the count:

- test code: items under `#[cfg(test)]` (inline test modules and helpers) and
  the files of `#[cfg(test)] mod name;` declarations (integration tests under
  `crates/*/tests` are never counted);
- waived lines: a line carrying `// coverage: unreachable, <reason>` (or the
  line right below such a comment standing on its own). A waiver states why no
  input can reach the code, so every exclusion stays deliberate and reviewable.

Prints each production gap and the waivers in effect; exits 1 on any gap.
"""

import json
import re
import sys
from collections import defaultdict
from pathlib import Path

WAIVER = re.compile(r"//\s*coverage:\s*unreachable,\s*\S")
CFG_TEST = re.compile(r"^\s*#\[cfg\(test\)\]\s*$")
OUTLINE_MOD = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;")
SOURCE = re.compile(r"(^|/)crates/[^/]+/src/")


def code_only(text: str) -> str:
    """Blanks comments, strings and char literals so braces can be counted."""
    out = []
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            out.append(" " * (j - i))
            i = j
        elif text.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if text.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif text.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            out.append(re.sub(r"[^\n]", " ", text[i:j]))
            i = j
        elif (m := re.match(r'b?r(#*)"', text[i:])) and (i == 0 or not text[i - 1].isalnum()):
            end = text.find('"' + m.group(1), i + m.end())
            j = n if end < 0 else end + 1 + len(m.group(1))
            out.append(re.sub(r"[^\n]", " ", text[i:j]))
            i = j
        elif c == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            out.append(re.sub(r"[^\n]", " ", text[i : j + 1]))
            i = j + 1
        elif c == "'" and (m := re.match(r"'(\\.[^']*|[^\\'])'", text[i:])):
            out.append(" " * m.end())
            i += m.end()
        else:
            out.append(c)
            i += 1
    return "".join(out)


def test_ranges(code: list[str]) -> list[tuple[int, int]]:
    """1-based inclusive line ranges of items under `#[cfg(test)]`."""
    ranges = []
    for index, line in enumerate(code):
        # matched on the blanked text, so an attribute inside a string never counts
        if not CFG_TEST.match(line):
            continue
        # the attributed item starts at the next line that is no attribute
        start = index + 1
        while start < len(code) and code[start].lstrip().startswith("#["):
            start += 1
        depth, opened, end = 0, False, start
        for end in range(start, len(code)):
            for char in code[end]:
                if char == "{":
                    depth, opened = depth + 1, True
                elif char == "}":
                    depth -= 1
            if opened and depth <= 0:
                break
            if not opened and code[end].rstrip().endswith(";"):
                break
        ranges.append((index + 1, end + 1))
    return ranges


def test_files(root: Path, sources: list[Path]) -> set[Path]:
    """Files declared only under `#[cfg(test)]`, plus everything below them."""
    found: set[Path] = set()
    for path in sources:
        lines = code_only(path.read_text()).split("\n")
        for index, line in enumerate(lines):
            if not CFG_TEST.match(line):
                continue
            nxt = index + 1
            while nxt < len(lines) and lines[nxt].lstrip().startswith("#["):
                nxt += 1
            m = OUTLINE_MOD.match(lines[nxt]) if nxt < len(lines) else None
            if not m:
                continue
            base = path.parent if path.name in ("lib.rs", "main.rs", "mod.rs") else path.with_suffix("")
            for candidate in (base / f"{m.group(1)}.rs", base / m.group(1) / "mod.rs"):
                if candidate.exists():
                    found.add(candidate.resolve())
            directory = base / m.group(1)
            if directory.is_dir():
                found.update(p.resolve() for p in directory.rglob("*.rs"))
    return found


def excluded_lines(path: Path) -> tuple[set[int], dict[int, str]]:
    """Test-code lines and waived lines (with their reasons) of one file."""
    text = path.read_text()
    lines = text.split("\n")
    code = code_only(text).split("\n")
    tests: set[int] = set()
    for start, end in test_ranges(code):
        tests.update(range(start, end + 1))
    waived: dict[int, str] = {}
    for number, line in enumerate(lines, start=1):
        m = WAIVER.search(line)
        if not m:
            continue
        reason = line[m.start() :].split(",", 1)[1].strip()
        if line.strip().startswith("//"):
            target = number + 1
            while target <= len(lines) and lines[target - 1].strip().startswith("//"):
                target += 1
            waived[target] = reason
        else:
            waived[number] = reason
    return tests, waived


def main() -> int:
    cov_path, lcov_path, root_arg = sys.argv[1:4]
    prefixes = tuple(sys.argv[4:]) or ("",)
    root = Path(root_arg).resolve()
    data = json.loads(Path(cov_path).read_text())["data"][0]

    def rel(name: str) -> str:
        path = Path(name).resolve()
        return str(path.relative_to(root)) if root in path.parents else name

    production = {
        rel(f["filename"]): f
        for f in data["files"]
        if SOURCE.search(rel(f["filename"])) and rel(f["filename"]).startswith(prefixes)
    }
    sources = [root / name for name in production]
    in_test_file = test_files(root, sources)

    gaps: dict[str, dict[str, list]] = defaultdict(lambda: defaultdict(list))
    waivers: list[str] = []
    skip: dict[str, set[int]] = {}
    for name in production:
        path = root / name
        if path.resolve() in in_test_file:
            continue
        tests, waived = excluded_lines(path)
        skip[name] = tests | set(waived)
        waivers.extend(f"{name}:{line}: {reason}" for line, reason in sorted(waived.items()))

    # regions: segment [line, col, count, has_count, is_region_entry, is_gap]
    for name, entry in production.items():
        if name not in skip:
            continue
        for line, col, count, has_count, is_entry, is_gap, *_ in entry["segments"]:
            if has_count and is_entry and not is_gap and count == 0 and line not in skip[name]:
                gaps[name]["regions"].append(f"{line}:{col}")

    # lines: lcov DA records
    current = None
    for raw in Path(lcov_path).read_text().split("\n"):
        if raw.startswith("SF:"):
            current = rel(raw[3:])
        elif raw.startswith("DA:") and current in skip:
            line, count = (int(v) for v in raw[3:].split(",")[:2])
            if count == 0 and line not in skip[current]:
                gaps[current]["lines"].append(line)

    # functions: covered when any instantiation ran
    instances: dict[tuple[str, int, int], bool] = {}
    for function in data["functions"]:
        if not function["filenames"] or not function["regions"]:
            continue
        name = rel(function["filenames"][0])
        line, col = function["regions"][0][:2]
        if name in skip and line not in skip[name]:
            key = (name, line, col)
            instances[key] = instances.get(key, False) or function["count"] > 0
    for (name, line, col), covered in sorted(instances.items()):
        if not covered:
            gaps[name]["functions"].append(f"{line}:{col}")

    for reason in waivers:
        print(f"waived {reason}")
    total = 0
    for name in sorted(gaps):
        for kind in ("lines", "functions", "regions"):
            found = gaps[name][kind]
            if found:
                total += len(found)
                print(f"GAP {name} {kind}: {' '.join(str(item) for item in found)}")
    print(f"coverage gate: {len(skip)} production files, {len(waivers)} waived lines, {total} gaps")
    return 1 if total else 0


if __name__ == "__main__":
    sys.exit(main())
