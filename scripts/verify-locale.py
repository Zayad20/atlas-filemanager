#!/usr/bin/env python3
"""
Verify locale file integrity against en.yml.
Usage:
    python scripts/verify-locale.py <locale_code>
    python scripts/verify-locale.py --all
"""

import sys
import os
import yaml

LOCALES_DIR = os.path.join(os.path.dirname(__file__), "..", "crates", "atlas-ui", "locales")
EN_FILE = os.path.join(LOCALES_DIR, "en.yml")

def load_yaml(path):
    with open(path, "r", encoding="utf-8") as f:
        return yaml.safe_load(f)

def verify_locale(locale_code):
    filename = f"{locale_code}.yml"
    locale_path = os.path.join(LOCALES_DIR, filename)

    if not os.path.exists(locale_path):
        print(f"[-] File not found: {locale_path}")
        return False

    try:
        data = load_yaml(locale_path)
    except Exception as e:
        print(f"[-] YAML syntax error in {filename}: {e}")
        return False

    if not isinstance(data, dict):
        print(f"[-] {filename} does not contain a top-level dictionary")
        return False

    en_data = load_yaml(EN_FILE)
    en_keys = set(en_data.keys())
    loc_keys = set(data.keys())

    missing = en_keys - loc_keys
    extra = loc_keys - en_keys
    empty = [k for k, v in data.items() if v is None or (isinstance(v, str) and not v.strip())]

    print(f"=== Locale Report: {locale_code} ===")
    print(f"File: {filename}")
    print(f"Total keys: {len(loc_keys)} / {len(en_keys)}")
    print(f"Missing keys: {len(missing)}")
    if missing:
        sample_missing = list(missing)[:10]
        print(f"  Sample missing: {sample_missing}")
    print(f"Extra keys: {len(extra)}")
    if extra:
        sample_extra = list(extra)[:10]
        print(f"  Sample extra: {sample_extra}")
    print(f"Empty values: {len(empty)}")
    if empty:
        print(f"  Empty keys: {empty[:10]}")

    is_ok = len(missing) == 0 and len(empty) == 0
    if is_ok:
        print(f"[+] {locale_code} is 100% COMPLETE and valid!")
    else:
        print(f"[!] {locale_code} has {len(missing)} missing and {len(empty)} empty keys.")
    return is_ok

def main():
    if len(sys.argv) < 2:
        print("Usage: python scripts/verify-locale.py <locale_code | --all>")
        sys.exit(1)

    arg = sys.argv[1]
    if arg == "--all":
        files = [f for f in os.listdir(LOCALES_DIR) if f.endswith(".yml")]
        all_ok = True
        for f in files:
            loc = f[:-4]
            if not verify_locale(loc):
                all_ok = False
            print("-" * 40)
        sys.exit(0 if all_ok else 1)
    else:
        ok = verify_locale(arg)
        sys.exit(0 if ok else 1)

if __name__ == "__main__":
    main()
