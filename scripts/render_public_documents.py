#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import html
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

import markdown
from bs4 import BeautifulSoup
from bs4.element import NavigableString, Tag
from reportlab.graphics.shapes import Circle, Drawing, Line, Polygon, Rect, String
from reportlab.lib import colors
from reportlab.lib.enums import TA_CENTER, TA_LEFT
from reportlab.lib.pagesizes import LETTER
from reportlab.lib.styles import ParagraphStyle, getSampleStyleSheet
from reportlab.lib.units import inch
from reportlab.platypus import (
    BaseDocTemplate,
    Frame,
    KeepTogether,
    ListFlowable,
    ListItem,
    PageBreak,
    PageTemplate,
    Paragraph,
    Preformatted,
    Spacer,
    Table,
    TableStyle,
)
from reportlab.platypus.tableofcontents import TableOfContents


ROOT = Path(__file__).resolve().parent.parent
SOURCE_DIR = ROOT / "docs" / "whitepaper"
WHITEPAPER_SOURCE = SOURCE_DIR / "ATHO_WHITE_PAPER.md"
SUPPLEMENT_SOURCE = SOURCE_DIR / "ATHO_MONETARY_POLICY_AND_150_YEAR_SUPPLY_SCHEDULE.md"
WHITEPAPER_PDF = ROOT / "ATHO_WHITE_PAPER.pdf"
SUPPLEMENT_PDF = ROOT / "ATHO_MONETARY_POLICY_AND_150_YEAR_SUPPLY_SCHEDULE.pdf"

AUTHOR = "Ghost Genull"
CONTACT = "labs@atho.io"
TAGLINE = "The Platinum Standard of the Quantum Age"

HEADER_COLOR = colors.HexColor("#5f665f")
TABLE_HEADER_BG = colors.HexColor("#171717")
TABLE_ROW_ALT = colors.HexColor("#f1f2ef")
BOX_FILL = colors.HexColor("#fbfbfa")
BOX_STROKE = colors.HexColor("#111111")
ARROW_STROKE = colors.HexColor("#111111")


@dataclass(frozen=True)
class FigureSpec:
    heading: str
    label: str
    title: str


WHITEPAPER_FIGURES = [
    FigureSpec("4.1. System Flow", "Figure 1.", "Atho System Overview"),
    FigureSpec(
        "8. Transaction Security",
        "Figure 2.",
        "Atho Transaction Signing and Verification Flow",
    ),
    FigureSpec(
        "6. Emission Model and 150-Year Supply Projection",
        "Figure 3.",
        "Atho Emission Model Under Current Consensus",
    ),
    FigureSpec("7.1. Validation Pipeline", "Figure 4.", "Block Validation Pipeline"),
    FigureSpec("10. Network Participants", "Figure 5.", "Wallet-to-Node Interaction"),
    FigureSpec(
        "7. Consensus and Validation",
        "Figure 6.",
        "Validation Pipeline and Parallel Work Distribution",
    ),
]

WHITEPAPER_TABLES = [
    "Table 1. Technical Overview of Current Code Parameters",
    "Table 2. Atho 150-Year Monetary Supply Projection Under the Current Emission Policy",
    "Table 3. Post-Quantum Security Comparison",
]

SUPPLEMENT_TABLES = [
    "Table 1. Network Monetary Parameters",
    "Table 2. Atho 150-Year Monetary Supply Projection Under the Current Emission Policy",
]

KEYWORDS = (
    "Atho, Falcon-512, SHA3-384, proof-of-work, public UTXO, Rust, "
    "post-quantum cryptography, payment network, digital money"
)


