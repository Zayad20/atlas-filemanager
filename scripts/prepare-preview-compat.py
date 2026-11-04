"""Create small, real-format fixtures for the native preview and Miller checks."""
from pathlib import Path
import argparse
import json
import shutil
import subprocess
import zipfile


def package(path, parts):
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, content in parts.items():
            archive.writestr(name, content)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    folder = args.output.resolve() / "fixtures"
    folder.mkdir(parents=True, exist_ok=True)
    text = "café, niño, 日本語 y 🧑🏽‍💻 → texto válido\n"
    (folder / "unicode.txt").write_text(text, encoding="utf-8")
    (folder / "unicode.rs").write_text('fn main() { let café = "日本語 🧑🏽‍💻"; }\n', encoding="utf-8")
    (folder / "unicode.md").write_text('# Café 日本語\n\n**niño 🧑🏽‍💻**\n\n```rust\nlet café = "日本語";\n```\n', encoding="utf-8")
    (folder / "table.csv").write_text('nombre;nota\nniño;"primera línea\n日本語 y ""comillas"""\n', encoding="utf-8-sig")
    (folder / "utf16.csv").write_text('nombre;ciudad\nniño;México\n', encoding="utf-16")
    package(folder / "document.docx", {
        "[Content_Types].xml": '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>',
        "_rels/.rels": '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>',
        "word/document.xml": f'<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:body></w:document>',
    })
    package(folder / "workbook.xlsx", {
        "[Content_Types].xml": '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>',
        "_rels/.rels": '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>',
        "xl/workbook.xml": '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Datos" sheetId="1" r:id="rId1"/></sheets></workbook>',
        "xl/_rels/workbook.xml.rels": '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>',
        "xl/worksheets/sheet1.xml": '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><dimension ref="A1:B2"/><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>nombre</t></is></c><c r="B1" t="inlineStr"><is><t>precio</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>niño 日本語</t></is></c><c r="B2"><v>42</v></c></row></sheetData></worksheet>',
    })
    package(folder / "slides.pptx", {
        "[Content_Types].xml": '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/><Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/></Types>',
        "_rels/.rels": '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>',
        "ppt/presentation.xml": '<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst></p:presentation>',
        "ppt/_rels/presentation.xml.rels": '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/></Relationships>',
        "ppt/slides/slide1.xml": '<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:sp><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Café 日本語</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>',
    })
    flags = getattr(subprocess, "CREATE_NO_WINDOW", 0)
    magick = shutil.which("magick")
    if not magick:
        raise RuntimeError("ImageMagick is required for the PSD fixture")
    subprocess.run([magick, "-size", "64x48", "xc:#3070c0", str(folder / "image.psd")], check=True, capture_output=True, creationflags=flags)
    archive_files = folder / "archive-files"
    archive_files.mkdir(exist_ok=True)
    for name in ["unicode.txt", "unicode.md", "unicode.rs", "table.csv", "document.docx"]:
        shutil.copyfile(folder / name, archive_files / name)
    shutil.copyfile(folder / "unicode.txt", archive_files / "niño-日本語.txt")
    (archive_files / "Vacía").mkdir(exist_ok=True)
    with zipfile.ZipFile(folder / "bundle.zip", "w", zipfile.ZIP_DEFLATED) as archive:
        for path in archive_files.iterdir():
            archive.write(path, f"archive-files/{path.name}")
    rar = Path("C:/Program Files/WinRAR/Rar.exe")
    subprocess.run([str(rar), "a", "-idq", "-r", str(folder / "bundle.rar"), "archive-files"], cwd=folder, check=True, capture_output=True, creationflags=flags)
    for name in ["broken.docx", "broken.zip", "broken.rar", "broken.psd", "broken.xlsx", "broken.pptx"]:
        (folder / name).write_bytes(b"truncated or invalid file\x00\xff")
    (folder / "binary.txt").write_bytes(b"\xff\xfe\x00\x81")
    package(folder / "oversized.pptx", {"ppt/slides/slide1.xml": " " * (8 * 1024 * 1024 + 1)})
    # Overflowing columns, long titles and names, an empty column, and Unicode.
    nested = folder
    for i in range(4):
        nested = nested / f"Columna {i} con nombre muy largo y acentos"
        nested.mkdir(exist_ok=True)
        for row in range(120):
            (nested / f"{row:03} archivo con un nombre muy largo que debe recortarse niño 日本語.txt").write_text(text, encoding="utf-8")
    (nested / "Vacía con un título largo").mkdir(exist_ok=True)
    (args.output / "fixture-tools.json").write_text(json.dumps({"magick": magick, "rar": str(rar)}, indent=2), encoding="utf-8")
    print(f"Fixtures ready in {folder}")


if __name__ == "__main__":
    main()
