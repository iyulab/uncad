# uncad

[![CI](https://github.com/iyulab/uncad/actions/workflows/ci.yml/badge.svg)](https://github.com/iyulab/uncad/actions/workflows/ci.yml)
[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](LICENSE)

CAD 파일(DWG/DXF)을 파싱해 모델로 만들고, 그 모델을 JSON으로 내보내거나 SVG/PNG로 렌더링하는 오픈소스
Rust 라이브러리. 읽기 전용이다 -- DWG/DXF를 쓰거나 서로 변환하지 않는다.

## 빠른 시작

```bash
git submodule update --init   # 테스트 픽스처용 -- 빌드에는 불필요 (아래 "플랫폼" 절 참고)
cargo build --workspace
cargo test --workspace
```

```rust
let db = uncad::parse("drawing.dwg")?;   // DWG든 DXF든 같은 모델 (entities + tables)
println!("{} entities", db.entities.len());

let json = db.to_json(uncad::ToJsonOptions { pretty: true })?;   // 모델을 그대로 직렬화
std::fs::write("drawing.json", json)?;

let result = db.to_svg(uncad::ToSvgOptions::default());
std::fs::write("drawing.svg", result.svg)?;

let png = db.to_png(uncad::ToPngOptions::default())?;   // to_svg() -> 래스터화, 중간 SVG는 디스크에 안 씀
std::fs::write("drawing.png", png.png)?;
```

## CLI

```bash
cargo run -p uncad-cli -- drawing.dwg                  # 요약 정보 (엔티티 타입별 개수)
cargo run -p uncad-cli -- drawing.dwg -o drawing.json --pretty   # 파싱한 모델을 JSON으로 추출
cargo run -p uncad-cli -- drawing.dwg -o drawing.svg    # 이미지(SVG)로 추출 (기본: 모델 스페이스만)
cargo run -p uncad-cli -- drawing.dwg -o drawing.png    # 이미지(PNG)로 추출 (SVG를 거쳐 래스터화)
cargo run -p uncad-cli -- drawing.dwg -o drawing.png --scale 2   # 2배 해상도로 래스터화
cargo run -p uncad-cli -- drawing.dwg -o drawing.svg --no-trim  # 이상치 좌표를 뷰박스에서 자동 제외하지 않음
cargo run -p uncad-cli -- drawing.dwg -o sheet.svg --space paper   # 도곽/타이틀블록만
cargo run -p uncad-cli -- drawing.dwg -o all.svg --space all       # 모든 스페이스 합침
```

## 우선순위

1. **DWG** — [LibreDWG](https://www.gnu.org/software/libredwg/)(GPLv3+) 기반, Rust FFI(`bindgen`)로 직접 바인딩. 모든 버전 읽기.
2. **DXF** — 같은 LibreDWG 엔진으로 읽음 (확장자로 자동 판별). LibreDWG의 DXF 임포터는 "대부분의 오브젝트"에서 동작하는 수준이라 DWG 읽기만큼 완전하지 않다(`docs/CAVEATS.md` 참고).
3. **출력** — 파싱한 모델을 JSON(`to_json`), SVG(`to_svg`), PNG(`to_png`)로. DWG/DXF 쓰기는 제공하지 않는다 (0.1.0에 있던 `write_dwg`/`write_dxf`/`dwg_to_dxf`는 제거됨 -- `CHANGELOG.md`).

## 플랫폼

순수 Rust + 네이티브 FFI. WebAssembly/브라우저는 목표가 아니다 — 실제 사용처가 라이브러리/바이너리(CLI, 서버, 데스크톱 앱)뿐이다.

`bindgen`이 `libclang`을 필요로 하므로 시스템에 LLVM/Clang이 설치되어 있어야 한다
(Windows: `winget install LLVM.LLVM`, Ubuntu: `apt install libclang-dev`). LibreDWG C 소스는
`crates/libredwg-sys/vendor/libredwg/`에 벤더링되어 있어 **빌드에는 `lib/libredwg` submodule이
필요 없다**. 다만 `cargo test --workspace`의 실 파일 테스트(`uncad`의 `png.rs`,
`tests/dxf_pipeline.rs`, `tests/acis_sab.rs`와 `uncad-cli`의
`tests/documented_invocations.rs`)가 그 submodule의 `test/test-data/` 픽스처를 읽으므로,
테스트를 돌리려면 `git clone --recurse-submodules`로 받거나 이미 클론했다면
`git submodule update --init`으로 받아온다. Linux(`x86_64-unknown-linux-gnu`)에서도 빌드/테스트
전부 통과 확인됨.

## 라이선스

**GPLv3-or-later**. LibreDWG(GPLv3+)만 결합되어 있어 그 라이선스를 그대로 물려받는다. 서드파티 컴포넌트의 저작권/라이선스 상세는 [`docs/THIRD_PARTY_NOTICES.md`](./docs/THIRD_PARTY_NOTICES.md) 참고.

## 저장소 구조

```
lib/libredwg/            LibreDWG 업스트림 -- git submodule. 빌드에는 안 쓰이고, vendor/ 갱신의
                         원본이자 실 파일 테스트 픽스처(test/test-data/)의 출처
crates/
  libredwg-sys/          raw FFI (cc + bindgen). vendor/libredwg/에 실제 컴파일되는 C 소스
                         부분집합이 수정 없이 복사되어 있음 (crates.io 발행용). shim/은 opaque
                         타입 접근자 C 코드, vendor-config/config.h는 autotools 대체
  uncad/                 안전한 API: parse() -> CadDatabase::{to_json,to_svg,to_png}()
  uncad-cli/             CLI 바이너리 (uncad 명령)
crates/*/tests/          공개 API 통합 테스트. crates/*/examples/는 수동 확인용 예제,
                         src/*.rs 안의 #[cfg(test)]는 유닛 테스트 -- 세 계층의 배치 규칙은
                         docs/ARCHITECTURE.md의 "테스트 구조" 절
scripts/                 sync-libredwg-vendor.sh -- submodule 갱신 후 vendor/ 복사본 재생성
samples/                 gitignored (README 제외) -- 라이선스 확인 없이 아무 DWG/DXF나 넣고
                         수동 테스트하는 용도. 자동화된 회귀 테스트는 없음
docs/                    아키텍처, 알려진 제한, 서드파티 고지
```

더 자세한 내용은:

- [`docs/ARCHITECTURE.md`](./docs/ARCHITECTURE.md) — 크레이트 구조, 빌드 시스템, 테스트 구조, FFI/bindgen 경계, 스레드 세이프티, 엔티티 모델
- [`docs/CAVEATS.md`](./docs/CAVEATS.md) — 엔티티 타입 커버리지, 알려진 제한/버그, 크로스플랫폼 노트