def base_styles():
    sheet = getSampleStyleSheet()
    styles = {
        "title": ParagraphStyle(
            "TitlePageTitle",
            parent=sheet["Title"],
            fontName="Times-Bold",
            fontSize=26,
            leading=31,
            alignment=TA_CENTER,
            spaceAfter=0.18 * inch,
            textColor=colors.black,
        ),
        "subtitle": ParagraphStyle(
            "TitlePageSubtitle",
            parent=sheet["Title"],
            fontName="Times-Italic",
            fontSize=17,
            leading=21,
            alignment=TA_CENTER,
            spaceAfter=0.28 * inch,
        ),
        "title_meta": ParagraphStyle(
            "TitlePageMeta",
            parent=sheet["BodyText"],
            fontName="Times-Roman",
            fontSize=12.5,
            leading=17,
            alignment=TA_CENTER,
            spaceAfter=0.06 * inch,
        ),
        "paper_heading": ParagraphStyle(
            "PaperHeading",
            parent=sheet["Heading1"],
            fontName="Times-Bold",
            fontSize=18,
            leading=22,
            alignment=TA_CENTER,
            spaceBefore=0.22 * inch,
            spaceAfter=0.12 * inch,
            keepWithNext=True,
        ),
        "subheading": ParagraphStyle(
            "PaperSubheading",
            parent=sheet["Heading2"],
            fontName="Times-Bold",
            fontSize=13.25,
            leading=16,
            alignment=TA_LEFT,
            spaceBefore=0.14 * inch,
            spaceAfter=0.06 * inch,
            keepWithNext=True,
        ),
        "body": ParagraphStyle(
            "PaperBody",
            parent=sheet["BodyText"],
            fontName="Times-Roman",
            fontSize=12.25,
            leading=18,
            alignment=TA_LEFT,
            firstLineIndent=0.24 * inch,
            spaceAfter=0.08 * inch,
        ),
        "body_no_indent": ParagraphStyle(
            "PaperBodyNoIndent",
            parent=sheet["BodyText"],
            fontName="Times-Roman",
            fontSize=12.25,
            leading=18,
            alignment=TA_LEFT,
            firstLineIndent=0,
            spaceAfter=0.08 * inch,
        ),
        "bullet": ParagraphStyle(
            "PaperBullet",
            parent=sheet["BodyText"],
            fontName="Times-Roman",
            fontSize=12.1,
            leading=17,
            leftIndent=0.2 * inch,
            firstLineIndent=0,
            spaceAfter=0.04 * inch,
        ),
        "caption_label": ParagraphStyle(
            "CaptionLabel",
            parent=sheet["BodyText"],
            fontName="Times-Bold",
            fontSize=12.1,
            leading=14,
            spaceBefore=0.08 * inch,
            spaceAfter=0.02 * inch,
        ),
        "caption_title": ParagraphStyle(
            "CaptionTitle",
            parent=sheet["BodyText"],
            fontName="Times-Italic",
            fontSize=12.1,
            leading=14,
            spaceAfter=0.08 * inch,
        ),
        "figure_caption": ParagraphStyle(
            "FigureCaption",
            parent=sheet["BodyText"],
            fontName="Times-Italic",
            fontSize=11.5,
            leading=14.5,
            spaceBefore=0.05 * inch,
            spaceAfter=0.12 * inch,
        ),
        "pre": ParagraphStyle(
            "PaperPre",
            parent=sheet["Code"],
            fontName="Courier",
            fontSize=10.5,
            leading=13,
            leftIndent=0.14 * inch,
            rightIndent=0.14 * inch,
            spaceBefore=0.05 * inch,
            spaceAfter=0.1 * inch,
        ),
        "toc_heading": ParagraphStyle(
            "TOCHeading",
            parent=sheet["Heading1"],
            fontName="Times-Bold",
            fontSize=18,
            leading=22,
            alignment=TA_CENTER,
            spaceBefore=0.18 * inch,
            spaceAfter=0.18 * inch,
        ),
        "toc_entry": ParagraphStyle(
            "TOCEntry",
            parent=sheet["BodyText"],
            fontName="Times-Roman",
            fontSize=12.2,
            leading=16,
            leftIndent=0.1 * inch,
            firstLineIndent=0,
        ),
        "list_page_entry": ParagraphStyle(
            "ListPageEntry",
            parent=sheet["BodyText"],
            fontName="Times-Roman",
            fontSize=12.2,
            leading=17,
            spaceAfter=0.05 * inch,
        ),
    }
    return styles


