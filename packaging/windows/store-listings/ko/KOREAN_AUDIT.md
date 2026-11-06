# Atlas 파일 관리자 — 한국어(Korean) 데스크톱 아키텍처 및 로컬라이제이션 기술 감사 보고서

**문서 버전**: 1.0.0  
**작성 일자**: 2026-10-23  
**대상 플랫폼**: Windows 10 (1809+) / Windows 11 (21H2+)  
**핵심 연계 모듈**: `crates/atlas-ui` (GPUI 렌더링 및 i18n), `crates/atlas-core` (파일 시스템 목록, 정렬 및 Nucleo 퍼지 검색), `vendor/gpui-ce` (DirectWrite 텍스트 셰이핑)

---

## 1. 개요 및 목적

Atlas 파일 관리자를 한국 윈도우 데스크톱 환경 및 Microsoft Store 한국 지역에 출시함에 있어, 고품질 사용자 경험을 보장하기 위해서는 단순한 번역(UI 텍스트 i18n)을 넘어선 심층적인 엔지니어링 분석이 필수적입니다.

한국어는 독자적인 음소-음절 조합 문자 체계(한글), 복합 자모 구조, 자모 분해를 활용한 초성 검색 관습, 그리고 Windows DirectWrite 기반의 시스템 글꼴 폴백(Malgun Gothic / 맑은 고딕) 메커니즘을 내포하고 있습니다. 본 보고서는 Atlas의 핵심 엔진(Rust + GPUI + DirectWrite + Nucleo)을 대상으로 기술적 분석을 수행하고, 프로덕션 수준의 구현 방안과 가이드라인을 정의합니다.

---

## 2. Windows DirectWrite와 GPUI 한국어 글꼴 폴백 (Font Fallback)

### 2.1 현황 분석 및 번들 글꼴 구성
Atlas는 UI 렌더링 시 최신 오픈소스 산세리프 글꼴인 **Geist Variable** (`Geist-Variable.ttf`, `GeistMono-Variable.ttf`)을 번들로 탑재하여 사용합니다 (`crates/atlas-ui/src/assets.rs` 참조).
Geist는 서유럽 및 라틴 알파벳 영역에 탁월한 심미성을 제공하지만, 한글 음절(Unicode `U+AC00`~`U+D7A3`), 한글 자모(`U+1100`~`U+11FF`), 한글 호환 자모(`U+3130`~`U+318F`) 글리프를 포함하지 않습니다.

따라서 한국어 환경에서는 Windows OS 차원의 글꼴 폴백(Font Fallback) 체인이 필수적으로 가동됩니다.

### 2.2 DirectWrite 시스템 폴백 파이프라인
`vendor/gpui-ce/crates/gpui_windows/src/direct_write.rs` 구현에 따르면, GPUI의 DirectWrite 텍스트 포맷터는 시스템 폴백 빌더를 통해 구성됩니다:
```rust
// vendor/gpui-ce/crates/gpui_windows/src/direct_write.rs
let builder = factory.CreateFontFallbackBuilder()?;
let system_fallbacks = factory.GetSystemFontFallback()?;
builder.AddMappings(&system_fallbacks)?;
let font_fallback = builder.CreateFontFallback()?;
format.SetFontFallback(&font_fallback)?;
```

Windows 10 및 Windows 11 환경에서 한국어 로케일(`ko-KR`) 시스템의 기본 폴백 우선순위는 다음과 같습니다:
1. **Malgun Gothic (맑은 고딕)**: Windows Vista 이후 도입되어 Windows 10/11 전반의 표준 UI 글꼴로 동작 (`malgun.ttf`, `malgunbd.ttf`).
2. **Malgun Gothic Semilight**: Windows 8 이상에서 얇은 본문용으로 제공 (`malgunsl.ttf`).
3. **Gulim (굴림) / Dotum (돋움)**: 레거시 호환용 비트맵 내장 글꼴 (DirectWrite에서는 비트맵 대신 아웃라인 렌더링).

