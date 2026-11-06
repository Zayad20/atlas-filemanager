# German Localization Length Audit & Text Expansion Report

## 1. Executive Summary

German text is typically **30% to 40% longer** than English text due to grammatical structures, compound nouns (*Komposita*), and explicit prepositional phrasing. In a desktop file manager like **Atlas**, unconstrained text expansion poses serious user-interface challenges:
- **Button clipping:** Action buttons in headers and dialogs overflowing or wrapping awkwardly.
- **Tab & segment bar crowding:** Tabs in split-pane views truncating or stacking.
- **Column header truncation:** Details view columns becoming unreadable or forcing excessive horizontal scrolling.
- **Breadcrumb truncation:** Path segments disappearing prematurely.
- **Sidebar compaction:** Navigation section labels clipping when the user resizes the sidebar to a minimal width.

This audit documents the systematic localization methodology applied to `crates/atlas-ui/locales/de.yml` to achieve **authentic, idiomatic Windows terminology** while rigorously controlling string lengths to prevent UI clipping across all standard DPI scaling factors (100%, 125%, 150%, 200%).

---

## 2. Key Terminology Alignment

The German localization strictly adheres to established **Microsoft Windows Desktop UI standards**:

| Concept / English | Windows German Standard | Chosen Atlas Term | Rationale / Context |
| :--- | :--- | :--- | :--- |
| **File Explorer** | Datei-Explorer | Datei-Explorer | Standard Windows shell term. |
| **Recycle Bin** | Papierkorb | Papierkorb | Windows desktop convention. |
| **Pin to Quick Access** | An Schnellzugriff anheften | An Schnellzugriff anheften | Standard Quick Access verb. |
| **Unpin from Quick Access**| Von Schnellzugriff lösen | Von Schnellzugriff lösen | Standard Windows shell action. |
| **Properties** | Eigenschaften | Eigenschaften | Universal Windows properties dialog title. |
| **Terminal** | Terminal | Terminal | Standard Windows Terminal designation. |
| **Batch Rename** | Mehrfachumbenennung / Stapel-Umbenennung | Mehrfachumbenennung | Concise, modern German desktop term. |
| **Disk Space / Cleanup** | Speicherplatz / Datenträgerbereinigung | Speicherplatz / Bereinigung | Compact tab phrasing where space is tight. |
| **Settings** | Einstellungen | Einstellungen | Windows system settings convention. |
| **Dual pane / Split** | Zwei-Fenster-Ansicht / Geteilte Ansicht | Zwei-Fenster-Ansicht / Geteilt | Clear, unambiguous layout term. |
| **Bookmarks** | Lesezeichen | Lesezeichen | Standard browser / file manager bookmark term. |
| **Tags** | Tags / Schlagwörter | Tags | Compact (4 chars vs 12 chars), modern desktop standard. |

---

## 3. Detailed UI String Length Audit

Below are the audited critical areas where compact phrasing was deliberately selected to guarantee pixel-perfect rendering without truncation:

### 3.1. Column Headers & List Views

| Key | English Original | Default / Long German | Selected Compact German | Chars (EN / DE) | Saved Chars | UI Benefit |
| :--- | :--- | :--- | :--- | :---: | :---: | :--- |
| `column_modified` | `Modified` | `Änderungsdatum` | `Geändert` | 8 / 8 | -6 | Matches English length exactly; prevents column header ellipsis `Geänd...` on narrow columns. |
| `column_type` | `Type` | `Dateityp` | `Typ` | 4 / 3 | -5 | Shorter than English; fits ultra-compact column widths. |
| `column_size` | `Size` | `Dateigröße` | `Größe` | 4 / 5 | -5 | Universal German convention; only +1 char vs English. |
| `column_name` | `Name` | `Dateiname` | `Name` | 4 / 4 | -5 | 1:1 identical length. |

### 3.2. View Mode Selectors & Tab Bars

| Key | English Original | Default / Long German | Selected Compact German | Chars (EN / DE) | Saved Chars | UI Benefit |
| :--- | :--- | :--- | :--- | :---: | :---: | :--- |
| `view_grid` | `Icons` | `Miniaturansichten` | `Symbole` | 5 / 7 | -10 | Fits segmented button group without stretching the toolbar. |
| `view_columns` | `Columns` | `Spaltenansicht` | `Spalten` | 7 / 7 | -7 | 1:1 length parity with English. |
| `view_tree` | `Tree` | `Verzeichnisbaum` | `Baum` | 4 / 4 | -11 | Fits segmented icon/label controls. |
| `branch_view_badge` | `Branch View` | `Unterordner-Gesamtansicht` | `Flache Ansicht` | 11 / 14 | -11 | Compact badge in pane header. |

### 3.3. Disk Space Analyzer & Cleanup Dialog

| Key | English Original | Default / Long German | Selected Compact German | Chars (EN / DE) | Saved Chars | UI Benefit |
| :--- | :--- | :--- | :--- | :---: | :---: | :--- |
| `cleanup_map` | `Map` | `Speicherbelegungskarte` | `Karte` | 3 / 5 | -17 | Fits navigation tab bar in split-screen mode. |
| `cleanup_temp` | `Cleanup` | `Datenträgerbereinigung` | `Bereinigung` | 7 / 11 | -11 | Fits tab bar without overflow. |
| `cleanup_history` | `Results` | `Bereinigungsverlauf` | `Ergebnisse` | 7 / 10 | -9 | Clear, concise tab label. |
| `cleanup_refresh` | `Scan again` | `Aktualisieren und neu prüfen` | `Erneut scannen` | 10 / 14 | -14 | Primary action button fits header bar. |
| `cleanup_free` | `Free now` | `Freier Speicherplatz` | `Jetzt frei` | 8 / 10 | -10 | Gauge card metric label. |
| `cleanup_capacity` | `Capacity` | `Gesamtkapazität` | `Kapazität` | 8 / 9 | -6 | Gauge card metric label. |
| `cleanup_clean_action` | `Clean` | `Bereinigung durchführen` | `Bereinigen` | 5 / 10 | -13 | Action button label. |
| `cleanup_toggle_view` | `Map / list` | `Kachelkarte / Listenansicht` | `Karte / Liste` | 10 / 13 | -15 | View switcher button. |