STYLES = base_styles()


class TocHeading(Paragraph):
    def __init__(self, text: str, style: ParagraphStyle, level: int = 0):
        super().__init__(text, style)
        self.toc_level = level
        self.heading_text = re.sub(r"<[^>]+>", "", text)


class AthoDocTemplate(BaseDocTemplate):
    def __init__(self, filename: str, header_text: str):
        super().__init__(
            filename,
            pagesize=LETTER,
            rightMargin=0.95 * inch,
            leftMargin=0.95 * inch,
            topMargin=0.95 * inch,
            bottomMargin=0.85 * inch,
        )
        frame = Frame(
            self.leftMargin,
            self.bottomMargin,
            self.width,
            self.height,
            id="normal",
        )
        self.header_text = header_text
        self._bookmark_counter = 0
        template = PageTemplate(id="main", frames=[frame], onPage=self._on_page)
        self.addPageTemplates([template])

    def beforeDocument(self):
        self._bookmark_counter = 0

    def _on_page(self, canvas, doc):
        canvas.saveState()
        canvas.setFont("Times-Roman", 10.5)
        canvas.setFillColor(HEADER_COLOR)
        canvas.drawString(doc.leftMargin, LETTER[1] - 0.55 * inch, self.header_text)
        canvas.drawRightString(
            LETTER[0] - doc.rightMargin,
            LETTER[1] - 0.55 * inch,
            str(canvas.getPageNumber()),
        )
        canvas.restoreState()

    def afterFlowable(self, flowable):
        if hasattr(flowable, "toc_level"):
            self._bookmark_counter += 1
            key = f"bmk-{self._bookmark_counter}"
            self.canv.bookmarkPage(key)
            self.notify(
                "TOCEntry",
                (flowable.toc_level, flowable.heading_text, self.page, key),
            )


def title_page(title: str, subtitle: str, project_line: str | None = None):
    story = [
        Spacer(1, 1.7 * inch),
        Paragraph(title, STYLES["title"]),
        Paragraph(subtitle, STYLES["subtitle"]),
        Paragraph(TAGLINE, STYLES["title_meta"]),
        Spacer(1, 0.25 * inch),
    ]
    if project_line:
        story.append(Paragraph(project_line, STYLES["title_meta"]))
        story.append(Spacer(1, 0.08 * inch))
    story.extend(
        [
            Paragraph(f"Author: {AUTHOR}", STYLES["title_meta"]),
            Paragraph(f"Contact: {CONTACT}", STYLES["title_meta"]),
            PageBreak(),
        ]
    )
    return story


def load_document_body(source_path: Path) -> str:
    text = source_path.read_text(encoding="utf-8")
    lines = text.splitlines()
    index = 0
    if index < len(lines) and lines[index].startswith("# "):
        index += 1
    if index < len(lines) and lines[index].startswith("## "):
        index += 1
    while index < len(lines):
        stripped = lines[index].strip()
        if not stripped or stripped.startswith("**"):
            index += 1
            continue
        break
    return "\n".join(lines[index:])


def split_h2_sections(markdown_text: str) -> list[tuple[str, str]]:
    sections: list[tuple[str, str]] = []
    current_title: str | None = None
    current_lines: list[str] = []
    for line in markdown_text.splitlines():
        if line.startswith("## "):
            if current_title is not None:
                sections.append((current_title, "\n".join(current_lines).strip()))
            current_title = line[3:].strip()
            current_lines = []
        else:
            current_lines.append(line)
    if current_title is not None:
        sections.append((current_title, "\n".join(current_lines).strip()))
    return sections


def markdown_to_html_fragment(markdown_text: str) -> str:
    return markdown.markdown(
        markdown_text,
        extensions=["extra", "tables", "fenced_code", "sane_lists"],
        output_format="html5",
    )


