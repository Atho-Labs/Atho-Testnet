#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import html
import re
import subprocess
import tempfile
from pathlib import Path

import markdown


ROOT = Path(__file__).resolve().parent.parent
SOURCE_DIR = ROOT / "docs" / "whitepaper"
DOCUMENTS = {
    "ATHO_WHITE_PAPER.md": "ATHO_WHITE_PAPER.pdf",
    "ATHO_MONETARY_POLICY_AND_150_YEAR_SUPPLY_SCHEDULE.md": (
        "ATHO_MONETARY_POLICY_AND_150_YEAR_SUPPLY_SCHEDULE.pdf"
    ),
}


CSS = """
@page {
  margin: 0.9in 0.85in;
}

html, body {
  background: #ffffff;
  color: #111111;
  font-family: "Liberation Serif", "Times New Roman", serif;
  font-size: 11.5pt;
  line-height: 1.45;
}

body {
  max-width: 7.15in;
  margin: 0 auto;
}

h1, h2, h3, h4 {
  color: #000000;
  font-weight: 700;
  page-break-after: avoid;
}

h1 {
  font-size: 21pt;
  text-align: center;
  margin: 0.25in 0 0.15in;
}

h2 {
  font-size: 15pt;
  margin-top: 0.28in;
  border-top: 1px solid #000000;
  padding-top: 0.12in;
}

h3 {
  font-size: 12.5pt;
  margin-top: 0.2in;
}

p, li {
  orphans: 3;
  widows: 3;
}

strong {
  font-weight: 700;
}

em {
  font-style: italic;
}

hr {
  border: 0;
  border-top: 1px solid #000000;
  margin: 0.2in 0;
}

blockquote {
  border-left: 2px solid #000000;
  margin: 0.12in 0 0.12in 0.12in;
  padding-left: 0.14in;
}

pre, code {
  font-family: "Liberation Mono", "Courier New", monospace;
}

pre {
  white-space: pre-wrap;
  word-break: break-word;
  border: 1px solid #000000;
  padding: 0.12in;
  margin: 0.14in 0;
  background: #ffffff;
  font-size: 9.5pt;
}

table {
  width: 100%;
  border-collapse: collapse;
  margin: 0.16in 0 0.2in;
  font-size: 10pt;
}

th, td {
  border: 1px solid #000000;
  padding: 0.08in 0.07in;
  text-align: left;
  vertical-align: top;
}

th {
  background: #ffffff;
  font-weight: 700;
}

ul, ol {
  padding-left: 0.24in;
}

.doc-meta {
  text-align: center;
  margin-bottom: 0.2in;
}

.doc-meta p {
  margin: 0.03in 0;
}

.title-page {
  min-height: 8.6in;
  display: flex;
  flex-direction: column;
  justify-content: center;
  text-align: center;
  page-break-after: always;
}

.title-page h1,
.title-page h2 {
  border-top: 0;
  padding-top: 0;
  margin-top: 0;
}

.title-page h1 {
  font-size: 23pt;
  margin-bottom: 0.12in;
}

.title-page h2 {
  font-size: 14pt;
  margin-bottom: 0.26in;
}

.title-page p {
  margin: 0.05in 0;
}
"""


def extract_title(markdown_text: str, fallback: str) -> str:
    for line in markdown_text.splitlines():
        stripped = line.strip()
        if stripped.startswith("# "):
            return stripped[2:].strip()
    return fallback


def render_html(source_path: Path) -> str:
    markdown_text = source_path.read_text(encoding="utf-8")
    title = extract_title(markdown_text, source_path.stem.replace("_", " "))
    body = markdown.markdown(
        markdown_text,
        extensions=[
            "extra",
            "tables",
            "fenced_code",
            "sane_lists",
            "toc",
        ],
        output_format="html5",
    )
    body = add_title_page(body)
    return f"""<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <title>{html.escape(title)}</title>
    <style>{CSS}</style>
  </head>
  <body>
    {body}
  </body>
</html>
"""


def add_title_page(body: str) -> str:
    match = re.match(
        r"(?s)^\s*(<h1>.*?</h1>\s*(?:<h2>.*?</h2>\s*)?<p>.*?</p>)(.*)$",
        body,
    )
    if not match:
        return body
    title_block, remainder = match.groups()
    return f'<section class="title-page">{title_block}</section>{remainder}'


def convert_html_to_pdf(html_text: str, output_path: Path) -> None:
    with tempfile.TemporaryDirectory() as temp_dir_name:
        temp_dir = Path(temp_dir_name)
        html_path = temp_dir / f"{output_path.stem}.html"
        html_path.write_text(html_text, encoding="utf-8")
        subprocess.run(
            [
                "soffice",
                "--headless",
                "--convert-to",
                "pdf",
                "--outdir",
                str(temp_dir),
                str(html_path),
            ],
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        rendered_pdf = temp_dir / output_path.name
        output_path.write_bytes(rendered_pdf.read_bytes())


def main() -> None:
    for source_name, output_name in DOCUMENTS.items():
        source_path = SOURCE_DIR / source_name
        output_path = ROOT / output_name
        html_text = render_html(source_path)
        convert_html_to_pdf(html_text, output_path)
        print(f"rendered {output_path.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
