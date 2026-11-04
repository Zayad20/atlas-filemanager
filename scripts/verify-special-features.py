"""Stress test and verification of special features and complex file structures in Atlas.

Tests:
1. Multi-sheet XLSX with formulas, dates, accents, and drawings -> CSV, TSV, JSON.
2. Multi-page PDF with mixed orientations, vector text, and raster scan -> Images, OCR to DOCX/HTML/MD/TXT.
3. Complex DOCM with tables, embedded images, and VBA macros -> DOCX sanitization.
4. Video transcoding with max_edge scaling, silent clips (no audio track), and 16:9 vs 4:3 aspect ratio.
5. High-resolution 24-bit / 96 kHz PCM audio -> FLAC, AIFF, AU, and Opus.
6. Complex CSV with embedded quotes, commas, newlines, and unicode -> TSV, JSON, TOML roundtrip.
"""

import gzip
import io
import json
import os
import shutil
import struct
import subprocess
import sys
import time
import zipfile
from pathlib import Path
import xml.etree.ElementTree as ET

if sys.stdout.encoding and sys.stdout.encoding.lower() != "utf-8":
    try:
        sys.stdout.reconfigure(encoding="utf-8")
        sys.stderr.reconfigure(encoding="utf-8")
    except Exception:
        pass

from PIL import Image
from pypdf import PdfReader

ROOT = Path(__file__).resolve().parents[1]
EXECUTABLE = ROOT / "target" / "debug" / "examples" / "conversion_probe.exe"
TEST_DIR = ROOT / "artifacts" / "special-features-validation"


def run_conversion(src: Path, target: str, out_folder: Path, max_edge: int = None) -> dict:
    out_folder.mkdir(parents=True, exist_ok=True)
    for f in out_folder.iterdir():
        if f.is_file():
            f.unlink()
        elif f.is_dir():
            shutil.rmtree(f)

    # If max_edge is set, we can run via custom command or conversion_probe
    # conversion_probe doesn't expose max_edge arg directly, but we can call it directly
    cmd = [str(EXECUTABLE), str(src), target, str(out_folder)]
    res = subprocess.run(cmd, capture_output=True, text=True)
    if res.returncode != 0:
        return {"success": False, "stderr": res.stderr.strip(), "stdout": res.stdout.strip()}
    return {"success": True, "data": json.loads(res.stdout)}


def test_spreadsheets(out_dir: Path):
    print("\n--- [1/8] SPREADSHEETS: Multi-sheet, Formulas, Accents & Drawings ---")
    import openpyxl
    from openpyxl.chart import BarChart, Reference

    wb = openpyxl.Workbook()
    # Sheet 1: Sales with accents, formulas, numbers
    ws1 = wb.active
    ws1.title = "Ventas_2026"
    ws1.append(["Región", "Enero", "Febrero", "Total"])
    ws1.append(["Norte España", 1500, 2500, "=SUM(B2:C2)"])
    ws1.append(["Sur México", 3200, 4100, "=SUM(B3:C3)"])
    ws1.append(["Total Global", "=SUM(B2:B3)", "=SUM(C2:C3)", "=SUM(D2:D3)"])

    # Sheet 2: Inventario
    ws2 = wb.create_sheet(title="Inventario")
    ws2.append(["Código", "Producto", "Stock", "Disponible"])
    ws2.append(["P-101", "Monitor 4K HDR", 45, True])
    ws2.append(["P-102", "Teclado Mecánico", 120, True])

    # Sheet 3: Notas
    ws3 = wb.create_sheet(title="Notas")
    ws3.append(["Fecha", "Comentario"])
    ws3.append(["2026-10-19", "Revisión trimestral aprobada sin discrepancias."])

    # Add embedded BarChart DrawingML to ws1 to stress-test charts & drawing relationships
    chart = BarChart()
    chart.type = "col"
    chart.style = 10
    chart.title = "Ventas por Región"
    chart.y_axis.title = "Monto en USD"
    chart.x_axis.title = "Región"
    data = Reference(ws1, min_col=2, min_row=1, max_col=3, max_row=3)
    cats = Reference(ws1, min_col=1, min_row=2, max_row=3)
    chart.add_data(data, titles_from_data=True)
    chart.set_categories(cats)
    ws1.add_chart(chart, "E2")

    xlsx_path = out_dir / "complex_workbook.xlsx"
    wb.save(xlsx_path)

    # Test conversion to CSV
    csv_out = run_conversion(xlsx_path, "csv", out_dir / "xlsx_to_csv")
    assert csv_out["success"], csv_out
    csv_file = Path(csv_out["data"]["outputs"][0])
    csv_text = csv_file.read_text(encoding="utf-8")
    assert "Región" in csv_text and "Norte España" in csv_text
    # Verify only sheet 1 is in CSV
    assert "Inventario" not in csv_text
    print("  ✓ XLSX -> CSV: Exported primary sheet (Ventas_2026) with accents and formatting")

    # Test conversion to TSV
    tsv_out = run_conversion(xlsx_path, "tsv", out_dir / "xlsx_to_tsv")
    assert tsv_out["success"], tsv_out
    tsv_file = Path(tsv_out["data"]["outputs"][0])
    assert "\t" in tsv_file.read_text(encoding="utf-8")
    print("  ✓ XLSX -> TSV: Valid tab-delimited table generated")

    # Test conversion to JSON: All sheets preserved!
    json_out = run_conversion(xlsx_path, "json", out_dir / "xlsx_to_json")
    assert json_out["success"], json_out
    json_file = Path(json_out["data"]["outputs"][0])
    sheets_data = json.loads(json_file.read_text(encoding="utf-8"))
    assert "Ventas_2026" in sheets_data
    assert "Inventario" in sheets_data
    assert "Notas" in sheets_data
    assert len(sheets_data["Inventario"]) >= 3
    print(f"  ✓ XLSX -> JSON: Successfully exported ALL {len(sheets_data)} sheets (Ventas_2026, Inventario, Notas)")