def html_markup(node) -> str:
    if isinstance(node, NavigableString):
        return html.escape(str(node))
    if not isinstance(node, Tag):
        return ""
    if node.name == "br":
        return "<br/>"
    if node.name in {"strong", "b"}:
        inner = "".join(html_markup(child) for child in node.children)
        return f"<b>{inner}</b>"
    if node.name in {"em", "i"}:
        inner = "".join(html_markup(child) for child in node.children)
        return f"<i>{inner}</i>"
    if node.name == "code":
        inner = html.escape(node.get_text())
        return f"<font name='Courier'>{inner}</font>"
    inner = "".join(html_markup(child) for child in node.children)
    return inner


def paragraph_from_tag(tag: Tag, style: ParagraphStyle):
    return Paragraph("".join(html_markup(child) for child in tag.children), style)


def list_flowable(tag: Tag):
    ordered = tag.name == "ol"
    items = []
    for li in tag.find_all("li", recursive=False):
        para = Paragraph("".join(html_markup(child) for child in li.children), STYLES["bullet"])
        items.append(ListItem(para))
    return ListFlowable(
        items,
        bulletType="1" if ordered else "bullet",
        leftIndent=0.22 * inch,
    )


def table_flowable(rows: list[list[str]]):
    cell_rows = []
    header_style = ParagraphStyle(
        "TableHeaderCell",
        parent=STYLES["body_no_indent"],
        fontName="Times-Bold",
        fontSize=10.8,
        leading=13.5,
        textColor=colors.white,
    )
    for row_index, row in enumerate(rows):
        cell_rows.append(
            [
                Paragraph(
                    cell,
                    header_style if row_index == 0 else STYLES["body_no_indent"],
                )
                if cell
                else Paragraph("", header_style if row_index == 0 else STYLES["body_no_indent"])
                for cell in row
            ]
        )
    table = Table(cell_rows, repeatRows=1, hAlign="LEFT")
    table.setStyle(
        TableStyle(
            [
                ("BACKGROUND", (0, 0), (-1, 0), TABLE_HEADER_BG),
                ("TEXTCOLOR", (0, 0), (-1, 0), colors.white),
                ("FONTNAME", (0, 0), (-1, 0), "Times-Bold"),
                ("FONTSIZE", (0, 0), (-1, -1), 10.8),
                ("LEADING", (0, 0), (-1, -1), 13.5),
                ("ROWBACKGROUNDS", (0, 1), (-1, -1), [colors.white, TABLE_ROW_ALT]),
                ("TOPPADDING", (0, 0), (-1, -1), 7),
                ("BOTTOMPADDING", (0, 0), (-1, -1), 7),
                ("LEFTPADDING", (0, 0), (-1, -1), 8),
                ("RIGHTPADDING", (0, 0), (-1, -1), 8),
                ("LINEBELOW", (0, 0), (-1, 0), 0.75, colors.black),
                ("LINEBELOW", (0, -1), (-1, -1), 0.75, colors.black),
            ]
        )
    )
    return table


def add_arrow(drawing: Drawing, x1: float, y1: float, x2: float, y2: float):
    drawing.add(Line(x1, y1, x2, y2, strokeColor=ARROW_STROKE, strokeWidth=1.3))
    angle_x = x2 - x1
    angle_y = y2 - y1
    if abs(angle_x) >= abs(angle_y):
        sign = 1 if angle_x >= 0 else -1
        drawing.add(
            Polygon(
                [
                    x2,
                    y2,
                    x2 - 8 * sign,
                    y2 + 4,
                    x2 - 8 * sign,
                    y2 - 4,
                ],
                fillColor=ARROW_STROKE,
                strokeColor=ARROW_STROKE,
            )
        )
    else:
        sign = 1 if angle_y >= 0 else -1
        drawing.add(
            Polygon(
                [
                    x2,
                    y2,
                    x2 - 4,
                    y2 - 8 * sign,
                    x2 + 4,
                    y2 - 8 * sign,
                ],
                fillColor=ARROW_STROKE,
                strokeColor=ARROW_STROKE,
            )
        )


