#!/usr/bin/env python3
"""Rewrite the generated C header's doc comments into C-appropriate prose.

cbindgen copies Rust doc comments verbatim, which would leave Rust-only
markdown in the C header:

  * ``[`Foo`]`` / ``[`Foo`](path)`` intra-doc links
  * whole fenced code blocks containing Rust (`use librtmp2::…`,
    `assert_eq!`) -- a C reader cannot use these and they are misleading
  * ``# Heading`` section markers, which read like preprocessor directives
    inside a comment
  * ``**bold**`` emphasis

This runs on the freshly generated header, so the Rust sources keep their links
and examples (they still work on docs.rs) and only the published header is
normalised.

Usage: fix-header-docs.py <header>
"""

import re
import sys

DOC_PREFIX = re.compile(r"^(?P<indent>\s*)\* ?")
FENCE = re.compile(r"^\s*\* ?```")
HEADING = re.compile(r"^(?P<indent>\s*)\* (?P<hashes>#{1,6}) (?P<title>.+?)\s*$")


def strip_code_fences(lines):
    """Remove every fenced code block, plus the blank comment lines that
    surrounded it. All fences in this header come from Rust doc examples."""
    out = []
    i = 0
    while i < len(lines):
        if FENCE.match(lines[i]):
            # Opening fence: skip it and everything up to the closing fence.
            i += 1
            while i < len(lines):
                closing = bool(FENCE.match(lines[i]))
                i += 1
                if closing:
                    break
            # Drop blank comment lines left dangling before/after.
            while i < len(lines):
                m = DOC_PREFIX.match(lines[i])
                if m and lines[i][m.end():].strip() == "":
                    i += 1
                    continue
                break
            while out:
                m = DOC_PREFIX.match(out[-1])
                if m and out[-1][m.end():].strip() == "":
                    out.pop()
                    continue
                break
            continue
        out.append(lines[i])
        i += 1
    return out


def headings_to_prose(lines):
    out = []
    for line in lines:
        m = HEADING.match(line)
        if m:
            line = f"{m.group('indent')}* {m.group('title')}:"
        out.append(line)
    return out


def drop_empty_sections(lines):
    """Remove a heading whose section body is empty (its only content was a
    code block that `strip_code_fences` deleted)."""
    out = []
    for i, line in enumerate(lines):
        m = HEADING.match(line)
        if not m:
            # already converted? look for "Title:" form
            dm = re.match(r"^(?P<indent>\s*)\* (?P<title>[A-Z][A-Za-z ]+):$", line)
            if dm is None:
                out.append(line)
                continue
        indent = (m or dm).group("indent")
        # Look ahead past blank comment lines for the end of the doc block.
        j = i + 1
        while j < len(lines):
            nm = DOC_PREFIX.match(lines[j])
            if nm and lines[j][nm.end():].strip() == "":
                j += 1
                continue
            break
        nxt = lines[j].strip() if j < len(lines) else ""
        if nxt in ("*/", "};") or nxt.startswith("/**"):
            continue  # empty section -> drop the heading
        out.append(line)
    return out


def normalise_text(text):
    # [`X`](target) and [`X`][ref] -> `X`
    text = re.sub(r"\[`([^`]*)`\]\([^)]*\)", r"`\1`", text)
    text = re.sub(r"\[`([^`]*)`\]\[[^\]]*\]", r"`\1`", text)
    # [`X`] -> `X`
    text = re.sub(r"\[`([^`]*)`\]", r"`\1`", text)
    # **bold** / __bold__ -> bold
    text = re.sub(r"\*\*([^*]+)\*\*", r"\1", text)
    text = re.sub(r"__([^_]+)__", r"\1", text)
    # Bare `crate::…` / `Self::…` paths are Rust-only; shorten the prefix so the
    # prose still reads correctly in C.
    text = text.replace("`crate::", "`")
    text = text.replace("`Self::", "`")
    # Inline mentions of a section marker, e.g. "see that function's `# Safety`
    # section", read as preprocessor directives; spell the word instead.
    for h in ("Safety", "Errors", "Panics", "Examples"):
        text = text.replace(f"`# {h}`", h)
        text = text.replace(f"# {h}", h)
    return text


def trim_block_edges(lines):
    """Drop blank comment lines immediately before `*/` or after `/**`."""
    out = []
    for line in lines:
        s = line.strip()
        if s == "*/":
            while out:
                m = DOC_PREFIX.match(out[-1])
                if m and out[-1][m.end():].strip() == "":
                    out.pop()
                    continue
                break
        out.append(line)
        if s.startswith("/**"):
            # nothing to do; the next line is written normally
            pass
    return out


def main():
    path = sys.argv[1]
    with open(path, encoding="utf-8", newline="") as f:
        lines = f.read().split("\n")

    lines = strip_code_fences(lines)
    lines = headings_to_prose(lines)
    lines = drop_empty_sections(lines)
    lines = [normalise_text(l) for l in lines]
    lines = trim_block_edges(lines)

    text = "\n".join(lines)

    # Guard: the whole point of this pass is that none of the Rust-only
    # markdown survives. Fail loudly rather than silently shipping a header
    # that still contains Rust code or links.
    leftovers = []
    if "[`" in text:
        leftovers.append("intra-doc links")
    if FENCE.search(text):
        leftovers.append("code fences")
    if HEADING.search(text):
        leftovers.append("# headings")
    if "**" in text and re.search(r"\*\*[^*\n]+\*\*", text):
        leftovers.append("bold markers")
    if leftovers:
        sys.stderr.write(
            f"error: {path} still contains Rust doc markdown: "
            + ", ".join(leftovers)
            + "\n"
        )
        return 1

    with open(path, "w", encoding="utf-8", newline="") as f:
        f.write(text)
    print(f"Normalised doc comments in {path}")
    return 0


sys.exit(main())
