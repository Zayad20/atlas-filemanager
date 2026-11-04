"""Independently inspect artifacts from ATLAS_PAGE_TEST_ARTIFACTS and the native catalog.

Run the atlas-core tests with that variable pointing to artifacts/office-pdf-validation,
then build the conversion_probe example and run this script. Pillow and pypdf are
validation readers only; Atlas does not use or distribute them.
"""
import hashlib
from html.parser import HTMLParser
import io
import json
from pathlib import Path
import subprocess
import xml.etree.ElementTree as ET
import zipfile

from PIL import Image
from pypdf import PdfReader

ROOT = Path(__file__).resolve().parents[1]
ARTIFACTS = ROOT / "artifacts" / "office-pdf-validation"
WORD = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"


def main():
    executable = ROOT / "target" / "debug" / "examples" / "conversion_probe.exe"
    matrix = json.loads(subprocess.check_output([str(executable), "--matrix"], encoding="utf-8"))
    for row in matrix["inputs"]:
        for route in row["routes"]:
            assert set(route["engines"]) <= {"Atlas", "Windows", "Windows OCR", "Opus"}
            assert set(route["missing"]) <= {"Opus"}
    (ARTIFACTS / "native-routes.json").write_text(json.dumps(matrix, indent=2) + "\n", encoding="utf-8")
    image_results = []
    for extension in ["png", "jpg"]:
        for maximum in [0, 120]:
            folder = ARTIFACTS / f"pdf-{extension}-{maximum}"
            pdf = PdfReader(folder / "original.pdf")
            assert len(pdf.pages) == 2
            paths = sorted(folder.glob(f"*.{extension}"))
            assert len(paths) == 2
            for index, path in enumerate(paths):
                with Image.open(path) as image:
                    image.verify()
                with Image.open(path) as image:
                    expected = (300, 600) if index == 0 else (600, 300)
                    if maximum:
                        expected = tuple(value // 5 for value in expected)
                    assert image.size == expected, (path, image.size)
                    density = image.info["dpi"]
                    dpi = 60 if maximum else 300
                    assert all(abs(value - dpi) < 0.02 for value in density), (path, density)
                    color = image.convert("RGB").getpixel((expected[0] // 2, expected[1] // 2))
                    assert color[index * 2] > 245 and sum(color) < 265, (path, color)
                    image_results.append({"path": path.relative_to(ROOT).as_posix(), "size": image.size, "dpi": density})
    text = (ARTIFACTS / "ocr.txt").read_text(encoding="utf-8")
    assert [page.strip() for page in text.split("\f")] == ["First page", "Second page"]
    assert (ARTIFACTS / "ocr.md").read_text(encoding="utf-8") == text
    class PageParser(HTMLParser):
        def __init__(self):
            super().__init__()
            self.pages = 0

        def handle_starttag(self, tag, attrs):
            if tag == "section":
                self.pages += 1
    parser = PageParser()
    parser.feed((ARTIFACTS / "ocr.html").read_text(encoding="utf-8"))
    assert parser.pages == 2
    with zipfile.ZipFile(ARTIFACTS / "ocr.docx") as docx:
        document = ET.fromstring(docx.read("word/document.xml"))
        assert len(document.findall(f".//{WORD}br[@{WORD}type='page']")) == 1
    scan = PdfReader(ARTIFACTS / "scan.pdf")
    assert len(scan.pages) == 1
    assert not scan.pages[0].extract_text().strip()
    assert "First page" in (ARTIFACTS / "scan.txt").read_text(encoding="utf-8")
    packages = []
    for extension in ["docm", "dotx"]:
        original = ARTIFACTS / "office" / f"original.{extension}"
        output = ARTIFACTS / "office" / f"from-{extension}.docx"
        with zipfile.ZipFile(original) as before, zipfile.ZipFile(output) as after:
            assert after.testzip() is None
            assert not any("vba" in name.lower() or name.startswith("_xmlsignatures/") for name in after.namelist())
            for name in after.namelist():
                if name.endswith(".xml") or name.endswith(".rels"):
                    ET.fromstring(after.read(name))
                if name not in ["[Content_Types].xml", "word/_rels/document.xml.rels"]:
                    assert before.read(name) == after.read(name), name
            body = ET.fromstring(after.read("word/document.xml"))
            assert body.find(f".//{WORD}tbl") is not None
            assert body.find(f".//{WORD}drawing") is not None
            assert body.find(f".//{WORD}headerReference") is not None
            with Image.open(io.BytesIO(after.read("word/media/image.png"))) as image:
                image.verify()
            packages.append({"input": extension, "output": "docx", "preserved": ["document", "styles", "table", "image", "header", "page_geometry"], "removed": ["macros", "digital_signatures"], "sha256": hashlib.sha256(output.read_bytes()).hexdigest()})
    report = {
        "policy": "Atlas + Windows + optional Opus only",
        "catalog_route_count": sum(len(row["routes"]) for row in matrix["inputs"]),
        "catalog_is_not_an_exhaustive_conversion_test": True,
        "images": image_results,
        "ocr": {"pages": 2, "outputs": ["txt", "md", "html", "docx"], "recognized": text, "docx_page_breaks": 1, "html_page_sections": parser.pages, "image_only_pdf_verified": True},
        "office": packages,
        "not_implemented": ["faithful editable PDF layout", "Pages", "RTFD", "textClipping", "WMA/WMV encoding", "animated GIF to video", "PDF to PPTX/XLSX/EPUB", "Office/Markdown/RTF/TXT to page images"],
    }
    (ARTIFACTS / "verification.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"independent_verification": "passed", "image_files": len(image_results), "word_packages": len(packages), "ocr_pages": 2, "catalog_routes": report["catalog_route_count"]}))


if __name__ == "__main__":
    main()