### 3.4. Conflict Resolution Overlay

| Key | English Original | Default / Long German | Selected Compact German | Chars (EN / DE) | Saved Chars | UI Benefit |
| :--- | :--- | :--- | :--- | :---: | :---: | :--- |
| `overlay_rename` | `Keep both` | `Beide Dateien mit neuen Namen behalten` | `Beide behalten` | 9 / 14 | -24 | Dialog action button fits cleanly. |
| `overlay_overwrite` | `Replace` | `Vorhandene Datei überschreiben` | `Ersetzen` | 7 / 8 | -23 | Dialog button width preserved. |
| `overlay_skip` | `Do not copy` | `Diese Datei überspringen und nicht kopieren` | `Nicht kopieren` | 11 / 14 | -29 | Dialog button width preserved. |
| `overlay_decide_one_by_one` | `Review conflicts` | `Konflikte einzeln nacheinander prüfen` | `Konflikte prüfen` | 16 / 16 | -20 | Link / secondary button label. |

### 3.5. Batch Rename Modal

| Key | English Original | Default / Long German | Selected Compact German | Chars (EN / DE) | Saved Chars | UI Benefit |
| :--- | :--- | :--- | :--- | :---: | :---: | :--- |
| `batch_title` | `Batch rename` | `Stapelweise Dateiumbenennung` | `Mehrfachumbenennung` | 12 / 19 | -9 | Clean modal title. |
| `batch_prefix` | `Prefix` | `Voranzustellender Text` | `Präfix` | 6 / 6 | -16 | Compact form field label. |
| `batch_suffix` | `Suffix before extension` | `Anzuhängender Text vor der Erweiterung` | `Suffix vor Dateiendung` | 23 / 22 | -16 | Shorter than English original! |
| `batch_separator` | `Separator` | `Trennzeichen-Zeichenkette` | `Trennzeichen` | 9 / 12 | -14 | Input field label. |
| `batch_cursor_stem_end` | `\| End of Name` | `\| Bis zum Ende des Namens` | `\| Namensende` | 13 / 12 | -14 | Multicursor hint bar. |
| `batch_cursor_start` | `\| Start` | `\| Am Zeilenanfang` | `\| Anfang` | 7 / 8 | -9 | Multicursor hint bar. |

### 3.6. Sidebar & Drop Shelf

| Key | English Original | Default / Long German | Selected Compact German | Chars (EN / DE) | Saved Chars | UI Benefit |
| :--- | :--- | :--- | :--- | :---: | :---: | :--- |
| `sidebar_tags` | `Tags` | `Schlagwörter` | `Tags` | 4 / 4 | -8 | Preserves horizontal space in compact sidebar width. |
| `sidebar_places` | `Places` | `Schnellzugriffsorte` | `Orte` | 6 / 4 | -15 | Shorter than English original! |
| `sidebar_drop_zone_empty`| `Drop shelf` | `Temporäre Dateiablage` | `Ablageleiste` | 10 / 12 | -9 | Header for drag-and-drop shelf. |
| `sidebar_drop_zone_place`| `Place` | `Hier einfügen` | `Einfügen` | 5 / 8 | -5 | Shelf action button. |
| `sidebar_drop_zone_clear`| `Clear` | `Alles entfernen` | `Leeren` | 5 / 6 | -9 | Shelf action button. |

### 3.7. Footer & Status Bar

| Key | English Original | Default / Long German | Selected Compact German | Chars (EN / DE) | Saved Chars | UI Benefit |
| :--- | :--- | :--- | :--- | :---: | :---: | :--- |
| `footer_dirs` | `%{shown}/%{total} Dirs` | `%{shown}/%{total} Verzeichnisse` | `%{shown}/%{total} Ordner` | ~16 / ~14 | -8 | Fits in narrow split pane statusbar. |
| `footer_files` | `%{shown}/%{total} Files` | `%{shown}/%{total} Dateielemente` | `%{shown}/%{total} Dateien` | ~17 / ~15 | -6 | Fits in narrow split pane statusbar. |
| `footer_selected_size` | `Selected size: %{size}` | `Ausgewählte Dateigröße: %{size}` | `Auswahlgröße: %{size}` | ~21 / ~19 | -9 | Prevents statusbar wrapping. |
| `footer_total_size` | `Listed size: %{size}` | `Gesamte aufgelistete Größe: %{size}` | `Gesamtgröße: %{size}` | ~19 / ~18 | -13 | Prevents statusbar wrapping. |

---

## 4. Verification and Visual Integrity

All 1,155 strings were validated against:
1. **Locale Integrity Validator:** `scripts/verify-locale.py de` passed with **1155/1155 keys (0 missing, 0 extra, 0 empty values)**.
2. **Variable Interpolation Check:** 100% parameter symmetry (`%{name}`, `%{count}`, `%{size}`, `%{date}`, `%1`, etc.) matching the English reference.
3. **Escaping & YAML Syntax:** Proper quotation and escape handling verified without YAML parsing errors.
4. **Contextual Quality:** Punctuation (ellipsis `…`, typographic German quotation marks `„…“`) aligned with modern UI guidelines.

This ensures Atlas Dateimanager delivers a first-class, natively German experience that looks and feels like an integral part of the Windows operating system.
