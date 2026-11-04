#!/usr/bin/env python3
"""
Build and verify zh-Hant.yml and zh-TW.yml from modular translation tables.
"""

import json
import os
import re
import sys
import yaml

# Add scripts directory to path
SCRIPTS_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, SCRIPTS_DIR)

from i18n_zh_hant.cmd import cmd_translations
from i18n_zh_hant.cleanup import cleanup_translations
from i18n_zh_hant.convert import convert_translations
from i18n_zh_hant.vault import vault_translations
from i18n_zh_hant.guide import guide_translations
from i18n_zh_hant.settings_palette import settings_palette_translations
from i18n_zh_hant.cloud_sidebar import cloud_sidebar_translations
from i18n_zh_hant.inspector_properties_op_batch import inspector_properties_op_batch_translations
from i18n_zh_hant.group_getting_overlay_root import group_getting_overlay_root_translations
from i18n_zh_hant.ext_tag_tooltip_theme import ext_tag_tooltip_theme_translations
from i18n_zh_hant.misc import misc_translations

LOCALES_DIR = os.path.join(SCRIPTS_DIR, "..", "crates", "atlas-ui", "locales")
REF_FILE = os.path.join(LOCALES_DIR, "reference_keys.json")
EN_FILE = os.path.join(LOCALES_DIR, "en.yml")

def assemble_translations():
    combined = {}
    modules = [
        cmd_translations,
        cleanup_translations,
        convert_translations,
        vault_translations,
        guide_translations,
        settings_palette_translations,
        cloud_sidebar_translations,
        inspector_properties_op_batch_translations,
        group_getting_overlay_root_translations,
        ext_tag_tooltip_theme_translations,
        misc_translations,
    ]
    for mod in modules:
        for k, v in mod.items():
            if k in combined:
                raise ValueError(f"Duplicate key in modules: {k}")
            combined[k] = v
    return combined

def validate(combined, ref_data):
    ref_dict = {item["key"]: item for item in ref_data}
    combined_keys = set(combined.keys())
    ref_keys = set(ref_dict.keys())

    missing = ref_keys - combined_keys
    extra = combined_keys - ref_keys

    if missing:
        print(f"[-] Missing {len(missing)} keys: {list(missing)[:10]}")
    if extra:
        print(f"[-] Extra {len(extra)} keys: {list(extra)[:10]}")

    if missing or extra:
        sys.exit(1)

    print(f"[+] Key counts match perfectly: {len(combined)} / {len(ref_data)}")

    # Check empty values
    empty = [k for k, v in combined.items() if not v or not v.strip()]
    if empty:
        print(f"[-] Empty values for keys: {empty}")
        sys.exit(1)

    # Check placeholders %{...}
    ph_errors = 0
    for k, item in ref_dict.items():
        en_ph = set(re.findall(r'%\{[^{}]*\}', item["en"]))
        zh_ph = set(re.findall(r'%\{[^{}]*\}', combined[k]))
        if en_ph != zh_ph:
            print(f"[-] Placeholder mismatch for '{k}': EN {en_ph} vs ZH {zh_ph}")
            ph_errors += 1

    if ph_errors > 0:
        print(f"[-] Found {ph_errors} placeholder errors")
        sys.exit(1)
    print("[+] All placeholders %{...} verified successfully!")

    # Check caret search terms
    caret_errors = 0
    for k, item in ref_dict.items():
        if "^" in item["en"] and "^" not in combined[k]:
            print(f"[-] Missing caret in '{k}': EN={item['en']} vs ZH={combined[k]}")
            caret_errors += 1
    if caret_errors > 0:
        sys.exit(1)
    print("[+] All caret search terms verified successfully!")

def format_yaml(combined, ref_data):
    # Preserve exact order of reference_keys.json / en.yml
    lines = []
    for item in ref_data:
        k = item["key"]
        v = combined[k]
        # Escape quotes and backslashes for double quoted yaml string
        escaped_v = v.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n")
        lines.append(f'{k}: "{escaped_v}"')
    return "\r\n".join(lines) + "\r\n"

def main():
    sys.stdout.reconfigure(encoding="utf-8")
    with open(REF_FILE, "r", encoding="utf-8") as f:
        ref_data = json.load(f)

    combined = assemble_translations()
    validate(combined, ref_data)

    yaml_content = format_yaml(combined, ref_data)

    zh_hant_path = os.path.join(LOCALES_DIR, "zh-Hant.yml")
    zh_tw_path = os.path.join(LOCALES_DIR, "zh-TW.yml")

    with open(zh_hant_path, "w", encoding="utf-8", newline="") as f:
        f.write(yaml_content)
    print(f"[+] Wrote {zh_hant_path} ({len(yaml_content)} bytes)")

    with open(zh_tw_path, "w", encoding="utf-8", newline="") as f:
        f.write(yaml_content)
    print(f"[+] Wrote {zh_tw_path} ({len(yaml_content)} bytes)")

    # Run verify-locale.py
    verify_script = os.path.join(SCRIPTS_DIR, "verify-locale.py")
    ret1 = os.system(f'python "{verify_script}" zh-Hant')
    ret2 = os.system(f'python "{verify_script}" zh-TW')

    if ret1 != 0 or ret2 != 0:
        print("[-] Verification failed!")
        sys.exit(1)
    print("[+] Both zh-Hant and zh-TW passed verify-locale.py with 100%!")

if __name__ == "__main__":
    main()