def add_box(drawing: Drawing, x: float, y: float, w: float, h: float, text: str):
    drawing.add(
        Rect(
            x,
            y,
            w,
            h,
            rx=8,
            ry=8,
            fillColor=BOX_FILL,
            strokeColor=BOX_STROKE,
            strokeWidth=1.1,
        )
    )
    lines = text.split("\n")
    base_y = y + h / 2 + ((len(lines) - 1) * 6)
    for index, line in enumerate(lines):
        drawing.add(
            String(
                x + w / 2,
                base_y - (index * 12),
                line,
                fontName="Times-Roman",
                fontSize=10,
                fillColor=colors.black,
                textAnchor="middle",
            )
        )


def fig_system_overview():
    d = Drawing(450, 180)
    top = [
        ("Wallet", 12),
        ("Transaction\nBuilder", 90),
        ("Falcon-512\nSigning", 168),
        ("Mempool\nAdmission", 246),
        ("Miner Block\nCandidate", 324),
    ]
    for label, x in top:
        add_box(d, x, 116, 70, 36, label)
    add_box(d, 192, 28, 94, 42, "Consensus\nValidation")
    add_box(d, 322, 24, 108, 48, "Block and UTXO\nStorage")
    add_box(d, 54, 24, 96, 48, "Wallet Sync")
    add_box(d, 170, 24, 96, 48, "Explorer\nand API")
    for (_, x1), (_, x2) in zip(top, top[1:]):
        add_arrow(d, x1 + 70, 134, x2, 134)
    add_arrow(d, 359, 116, 239, 70)
    add_arrow(d, 286, 47, 322, 47)
    add_arrow(d, 170, 47, 150, 47)
    return d


def fig_tx_signing():
    d = Drawing(450, 120)
    labels = [
        "Select\nUTXOs",
        "Canonical\nTx Body",
        "SHA3-384\nDigest",
        "Falcon-512\nSign",
        "Broadcast",
        "Node\nRebuilds",
        "Verify /\nReject",
    ]
    x = 8
    for index, label in enumerate(labels):
        add_box(d, x, 42, 56, 34, label)
        if index < len(labels) - 1:
            add_arrow(d, x + 56, 59, x + 68, 59)
        x += 64
    return d


def fig_block_validation():
    d = Drawing(450, 210)
    labels = [
        ("Raw Block\nBytes", 162),
        ("Header, Size,\nNetwork, Target", 126),
        ("Tx Decode and\nStructure Checks", 90),
        ("UTXO, Witness,\nFalcon Verification", 54),
        ("Coinbase, Fees,\nMerkle Commitments", 18),
    ]
    for label, y in labels:
        add_box(d, 132, y, 186, 28, label)
    for (_, y1), (_, y2) in zip(labels, labels[1:]):
        add_arrow(d, 225, y1, 225, y2 + 28)
    add_box(d, 20, 18, 86, 28, "Reject")
    add_box(d, 344, 18, 86, 28, "Atomic Commit")
    add_arrow(d, 132, 32, 106, 32)
    add_arrow(d, 318, 32, 344, 32)
    return d


