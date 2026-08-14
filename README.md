# uncad

[![CI](https://github.com/iyulab/uncad/actions/workflows/ci.yml/badge.svg)](https://github.com/iyulab/uncad/actions/workflows/ci.yml)
[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](LICENSE)

CAD 파일(DWG/DXF)을 파싱, 렌더링(SVG), 저장(DWG/DXF)하기 위한 오픈소스 Rust 라이브러리.

## 빠른 시작

```bash
git submodule update --init   # 처음 클론했다면
cargo build --workspace
cargo test --workspace
```

```rust
let mut db = uncad::parse("drawing.dwg")?;
println!("{} entities", db.entities.len());

let result = db.to_svg(uncad::ToSvgOptions::default());
std::fs::write("drawing.svg", result.svg)?;

let png = db.to_png(uncad::ToPngOptions::default())?;   // to_svg() -> 래스터화, 중간 SVG는 디스크에 안 씀
std::fs::write("drawing.png", png.png)?;

// 이미 파싱한 db를 그대로 다시 저장 -- 파일을 새로 읽지 않음.
db.write_dxf("drawing.dxf")?;
db.write_dwg("drawing_copy.dwg")?;   // R_2004 이하에서만 안정적, docs/CAVEATS.md 참고

// db 없이 파일→파일로 빠르게 변환만 하고 싶을 때(파싱 오버헤드 없음):
uncad::dwg_to_dxf("drawing.dwg", "drawing2.dxf")?;
```

## CLI

```bash
cargo run -p uncad-cli -- drawing.dwg                  # 요약 정보 (엔티티 타입별 개수)
cargo run -p uncad-cli -- drawing.dwg -o drawing.svg    # 이미지(SVG)로 추출 (기본: 모델 스페이스만)
cargo run -p uncad-cli -- drawing.dwg -o drawing.png    # 이미지(PNG)로 추출 (SVG를 거쳐 래스터화)
cargo run -p uncad-cli -- drawing.dwg -o drawing.png --scale 2   # 2배 해상도로 래스터화
cargo run -p uncad-cli -- drawing.dwg -o drawing.dxf    # DWG -> DXF 저장
cargo run -p uncad-cli -- drawing.dxf -o drawing.dwg    # DXF -> DWG 저장 (R_2004 이하만 안정적)
cargo run -p uncad-cli -- drawing.dwg -o sheet.svg --space paper   # 도곽/타이틀블록만
cargo run -p uncad-cli -- drawing.dwg -o all.svg --space all       # 모든 스페이스 합침
```

## 우선순위

1. **DWG** — [LibreDWG](https://www.gnu.org/software/libredwg/)(GPLv3+) 기반, Rust FFI(`bindgen`)로 직접 바인딩. 읽기는 모든 버전, 쓰기는 R_2004 이하만 안정적(업스트림 자체 제약, `docs/CAVEATS.md` 참고).
2. **DXF** — 같은 LibreDWG 엔진으로 처리 (읽기/쓰기 둘 다 지원, 확장자로 자동 판별)

## 플랫폼

순수 Rust + 네이티브 FFI. WebAssembly/브라우저는 목표가 아니다 — 실제 사용처가 라이브러리/바이너리(CLI, 서버, 데스크톱 앱)뿐이다.

`bindgen`이 `libclang`을 필요로 하므로 시스템에 LLVM/Clang이 설치되어 있어야 한다
(Windows: `winget install LLVM.LLVM`, Ubuntu: `apt install libclang-dev`). `lib/libredwg`는
git submodule이므로 클론 시 `git clone --recurse-submodules` 사용하거나, 이미 클론했다면
`git submodule update --init`으로 받아온다. Linux(`x86_64-unknown-linux-gnu`)에서도 빌드/테스트
전부 통과 확인됨.

## 라이선스

**GPLv3-or-later**. LibreDWG(GPLv3+)만 결합되어 있어 그 라이선스를 그대로 물려받는다. 서드파티 컴포넌트의 저작권/라이선스 상세는 [`docs/THIRD_PARTY_NOTICES.md`](./docs/THIRD_PARTY_NOTICES.md) 참고.

## 저장소 구조

```
lib/libredwg/            LibreDWG C 소스 -- git submodule, 수정 없이 그대로 씀
crates/
  libredwg-sys/          raw FFI (cc + bindgen)
  uncad/                 안전한 API: parse()/CadDatabase::{to_svg,to_png,write_dwg,write_dxf}()/dwg_to_dxf()
  uncad-cli/             CLI 바이너리 (uncad 명령)
samples/                 gitignored (README 제외) -- 라이선스 확인 없이 아무 DWG/DXF나 넣고
                         수동 테스트하는 용도. 자동화된 회귀 테스트는 없음
docs/                    아키텍처, 알려진 제한, 서드파티 고지
```

더 자세한 내용은:

- [`docs/ARCHITECTURE.md`](./docs/ARCHITECTURE.md) — 크레이트 구조, 빌드 시스템, FFI/bindgen 경계, 스레드 세이프티, 엔티티 모델
- [`docs/CAVEATS.md`](./docs/CAVEATS.md) — 엔티티 타입 커버리지, 알려진 제한/버그, 크로스플랫폼 노트