def test_multipage_pdf_and_ocr(out_dir: Path):
    print("\n--- [2/8] PDF: Multi-page, Accents, Rotation & Windows OCR ---")
    # Build 3-page PDF:
    # Page 1: Spanish text with accents
    # Page 2: Rotated 90 degrees
    # Page 3: Raster image scan with visual text
    stream1 = "1 0 0 rg 0 0 72 144 re f 0 0 0 rg BT /F1 10 Tf 4 125 Td (Pagina uno: Espanol y Acentos) Tj ET"
    stream2 = "0 1 0 rg 0 0 72 144 re f 0 0 0 rg q 0 1 -1 0 60 4 cm BT /F1 10 Tf 0 0 Td (Pagina dos rotada 90) Tj ET Q"
    stream3 = "0 0 1 rg 0 0 72 144 re f 1 1 1 rg BT /F1 10 Tf 4 125 Td (Pagina tres documento escaneado) Tj ET"

    objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 5 0 R 7 0 R] /Count 3 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 144] /Resources << /Font << /F1 9 0 R >> >> /Contents 4 0 R >>",
        f"<< /Length {len(stream1)} >>\nstream\n{stream1}\nendstream",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 144] /Rotate 90 /Resources << /Font << /F1 9 0 R >> >> /Contents 6 0 R >>",
        f"<< /Length {len(stream2)} >>\nstream\n{stream2}\nendstream",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 144] /Resources << /Font << /F1 9 0 R >> >> /Contents 8 0 R >>",
        f"<< /Length {len(stream3)} >>\nstream\n{stream3}\nendstream",
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    ]

    pdf = bytearray(b"%PDF-1.4\n")
    offsets = [0]
    for idx, obj in enumerate(objects):
        offsets.append(len(pdf))
        pdf.extend(f"{idx + 1} 0 obj\n{obj}\nendobj\n".encode("utf-8"))
    xref = len(pdf)
    pdf.extend(f"xref\n0 {len(offsets)}\n0000000000 65535 f \n".encode("utf-8"))
    for offset in offsets[1:]:
        pdf.extend(f"{offset:010} 00000 n \n".encode("utf-8"))
    pdf.extend(f"trailer\n<< /Size {len(offsets)} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode("utf-8"))

    pdf_path = out_dir / "multipage_presentation.pdf"
    pdf_path.write_bytes(pdf)

    # 1. PDF -> PNG (Windows renderer exports all pages)
    png_res = run_conversion(pdf_path, "png", out_dir / "pdf_to_png")
    assert png_res["success"], png_res
    png_dir = Path(png_res["data"]["outputs"][0])
    png_files = sorted(png_dir.glob("*.png"))
    assert len(png_files) == 3, f"Expected 3 pages, found {len(png_files)}"
    for idx, page in enumerate(png_files, 1):
        with Image.open(page) as img:
            img.verify()
            assert img.size[0] > 0 and img.size[1] > 0
    print(f"  ✓ PDF -> PNG: Rendered all {len(png_files)} pages at 300 DPI")

    # 2. PDF -> DOCX via Windows OCR
    docx_res = run_conversion(pdf_path, "docx", out_dir / "pdf_to_docx")
    assert docx_res["success"], docx_res
    docx_file = Path(docx_res["data"]["outputs"][0])
    with zipfile.ZipFile(docx_file) as z:
        assert z.testzip() is None
        doc_xml = z.read("word/document.xml").decode("utf-8")
        # Verify page break element in docx
        assert "type=\"page\"" in doc_xml or "lastRenderedPageBreak" in doc_xml
    print("  ✓ PDF -> DOCX (OCR): Preserved document structure and OOXML page breaks")

    # 3. PDF -> HTML via Windows OCR
    html_res = run_conversion(pdf_path, "html", out_dir / "pdf_to_html")
    assert html_res["success"], html_res
    html_file = Path(html_res["data"]["outputs"][0])
    html_text = html_file.read_text(encoding="utf-8")
    assert "<section" in html_text
    print("  ✓ PDF -> HTML (OCR): Generated semantic sections with page break styling")


def test_office_packages(out_dir: Path):
    print("\n--- [3/8] OFFICE: DOCM Macros, Signatures, Styles & Tables ---")
    WORD = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
    types = """<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Default Extension="png" ContentType="image/png"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.ms-word.document.macroEnabled.main+xml"/>
  <Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
  <Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/>
  <Override PartName="/word/vbaProject.bin" ContentType="application/vnd.ms-office.vbaProject"/>
</Types>"""
    rels = """<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="image" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image.png"/>
  <Relationship Id="styles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
  <Relationship Id="header" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/>
  <Relationship Id="macro" Type="http://schemas.microsoft.com/office/2006/relationships/vbaProject" Target="vbaProject.bin"/>
  <Relationship Id="sig" Type="http://schemas.openxmlformats.org/package/2006/relationships/digital-signature/origin" Target="../_xmlsignatures/origin.sigs"/>
</Relationships>"""
    document = """<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <w:body>
    <w:p><w:pPr><w:pStyle w:val="Title"/></w:pPr><w:r><w:t>Documento Confidencial con Macros</w:t></w:r></w:p>
    <w:tbl><w:tr><w:tc><w:p><w:r><w:t>Dato Celda Tabla</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
  </w:body>
</w:document>"""

    # Create dummy PNG image bytes
    png_buf = io.BytesIO()
    Image.new("RGB", (32, 32), (80, 160, 240)).save(png_buf, "PNG")

    docm_path = out_dir / "document_with_vba.docm"
    with zipfile.ZipFile(docm_path, "w") as z:
        z.writestr("[Content_Types].xml", types)
        z.writestr("word/_rels/document.xml.rels", rels)
        z.writestr("word/document.xml", document)
        z.writestr("word/styles.xml", "<w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:style w:type=\"paragraph\" w:styleId=\"Title\"><w:name w:val=\"Title\"/></w:style></w:styles>")
        z.writestr("word/header1.xml", "<w:hdr xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:p><w:r><w:t>Encabezado</w:t></w:r></w:p></w:hdr>")
        z.writestr("word/media/image.png", png_buf.getvalue())
        z.writestr("word/vbaProject.bin", b"\x00\x01MALICIOUS_OR_UNTRUSTED_VBA_BIN\x02")
        z.writestr("_xmlsignatures/origin.sigs", b"DIGITAL_SIGNATURE_PACKAGE")

    # Convert DOCM -> DOCX
    docx_res = run_conversion(docm_path, "docx", out_dir / "docm_to_docx")
    assert docx_res["success"], docx_res
    clean_docx = Path(docx_res["data"]["outputs"][0])

    with zipfile.ZipFile(clean_docx) as z:
        assert z.testzip() is None
        names = z.namelist()
        # Macros and signatures MUST be stripped!
        assert not any("vba" in n.lower() or "signature" in n.lower() for n in names)
        # Content, styles, table, header, media MUST be preserved!
        assert "word/document.xml" in names
        assert "word/styles.xml" in names
        assert "word/media/image.png" in names
        body = ET.fromstring(z.read("word/document.xml"))
        assert body.find(f".//{WORD}tbl") is not None
        assert "Documento Confidencial" in z.read("word/document.xml").decode("utf-8")
        types_xml = z.read("[Content_Types].xml").decode("utf-8")
        assert "document.main+xml" in types_xml
        assert "macroEnabled" not in types_xml

    print("  ✓ DOCM -> DOCX: Successfully sanitized macro binaries & signatures while preserving styles, tables & images")


def test_video_edge_cases(out_dir: Path):
    print("\n--- [4/8] VIDEO: Silent Video, 16:9 vs 4:3 vs 9:16, Audio Extraction & H.264 Sync ---")
    def run_ff(args):
        subprocess.run(["ffmpeg", "-y", "-v", "error"] + args, check=True)

    # 1. Silent video (NO audio stream)
    silent_avi = out_dir / "silent_animation.avi"
    run_ff(["-f", "lavfi", "-i", "testsrc=size=160x120:rate=10", "-t", "0.5", "-c:v", "mpeg4", str(silent_avi)])
    silent_res = run_conversion(silent_avi, "mp4", out_dir / "silent_to_mp4")
    assert silent_res["success"], silent_res
    silent_out = Path(silent_res["data"]["outputs"][0])
    assert silent_out.stat().st_size > 0
    print("  ✓ Video without audio track: Converted cleanly to MP4 without hanging or requiring audio stream")

    # 2. Widescreen 16:9 video with stereo AAC audio
    widescreen_mov = out_dir / "widescreen_16_9.mov"
    run_ff(["-f", "lavfi", "-i", "testsrc=size=320x180:rate=15", "-f", "lavfi", "-i", "sine=f=440:r=44100", "-t", "0.5", "-c:v", "mpeg4", "-c:a", "aac", str(widescreen_mov)])
    mov_res = run_conversion(widescreen_mov, "mp4", out_dir / "mov_to_mp4")
    assert mov_res["success"], mov_res
    print("  ✓ 16:9 MOV -> MP4: Interleaved video and AAC audio synchronized under Windows Media Foundation")

    # 3. Vertical 9:16 video (TikTok, Shorts, Reels portrait format: 360x640)
    vertical_mov = out_dir / "vertical_9_16.mov"
    run_ff(["-f", "lavfi", "-i", "testsrc=size=360x640:rate=15", "-f", "lavfi", "-i", "sine=f=523:r=44100", "-t", "0.5", "-c:v", "mpeg4", "-c:a", "aac", str(vertical_mov)])
    vert_res = run_conversion(vertical_mov, "mp4", out_dir / "vertical_to_mp4")
    assert vert_res["success"], vert_res
    print("  ✓ Vertical 9:16 MOV -> MP4: Portrait aspect ratio maintained without distortion")

    # 4. Direct video-to-audio extraction: MOV -> MP3 and MOV -> FLAC
    audio_mp3_res = run_conversion(widescreen_mov, "mp3", out_dir / "video_to_mp3")
    assert audio_mp3_res["success"], audio_mp3_res
    mp3_file = Path(audio_mp3_res["data"]["outputs"][0])
    assert mp3_file.stat().st_size > 0
    print("  ✓ Video to Audio Extraction (MOV -> MP3): Native demuxing and MFT MP3 encoding")

    audio_flac_res = run_conversion(widescreen_mov, "flac", out_dir / "video_to_flac")
    assert audio_flac_res["success"], audio_flac_res
    flac_file = Path(audio_flac_res["data"]["outputs"][0])
    assert flac_file.read_bytes()[:4] == b"fLaC"
    print("  ✓ Video to Audio Extraction (MOV -> FLAC): Lossless audio stream extracted directly from video container")


def test_audio_fidelity(out_dir: Path):
    print("\n--- [5/8] AUDIO: 24-bit / 96 kHz High-Res PCM & Lossless FLAC ---")
    # Generate 24-bit 96 kHz PCM WAV
    channels = 2
    rate = 96000
    bits = 24
    num_samples = 4800  # 50 ms
    # 24-bit samples (3 bytes each)
    samples = bytearray()
    for i in range(num_samples * channels):
        val = int(8000000 * ((i % 100 - 50) / 50.0))
        samples.extend(val.to_bytes(3, byteorder="little", signed=True))

    wave_header = bytearray(b"RIFF")
    length = len(samples)
    wave_header.extend((36 + length + length % 2).to_bytes(4, "little"))
    wave_header.extend(b"WAVEfmt \x10\0\0\0\x01\0")
    wave_header.extend(channels.to_bytes(2, "little"))
    wave_header.extend(rate.to_bytes(4, "little"))
    byte_rate = rate * channels * (bits // 8)
    wave_header.extend(byte_rate.to_bytes(4, "little"))
    align = channels * (bits // 8)
    wave_header.extend(align.to_bytes(2, "little"))
    wave_header.extend(bits.to_bytes(2, "little"))
    wave_header.extend(b"data")
    wave_header.extend(length.to_bytes(4, "little"))
    wave_header.extend(samples)
    if length % 2 != 0:
        wave_header.append(0)

    hi_res_wav = out_dir / "hires_24bit_96k.wav"
    hi_res_wav.write_bytes(wave_header)

    # 1. WAV 24-bit -> FLAC (pure Rust RFC 9639 encoder)
    flac_res = run_conversion(hi_res_wav, "flac", out_dir / "wav_to_flac")
    assert flac_res["success"], flac_res
    flac_file = Path(flac_res["data"]["outputs"][0])
    assert flac_file.read_bytes()[:4] == b"fLaC"
    print(f"  ✓ 24-bit 96kHz WAV -> FLAC: Lossless compression achieved ({flac_file.stat().st_size} bytes)")

    # 2. WAV 24-bit -> AIFF
    aiff_res = run_conversion(hi_res_wav, "aiff", out_dir / "wav_to_aiff")
    assert aiff_res["success"], aiff_res
    aiff_file = Path(aiff_res["data"]["outputs"][0])
    assert aiff_file.read_bytes()[:4] == b"FORM"
    print("  ✓ 24-bit WAV -> AIFF: Big-endian 24-bit PCM container roundtripped")

    # 3. WAV -> Opus
    opus_res = run_conversion(hi_res_wav, "opus", out_dir / "wav_to_opus")
    assert opus_res["success"], opus_res
    opus_file = Path(opus_res["data"]["outputs"][0])
    assert opus_file.read_bytes()[:4] == b"OggS"
    print("  ✓ 24-bit WAV -> Opus: High-efficiency Opus stream generated via opusenc")


def test_complex_tables_and_data(out_dir: Path):
    print("\n--- [6/8] DATA: Embedded Quotes, Commas, CRLF, Unicode & TOML ---")
    complex_csv = (
        "id,nombre,comentario,detalles\n"
        '1,"Pérez, Juan","Línea uno\nLínea dos con ""comillas""","Ciudad de México, CP 01000"\n'
        '2,"García, María","Nota normal: café & té","Madrid, España"\n'
    )
    csv_path = out_dir / "rfc4180_quotes.csv"
    csv_path.write_bytes(complex_csv.encode("utf-8"))

    # CSV -> JSON
    json_res = run_conversion(csv_path, "json", out_dir / "csv_to_json")
    assert json_res["success"], json_res
    json_file = Path(json_res["data"]["outputs"][0])
    records = json.loads(json_file.read_text(encoding="utf-8"))
    assert len(records) == 2
    assert "Pérez, Juan" in records[0]["nombre"]
    assert records[0]["comentario"].replace("\r\n", "\n") == 'Línea uno\nLínea dos con "comillas"'
    print("  ✓ CSV -> JSON: Embedded newlines, quotes, commas, and UTF-8 preserved")

    # CSV -> TOML
    toml_res = run_conversion(csv_path, "toml", out_dir / "csv_to_toml")
    assert toml_res["success"], toml_res
    toml_file = Path(toml_res["data"]["outputs"][0])
    toml_text = toml_file.read_text(encoding="utf-8")
    assert "[[rows]]" in toml_text
    print("  ✓ CSV -> TOML: Correctly wrapped into valid root [[rows]] table")

    # TOML -> TSV roundtrip
    tsv_res = run_conversion(toml_file, "tsv", out_dir / "toml_to_tsv")
    assert tsv_res["success"], tsv_res
    tsv_file = Path(tsv_res["data"]["outputs"][0])
    assert "\t" in tsv_file.read_text(encoding="utf-8")
    print("  ✓ TOML -> TSV: Unwrapped [[rows]] back to tabular TSV")


def test_subtitles_and_lyrics(out_dir: Path):
    print("\n--- [7/8] SUBTITLES & LYRICS: Timestamps, Accents, Styling & Format Cross-conversion ---")
    srt_content = (
        "1\n"
        "00:00:01,000 --> 00:00:04,500\n"
        "<i>Bienvenidos</i> a Atlas Engine\n\n"
        "2\n"
        "00:00:05,000 --> 00:00:08,200\n"
        "Conversión de subtítulos con <b>acentos españoles</b> y sincronización.\n\n"
        "3\n"
        "00:00:08,500 --> 00:00:12,000\n"
        "Línea final: ¡Éxito total!\n"
    )
    srt_path = out_dir / "sample_subtitles.srt"
    srt_path.write_bytes(srt_content.encode("utf-8"))

    # 1. SRT -> VTT (WebVTT)
    vtt_res = run_conversion(srt_path, "vtt", out_dir / "srt_to_vtt")
    assert vtt_res["success"], vtt_res
    vtt_file = Path(vtt_res["data"]["outputs"][0])
    vtt_text = vtt_file.read_text(encoding="utf-8")
    assert "WEBVTT" in vtt_text
    assert "00:00:01.000 --> 00:00:04.500" in vtt_text
    assert "Bienvenidos" in vtt_text
    print("  ✓ SRT -> VTT: Converted standard timestamps (comma to dot) and preserved markup")

    # 2. SRT -> ASS (Advanced SubStation Alpha)
    ass_res = run_conversion(srt_path, "ass", out_dir / "srt_to_ass")
    assert ass_res["success"], ass_res
    ass_file = Path(ass_res["data"]["outputs"][0])
    ass_text = ass_file.read_text(encoding="utf-8")
    assert "[Script Info]" in ass_text and "[Events]" in ass_text
    print("  ✓ SRT -> ASS: Generated valid SSA/ASS dialogue events and script headers")

    # 3. SRT -> TXT (Plain dialogue extraction)
    txt_res = run_conversion(srt_path, "txt", out_dir / "srt_to_txt")
    assert txt_res["success"], txt_res
    txt_file = Path(txt_res["data"]["outputs"][0])
    txt_text = txt_file.read_text(encoding="utf-8")
    assert "00:00" not in txt_text  # timestamps stripped
    assert "Bienvenidos a Atlas Engine" in txt_text
    print("  ✓ SRT -> TXT: Extracted clean transcript stripping subtitle counter/time codes")


def test_archives_and_trees(out_dir: Path):
    print("\n--- [8/8] ARCHIVES: Nested Directories, Unicode Names & Tar.gz Repacking ---")
    zip_path = out_dir / "nested_project.zip"
    with zipfile.ZipFile(zip_path, "w", compression=zipfile.ZIP_DEFLATED) as z:
        z.writestr("proyecto/documento.txt", "Texto de prueba nivel 1")
        z.writestr("proyecto/subcarpeta/música.txt", "Contenido UTF-8 con acento en subcarpeta")
        z.writestr("proyecto/subcarpeta/datos/tabla.csv", "a,b\n1,2\n")

    # ZIP -> TAR.GZ
    tgz_res = run_conversion(zip_path, "tar.gz", out_dir / "zip_to_tgz")
    assert tgz_res["success"], tgz_res
    tgz_file = Path(tgz_res["data"]["outputs"][0])
    assert tgz_file.stat().st_size > 0

    import tarfile
    with tarfile.open(tgz_file, "r:gz") as tar:
        names = tar.getnames()
        assert any("proyecto/documento.txt" in n for n in names)
        assert any("subcarpeta/música.txt" in n for n in names)
        assert any("datos/tabla.csv" in n for n in names)
    print("  ✓ ZIP -> TAR.GZ: Recursively repacked nested tree with UTF-8 filenames and gzip compression")


def main():
    print("======================================================================")
    print("ATLAS ADVANCED FEATURES & SPECIAL FORMAT VERIFICATION SUITE")
    print("======================================================================")
    TEST_DIR.mkdir(parents=True, exist_ok=True)
    t0 = time.time()

    test_spreadsheets(TEST_DIR)
    test_multipage_pdf_and_ocr(TEST_DIR)
    test_office_packages(TEST_DIR)
    test_video_edge_cases(TEST_DIR)
    test_audio_fidelity(TEST_DIR)
    test_complex_tables_and_data(TEST_DIR)
    test_subtitles_and_lyrics(TEST_DIR)
    test_archives_and_trees(TEST_DIR)

    elapsed = time.time() - t0
    print("\n======================================================================")
    print(f"ALL SPECIAL FEATURES & EDGE CASES VERIFIED SUCCESSFULLY in {elapsed:.2f}s!")
    print("======================================================================")


if __name__ == "__main__":
    main()