def fig_emission_model():
    d = Drawing(450, 190)
    d.add(Line(44, 34, 430, 34, strokeColor=colors.black, strokeWidth=1.2))
    d.add(Line(44, 34, 44, 162, strokeColor=colors.black, strokeWidth=1.2))
    d.add(String(20, 150, "Reward", fontName="Times-Roman", fontSize=10))
    d.add(String(360, 12, "Block Height", fontName="Times-Roman", fontSize=10))
    levels = [(5, 144), (2.5, 108), (1.25, 72), (0.625, 48)]
    for reward, y in levels:
        d.add(Line(40, y, 48, y, strokeColor=colors.black, strokeWidth=1))
        d.add(String(16, y - 3, str(reward), fontName="Times-Roman", fontSize=9))
    heights = [
        ("0", 52),
        ("1,260,000", 155),
        ("2,520,000", 258),
        ("3,780,000", 361),
        ("Tail", 411),
    ]
    for label, x in heights:
        d.add(Line(x, 30, x, 38, strokeColor=colors.black, strokeWidth=1))
        d.add(String(x, 18, label, fontName="Times-Roman", fontSize=8.5, textAnchor="middle"))
    points = [(52, 144), (155, 144), (155, 108), (258, 108), (258, 72), (361, 72), (361, 48), (420, 48)]
    for idx in range(len(points) - 1):
        x1, y1 = points[idx]
        x2, y2 = points[idx + 1]
        d.add(Line(x1, y1, x2, y2, strokeColor=colors.black, strokeWidth=2.2))
    d.add(
        String(
            225,
            170,
            "5 -> 2.5 -> 1.25 -> 0.625 ATHO tail emission",
            fontName="Times-Italic",
            fontSize=11,
            textAnchor="middle",
        )
    )
    return d


def fig_wallet_node():
    d = Drawing(450, 150)
    add_box(d, 28, 76, 88, 36, "Mnemonic /\nSeed")
    add_box(d, 154, 76, 90, 36, "Wallet")
    add_box(d, 292, 76, 126, 36, "Local Node / RPC")
    add_box(d, 86, 18, 102, 34, "Addresses and\nBalance")
    add_box(d, 252, 18, 116, 34, "Broadcast,\nValidation, Status")
    add_arrow(d, 116, 94, 154, 94)
    add_arrow(d, 244, 94, 292, 94)
    add_arrow(d, 199, 76, 148, 52)
    add_arrow(d, 328, 76, 310, 52)
    add_arrow(d, 292, 94, 244, 94)
    return d


def fig_parallel_validation():
    d = Drawing(450, 165)
    add_box(d, 18, 104, 82, 34, "Decode")
    add_box(d, 120, 104, 96, 34, "Structural\nChecks")
    add_box(d, 236, 104, 96, 34, "Batch UTXO\nReads")
    add_box(d, 352, 104, 82, 34, "Join")
    add_box(d, 320, 20, 110, 36, "Atomic Apply\nand Commit")
    for x in [248, 286, 324]:
        add_box(d, x, 58, 52, 24, "Worker")
    add_arrow(d, 100, 121, 120, 121)
    add_arrow(d, 216, 121, 236, 121)
    add_arrow(d, 332, 121, 352, 121)
    add_arrow(d, 284, 104, 274, 82)
    add_arrow(d, 284, 104, 312, 82)
    add_arrow(d, 284, 104, 350, 82)
    add_arrow(d, 274, 58, 360, 104)
    add_arrow(d, 312, 58, 384, 104)
    add_arrow(d, 350, 58, 408, 104)
    add_arrow(d, 393, 104, 375, 56)
    return d


FIGURE_DRAWINGS = {
    "4.1. System Flow": fig_system_overview,
    "8. Transaction Security": fig_tx_signing,
    "6. Emission Model and 150-Year Supply Projection": fig_emission_model,
    "7.1. Validation Pipeline": fig_block_validation,
    "10. Network Participants": fig_wallet_node,
    "7. Consensus and Validation": fig_parallel_validation,
}


def figure_flowables(spec: FigureSpec):
    drawing = FIGURE_DRAWINGS[spec.heading]()
    caption = Paragraph(f"{spec.label} {spec.title}", STYLES["figure_caption"])
    return [drawing, caption, Spacer(1, 0.04 * inch)]


def clean_heading_display(title: str) -> str:
    if title == "1. Abstract":
        return "Abstract"
    return title