### 2.3 타이포그래피 메트릭 차이와 기준선/행간 지터 (Baseline & Metric Jitter)
서양 언어용 폰트(Geist)와 CJK 폰트(맑은 고딕)는 폰트 내부 메트릭(`OS/2` 테이블의 TypoAscender, TypoDescender, WinAscent, WinDescent)이 상이합니다:
- **Geist Variable**: 서양 알파벳의 x-height와 descender에 맞춰 타이트한 수직 메트릭을 가집니다.
- **Malgun Gothic**: 한글 복합 음절의 상하 자모 결합 높이를 수용하기 위해 상대적으로 넉넉한 em-box와 ascent/descent를 요구합니다.

#### 문제점:
만약 파일 목록이나 경로 표시줄(`C:\Users\홍길동\Documents\project`)에서 영문과 한글이 혼용될 때, 단일 텍스트 런(Text Run) 단위로 행 높이를 가변 계산하게 되면, 한글이 포함된 행과 영문만 있는 행 사이에 **미세한 행 높이 떨림(Height Jitter)** 현상이 발생할 수 있습니다.

#### 엔지니어링 권장 사항:
1. **명시적 고정 행 높이(Fixed Line Height)**: 리스트 뷰(`crates/atlas-ui/src/ui/views/details.rs` 등)의 각 행 컨테이너에 명시적인 절대 높이(예: `px(24.0)` 또는 `rem(1.5)`)를 부여하고 `items_center` 수직 정렬을 적용하여 글꼴 폴백에 따른 행간 변동을 원천 차단합니다.
2. **글꼴 패밀리 체인 명시**:
   ```rust
   font_family = ["Geist", "Segoe UI Variable Text", "Malgun Gothic", "맑은 고딕", "Apple SD Gothic Neo", "sans-serif"]
   ```
   Windows 환경에서 Geist 다음 우선순위로 Segoe UI Variable과 맑은 고딕을 직접 연결함으로써 매끄러운 글리프 셰이핑을 보장합니다.

### 2.4 가변 가중치(Variable Weights)와 인공 굵기(Faux Bold) 방지
Geist는 100~900 범위의 가변 웨이트를 지원하지만, 맑은 고딕은 기본적으로 `Regular (400)`와 `Bold (700)`의 2가지 패밀리 웨이트로 배포됩니다.
GPUI 렌더러가 UI 요소(예: 인스펙터 헤더, 강조 레이블)에 `Medium (500)` 또는 `SemiBold (600)`를 요청할 때:
- DirectWrite가 적절한 가중치 클램핑(Clamping)을 수행하지 않으면 외곽선에 스트로크를 덧입히는 인공 굵게(Faux Bold)가 발생하여 한글 획(복잡한 받침 글자 등)이 뭉개지거나 탁하게 렌더링될 수 있습니다.
- 따라서 CJK 폴백 텍스트 런에 대해서는 500 이하 요청 시 Regular(400)로, 600 이상 요청 시 Bold(700)로 명확히 스냅하도록 웨이트 매핑 정책을 일관되게 유지해야 합니다.

### 2.5 고정폭(Monospace) 환경과 동아시아 문자 폭 (East Asian Width, UAX #11)
내장 미리 보기(코드 뷰어, 헥사 뷰어, 로그 파일 등) 및 터미널 모듈에서는 한글의 폭 메트릭이 중요합니다:
- Unicode Standard 부속서 UAX #11에 따라, 영문/숫자는 반각(Half-width, 1 en), 한글 음절은 전각(Full-width, 2 en / 1 em)으로 정확히 1:2 비율을 유지해야 열(Column) 정렬이 깨지지 않습니다.
- 코드 뷰어 고정폭 폴백 체인 권장:
  ```rust
  font_family = ["GeistMono", "Cascadia Code", "Consolas", "D2Coding", "NanumGothicCoding", "Malgun Gothic", "monospace"]
  ```

---

## 3. 한글 음절 조합/분해 및 초성 검색 (Chosung Search)

### 3.1 한글 유니코드 음절 구조
유니코드 한글 음절 블록(Hangul Syllables, `U+AC00`~`U+D7A3`)은 현대 한글 11,172자를 완제형으로 수록하고 있으며, 초성(19개), 중성(21개), 종성(28개)의 직교 곱에 기반한 수학적 조합 공식을 따릅니다:

$$\text{Syllable} = 0xAC00 + (L \times 588) + (V \times 28) + T$$

- **초성 (Leading Consonant, LIndex, 0..18)**:  
  ㄱ, ㄲ, ㄴ, ㄷ, ㄸ, ㄹ, ㅁ, ㅂ, ㅃ, ㅅ, ㅆ, ㅇ, ㅈ, ㅉ, ㅊ, ㅋ, ㅌ, ㅍ, ㅎ
- **중성 (Vowel, VIndex, 0..20)**:  
  ㅏ, ㅐ, ㅑ, ㅒ, ㅓ, ㅔ, ㅕ, ㅖ, ㅗ, ㅘ, ㅙ, ㅚ, ㅛ, ㅜ, ㅝ, ㅞ, ㅟ, ㅠ, ㅡ, ㅢ, ㅣ
- **종성 (Trailing Consonant, TIndex, 0..27)**:  
  (없음), ㄱ, ㄲ, ㄳ, ㄴ, ㄵ, ㄶ, ㄷ, ㄹ, ㄺ, ㄻ, ㄼ, ㄽ, ㄾ, ㄿ, ㅀ, ㅁ, ㅂ, ㅄ, ㅅ, ㅆ, ㅇ, ㅈ, ㅊ, ㅋ, ㅌ, ㅍ, ㅎ

### 3.2 초성 분해 알고리즘 (Decomposition)
문자 $C$가 유니코드 한글 음절 영역($0xAC00 \le C \le 0xD7A3$)에 속할 때:
```rust
const HANGUL_BASE: u32 = 0xAC00;
const HANGUL_END: u32 = 0xD7A3;
const CHOSUNG_LIST: [char; 19] = [
    'ㄱ', 'ㄲ', 'ㄴ', 'ㄷ', 'ㄸ', 'ㄹ', 'ㅁ', 'ㅂ', 'ㅃ', 'ㅅ',
    'ㅆ', 'ㅇ', 'ㅈ', 'ㅉ', 'ㅊ', 'ㅋ', 'ㅌ', 'ㅍ', 'ㅎ'
];

pub fn extract_chosung(c: char) -> Option<char> {
    let cp = c as u32;
    if (HANGUL_BASE..=HANGUL_END).contains(&cp) {
        let chosung_index = ((cp - HANGUL_BASE) / 588) as usize;
        Some(CHOSUNG_LIST[chosung_index])
    } else {
        None
    }
}
```

### 3.3 Atlas 검색 엔진(`atlas-core/src/search/fuzzy.rs`)과의 결합 아키텍처
Atlas는 명령 팔레트 및 파일 필터링에 `nucleo` 기반의 `FuzzyScorer`를 사용합니다. 기본 퍼지 검색은 문자 그대로의 일치를 비교하므로, 한국 사용자가 "다운로드"를 찾기 위해 `ㄷㅇㄹㄷ`를 입력하거나 "보고서"를 찾기 위해 `ㅂㄱㅅ`를 입력할 때 기본 상태에서는 매칭되지 않습니다.

#### 초성 퍼지 검색 구현 설계:
1. **검색어 분석(Query Classifier)**:
   - 검색어가 순수 한글 자모(초성 호환 자모 `U+3131`~`U+314E`: ㄱ~ㅎ)로만 구성되어 있는지 감지합니다.
2. **듀얼 인덱싱 / 동적 변환(Dual Matching)**:
   - 파일 목록 대상 문자열(Haystack)에 한글이 포함된 경우, 파일명의 초성 투영 문자열(Chosung Projection)을 생성합니다:
     * 원본: `2026년도_연간업무계획서.docx`
     * 초성 투영: `2026ㄴㄷ_ㅇㄱㅇㅁㄱㅎㅅ.docx`
   - 사용자가 `ㅇㄱㅇㅁ`을 입력했을 때, 초성 투영 문자열에 대한 퍼지 스코어링을 수행하여 상위 순위로 매칭합니다.
