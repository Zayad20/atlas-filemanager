"""Measure a Windows release without loading it or using third-party tools."""
import argparse
import hashlib
import json
from pathlib import Path
import struct


def imports(data):
    pe = struct.unpack_from("<I", data, 60)[0]
    if data[pe:pe + 4] != b"PE\0\0":
        raise ValueError("Expected a Windows PE image")
    count = struct.unpack_from("<H", data, pe + 6)[0]
    optional_size = struct.unpack_from("<H", data, pe + 20)[0]
    optional = pe + 24
    if struct.unpack_from("<H", data, optional)[0] != 0x20B:
        raise ValueError("Expected a 64-bit PE image")
    sections = [struct.unpack_from("<8sIIIIIIHHI", data, optional + optional_size + i * 40)
                for i in range(count)]

    def offset(rva):
        for section in sections:
            if section[2] <= rva < section[2] + max(section[1], section[3]):
                return section[4] + rva - section[2]
        raise ValueError(f"Unmapped image RVA: {rva}")

    rva = struct.unpack_from("<I", data, optional + 120)[0]
    if not rva:
        return []
    position = offset(rva)
    names = []
    for _ in range(4096):
        descriptor = struct.unpack_from("<IIIII", data, position)
        if not any(descriptor):
            return names
        name = offset(descriptor[3])
        names.append(data[name:data.index(b"\0", name)].decode("ascii"))
        position += 20
    raise ValueError("Unterminated import directory")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("executable", type=Path)
    parser.add_argument("--baseline-bytes", type=int, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    data = args.executable.read_bytes()
    dlls = imports(data)
    report = {
        "executable": args.executable.as_posix(),
        "profile": "release (opt-level=3, thin LTO, codegen-units=1, strip)",
        "baseline_bytes": args.baseline_bytes,
        "bytes": len(data),
        "added_bytes": len(data) - args.baseline_bytes,
        "added_kib": (len(data) - args.baseline_bytes) / 1024,
        "added_percent": (len(data) - args.baseline_bytes) * 100 / args.baseline_bytes,
        "sha256": hashlib.sha256(data).hexdigest(),
        "imported_dlls": dlls,
        "eager_media_foundation_imports": [name for name in dlls if name.lower().startswith("mf")],
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