def render_html_section(
    markdown_text: str,
    heading_context: str,
    table_caption_map: dict[str, str],
    skip_figure_pre_blocks: set[str],
):
    html_fragment = markdown_to_html_fragment(markdown_text)
    soup = BeautifulSoup(html_fragment, "lxml")
    root = soup.body if soup.body else soup
    flowables = []
    current_subheading = ""
    inserted_context_figures: set[str] = set()
    for node in root.children:
        if isinstance(node, NavigableString):
            if not str(node).strip():
                continue
            flowables.append(Paragraph(html.escape(str(node).strip()), STYLES["body"]))
            continue
        if not isinstance(node, Tag):
            continue
        if node.name == "h3":
            current_subheading = node.get_text(" ", strip=True)
            flowables.append(Paragraph(current_subheading, STYLES["subheading"]))
            if current_subheading in FIGURE_DRAWINGS and current_subheading not in inserted_context_figures:
                spec = next(item for item in WHITEPAPER_FIGURES if item.heading == current_subheading)
                flowables.extend(figure_flowables(spec))
                inserted_context_figures.add(current_subheading)
            continue
        if node.name == "p":
            text = node.get_text(" ", strip=True)
            if re.fullmatch(r"Table\s+\d+", text):
                continue
            if text.startswith("Atho 150-Year Monetary Supply Projection Under the Current Emission Policy"):
                continue
            if current_subheading == "" and heading_context in FIGURE_DRAWINGS and heading_context not in inserted_context_figures:
                spec = next(item for item in WHITEPAPER_FIGURES if item.heading == heading_context)
                flowables.extend(figure_flowables(spec))
                inserted_context_figures.add(heading_context)
            flowables.append(paragraph_from_tag(node, STYLES["body"]))
            continue
        if node.name in {"ul", "ol"}:
            if current_subheading == "" and heading_context in FIGURE_DRAWINGS and heading_context not in inserted_context_figures:
                spec = next(item for item in WHITEPAPER_FIGURES if item.heading == heading_context)
                flowables.extend(figure_flowables(spec))
                inserted_context_figures.add(heading_context)
            flowables.append(list_flowable(node))
            flowables.append(Spacer(1, 0.05 * inch))
            continue
        if node.name == "table":
            caption = table_caption_map.get(heading_context)
            parts = []
            if caption:
                number, _, title = caption.partition(". ")
                parts.append(Paragraph(number, STYLES["caption_label"]))
                parts.append(Paragraph(title, STYLES["caption_title"]))
            rows = []
            for tr in node.find_all("tr"):
                row = []
                for cell in tr.find_all(["th", "td"]):
                    row.append("".join(html_markup(child) for child in cell.children))
                if row:
                    rows.append(row)
            parts.append(table_flowable(rows))
            flowables.append(KeepTogether(parts))
            flowables.append(Spacer(1, 0.08 * inch))
            continue
        if node.name == "pre":
            if current_subheading in skip_figure_pre_blocks:
                continue
            text = node.get_text()
            flowables.append(Preformatted(text.rstrip(), STYLES["pre"]))
            continue
        if node.name == "blockquote":
            flowables.append(paragraph_from_tag(node, STYLES["body_no_indent"]))
            continue
    return flowables