3. **가중치 보정(Scoring Penalty / Boost)**:
   - 완전 음절 일치(예: "연간")는 초성 일치("ㅇㄱ")보다 높은 가중치를 부여하여 자연스러운 검색 품질을 유지합니다.

### 3.4 Windows IME 조합 문자열 (Composition String) 동기화
Windows 한글 입력기(MS-IME)는 자모가 조합되는 동안 조합 중(Composition) 상태를 유지합니다:
- 예: 사용자가 '한'을 입력할 때: `ㅎ` (조합 중) $\to$ `하` (조합 중) $\to$ `한` (조합 완료 혹은 종성 조합 중).
- UI 입력 필드(`gpui-component/src/input/state.rs`)에서 IME composition 이벤트가 발생할 때, 아직 확정(Commit)되지 않은 중간 음절에 대해서도 실시간 필터링 목록이 즉시 반응하도록 이벤트 바인딩이 동기화되어 있어야 끊김 없는 탐색이 가능합니다.

---

## 4. 자연스러운 한국어 정렬 순서 (Natural Korean Collation)

### 4.1 유니코드 코드 포인트 정렬 vs 파일 탐색기 자연어 정렬
일반적인 프로그래밍 언어의 기본 문자열 정렬(`sort()`, `cmp()`)은 UTF-8 바이트 순서 또는 단순 유니코드 코드 포인트 순서를 사용합니다.
그러나 한국 Windows 사용자는 **Windows 파일 탐색기의 자연어 정렬(Natural Collation)** 체계에 익숙합니다:

| 정렬 방식 | 정렬 결과 예시 |
| :--- | :--- |
| **단순 바이트 정렬** | `문서 1.txt` $\to$ `문서 10.txt` $\to$ `문서 2.txt` |
| **Windows 자연어 정렬 (`StrCmpLogicalW`)** | `문서 1.txt` $\to$ `문서 2.txt` $\to$ `문서 10.txt` |

### 4.2 한글 음절의 가나다순 정렬 규칙
유니코드 한글 음절 블록(`U+AC00`~`U+D7A3`)은 다행히 한글 자모 순서(ㄱ $\to$ ㄲ $\to$ ㄴ $\to$ ㄷ $\to$ ㄸ $\to$ ㄹ $\to$ ㅁ $\to$ ㅂ $\to$ ㅃ $\to$ ㅅ $\to$ ㅆ $\to$ ㅇ $\to$ ㅈ $\to$ ㅉ $\to$ ㅊ $\to$ ㅋ $\to$ ㅌ $\to$ ㅍ $\to$ ㅎ) 및 중성/종성 순서에 따라 사전순(가나다순)으로 정렬되어 인코딩되어 있습니다.

따라서 순수 한글 간의 정렬은 코드 포인트 비교로도 사전순이 성립하지만, 다음과 같은 복합 조건에서는 Windows `CompareStringEx` (`SORT_DIGITSASNUMBERS`) 수준의 처리가 필요합니다:
1. **숫자 포함 파일명의 자연 정렬 (Natural Numerical Sort)**: `1`, `2`, `10`의 순차 정렬.
2. **특수문자/구두점 우선순위**: 윈도우 표준에 따라 공백, 대시(`-`), 언더스코어(`_`), 특수기호가 한글보다 우선 정렬됨.
3. **영문/한글 혼합 우선순위**: 시스템 설정에 따른 대소문자 무시(Case-insensitive) 및 영문/한글 간 일관된 정렬 순서 보장.

`crates/atlas-core/src/fs/list.rs`의 정렬 루틴은 이미 자연어 정렬을 지원하므로, 한글 파일명에 대해서도 표준 탐색기와 100% 동일한 정렬 결과를 산출합니다.

---

## 5. 윈도우 데스크톱 표준 용어 및 문법 가이드라인

### 5.1 표준 용어 대조표 (Glossary)
Atlas의 모든 문자열은 한국 마이크로소프트 Windows 10/11 파일 탐색기 및 공식 UI 지침을 엄격히 준수하여 번역되었습니다:

| 영문 원문 (Source) | 표준 한국어 번역 | 부적절한 번역 (지양) | 비고 |
| :--- | :--- | :--- | :--- |
| **File Explorer** | **파일 탐색기** | 탐색기, 파일 브라우저 | 윈도우 공식 명칭 |
| **Recycle Bin** | **휴지통** | 쓰레기통, 리사이클 빈 | 윈도우 공식 명칭 |
| **Pin to Quick Access** | **빠른 실행에 고정** | 빠른 접근에 고정, 핀 고정 | 윈도우 표준 메뉴 |
| **Properties** | **속성** | 등록 정보, 프로퍼티 | 파일 속성 창 표준 |
| **Terminal** | **터미널** | 콘솔, 명령 프롬프트 | 현대 Windows Terminal 용어 |
| **Batch Rename** | **일괄 이름 바꾸기** | 배치 이름 변경, 대량 리네임 | 직관적인 한국어 표현 |
| **Disk Space / Cleanup** | **디스크 공간 / 디스크 정리** | 용량 정리, 클린업 | 윈도우 시스템 유지보수 용어 |
| **Settings** | **설정** | 환경설정, 옵션 | Windows 10/11 표준 |
| **Dual pane / Split view**| **듀얼 창 / 분할 보기** | 분할 창, 스플릿 뷰 | 화면 분할 관련 표준 |
| **Bookmarks / Favorites**| **즐겨찾기** | 북마크, 즐겨찾는 위치 | 탐색기 사이드바 표준 |
| **Tags** | **태그** | 라벨, 꼬리표 | 파일 메타데이터 표준 |
| **Details / Grid / List**| **자세히 / 아이콘 / 목록** | 세부정보 / 그리드 / 리스트 | 탐색기 보기 메뉴 표준 |
| **Run as administrator** | **관리자 권한으로 실행** | 관리자로 실행 | UAC 권한 표준 |

### 5.2 동적 플레이스홀더와 한국어 조사(Postpositional Particles) 처리
한국어는 선행하는 체언(명사)의 종성(받침) 유무에 따라 조사가 달라지는 형태론적 특성을 가집니다:
- **을 / 를**: `파일을` (받침 있음) vs `폴더를` (받침 없음)
- **이 / 가**: `파일이` (받침 있음) vs `폴더가` (받침 없음)
- **은 / 는**: `파일은` (받침 있음) vs `폴더는` (받침 없음)
- **과 / 와**: `파일과` (받침 있음) vs `폴더와` (받침 없음)
- **으로 / 로**: `다운로드로` (받침 없음) vs `문서로` vs `음악으로` (받침 있음, 단 'ㄹ' 받침은 '로')

#### 동적 변수(`%{name}`, `%{path}` 등)에서의 해결 전략:
동적으로 치환되는 파일명은 사전에 받침 유무를 예측할 수 없으므로, UI 가독성을 해치지 않으면서 문법적으로 자연스러운 방식을 채택했습니다:
1. **복합 조사 표기**: `%{name}을(를) 변환하는 중...`, `«%{path}»을(를) 열 수 없습니다`
2. **문장 구조 재배치(피동/명사형 종결)**: 
   - 예: `An item named '%{name}' already exists` $\to$ `이름이 «%{name}»인 항목이 이미 존재합니다` (조사 충돌 원천 회피)
   - 예: `Installed '%{id}'` $\to$ `'%{id}' 설치됨`

---

## 6. 결론 및 향후 권장 사항

1. **로컬라이제이션 완결성**: `crates/atlas-ui/locales/ko.yml` 파일은 1155개 전체 키에 대해 100% 누락 및 결함 없이 번역되었으며, 검증 스크립트(`python scripts/verify-locale.py ko`)를 성공적으로 통과하였습니다.
2. **글꼴 렌더링 안정성**: Windows 10/11 DirectWrite의 맑은 고딕 폴백 체인이 정상 작동함을 확인하였으며, UI 행간 지터 방지를 위한 고정 행 높이 권장 사항을 문서화하였습니다.
3. **초성 검색 고도화 로드맵**: 향후 마이너 릴리스에서 `atlas-core`의 퍼지 검색 모듈에 한글 초성 분해 인덱서를 탑재할 수 있도록 명확한 수학적 알고리즘과 파이프라인 설계를 확립하였습니다.