def build_whitepaper():
    markdown_body = load_document_body(WHITEPAPER_SOURCE)
    sections = split_h2_sections(markdown_body)
    section_map = {title: body for title, body in sections}

    doc = AthoDocTemplate(str(WHITEPAPER_PDF), "ATHO WHITE PAPER")
    toc = TableOfContents()
    toc.levelStyles = [STYLES["toc_entry"]]

    story = []
    story.extend(
        title_page(
            "Atho: A Post-Quantum Proof-of-Work Payment Network for the Quantum Age",
            "Digital Platinum for Quantum-Secure Public Settlement",
            "Project: Atho",
        )
    )

    story.append(TocHeading("Abstract", STYLES["paper_heading"], 0))
    story.extend(
        render_html_section(
            section_map["1. Abstract"],
            "1. Abstract",
            {},
            set(),
        )
    )
    story.append(
        Paragraph(f"Keywords: {KEYWORDS}", STYLES["body_no_indent"])
    )
    story.append(PageBreak())

    story.append(Paragraph("Table of Contents", STYLES["toc_heading"]))
    story.append(toc)
    story.append(PageBreak())

    story.append(Paragraph("List of Figures", STYLES["toc_heading"]))
    for figure in WHITEPAPER_FIGURES:
        story.append(
            Paragraph(f"{figure.label} {figure.title}", STYLES["list_page_entry"])
        )
    story.append(PageBreak())

    story.append(Paragraph("List of Tables", STYLES["toc_heading"]))
    for table in WHITEPAPER_TABLES:
        story.append(Paragraph(table, STYLES["list_page_entry"]))
    story.append(PageBreak())

    story.append(TocHeading("Executive Summary", STYLES["paper_heading"], 0))
    story.append(
        Paragraph(
            (
                "Atho is a proof-of-work payment network built for deterministic validation, "
                "post-quantum-aware transaction authorization, and operator-friendly reviewability. "
                "The current repository code is the authority for this document."
            ),
            STYLES["body"],
        )
    )
    story.extend(
        render_html_section(
            section_map["Code-Grounded Policy Note"],
            "Code-Grounded Policy Note",
            {},
            set(),
        )
    )
    story.append(
        Paragraph(
            (
                "In the active implementation, Atho enforces a 100-second target block time, "
                "6 standard confirmations, 100-block coinbase maturity, a 5 ATHO opening subsidy, "
                "a 1,260,000-block halving interval, and a 0.625 ATHO tail reward. The same code "
                "path binds canonical ownership locks to Falcon-512 witnesses and rejects malformed "
                "or legacy transaction forms."
            ),
            STYLES["body"],
        )
    )

    table_caption_map = {
        "4. Technical Overview": WHITEPAPER_TABLES[0],
        "6. Emission Model and 150-Year Supply Projection": WHITEPAPER_TABLES[1],
        "9. Post-Quantum Security Comparison": WHITEPAPER_TABLES[2],
    }
    skip_figure_pre = {"4.1. System Flow", "7.1. Validation Pipeline"}

    for title, body in sections:
        if title in {"Code-Grounded Policy Note", "1. Abstract"}:
            continue
        story.append(TocHeading(clean_heading_display(title), STYLES["paper_heading"], 0))
        story.extend(
            render_html_section(
                body,
                title,
                table_caption_map,
                skip_figure_pre,
            )
        )

    doc.multiBuild(story)


def build_supplement():
    markdown_body = load_document_body(SUPPLEMENT_SOURCE)
    sections = split_h2_sections(markdown_body)
    section_map = {title: body for title, body in sections}

    doc = AthoDocTemplate(
        str(SUPPLEMENT_PDF), "ATHO MONETARY POLICY AND 150-YEAR SUPPLY SCHEDULE"
    )
    story = []
    story.extend(
        title_page(
            "Atho Monetary Policy and 150-Year Supply Schedule",
            "Code-Grounded Monetary Reference for the Current Network",
            "Project: Atho",
        )
    )

    supplement_table_map = {
        "2. Network Monetary Parameters": SUPPLEMENT_TABLES[0],
        "4. 150-Year Supply Schedule": SUPPLEMENT_TABLES[1],
    }

    for title, body in sections:
        story.append(TocHeading(clean_heading_display(title), STYLES["paper_heading"], 0))
        story.extend(
            render_html_section(
                body,
                title,
                supplement_table_map,
                set(),
            )
        )
        if title == "3. Emission Formula":
            story.extend(
                [
                    fig_emission_model(),
                    Paragraph(
                        "Figure 1. Current subsidy path under 100-second blocks and live consensus constants.",
                        STYLES["figure_caption"],
                    ),
                ]
            )

    doc.multiBuild(story)


def main():
    build_whitepaper()
    print(f"rendered {WHITEPAPER_PDF.relative_to(ROOT)}")
    build_supplement()
    print(f"rendered {SUPPLEMENT_PDF.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
