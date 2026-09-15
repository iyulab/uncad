# Changelog

이 프로젝트의 눈에 띄는 변경을 기록한다. 형식은 [Keep a Changelog](https://keepachangelog.com/ko/1.1.0/),
버전은 [Semantic Versioning](https://semver.org/lang/ko/)을 따른다.

## [Unreleased]

**다음 릴리스는 0.2.0이다** -- 공개 API에서 쓰기 기능을 제거했다(아래 Removed).

### Removed

- DWG/DXF 쓰기 전부: `CadDatabase::write_dwg`/`write_dxf`, `uncad::dwg_to_dxf`, `WriteError`, CLI의
  `-o <x>.dxf`/`-o <x>.dwg`, `libredwg-sys`의 `uncad_write_dxf`/`uncad_write_dxf_file` 심과
  `dwg_write_file` 바인딩. 이 프로젝트의 범위는 "DWG/DXF -> 모델 -> JSON/SVG/PNG"다. C 빌드에는
  LibreDWG 인코더 소스와 `USE_WRITE`가 남는다 -- `dxf_read_file()`이 그것에 의존한다
  (`docs/ARCHITECTURE.md`의 "모델" 절).
- `CadDatabase`가 더는 LibreDWG의 `Dwg_Data`를 들고 있지 않는다: `parse()`가 변환 직후 해제한다.
  그 결과 `CadDatabase`는 `Debug`/`Clone`/`PartialEq`를 derive하는 순수 값이 됐고, 직접 생성할 수도
  있다(`CadDatabase { entities, tables }`).

### Added

- JSON 출력: `CadDatabase::to_json(ToJsonOptions { pretty })`, `uncad::json` 모듈, `JsonError`.
  모델(`entities` + `tables`)을 serde로 그대로 직렬화하며 `serde_json::from_str::<CadDatabase>`로
  되돌릴 수 있다. `RenderEntity`는 `"type"` 태그에 `type_name()`과 같은 DXF 이름(`"LINE"`,
  `"LWPOLYLINE"`, `"3DSOLID"`, ...; 미지원 타입은 `"UNKNOWN"` + `type_name` 필드)을 쓴다.
  `render_model`/`tables`의 모든 타입과 `Point2D`/`Point3D`가 `serde::Serialize`/`Deserialize`를
  derive한다. HATCH의 `boundary_paths`는 `{"type":"POLYLINE"|"EDGES","data":[...]}`, 엣지는
  `{"type":"LINE"|"ARC"|"ELLIPSE"|"SPLINE",...}`. 같은 입력이면 출력 바이트가 같다(아래 Changed).
  새 공개 의존성: `serde` 1.x, `serde_json` 1.x.
- CLI: `uncad <input> -o <output.json>`과 `--pretty`.
- 테스트: `RenderEntity`의 모든 variant에 대한 JSON 태그/왕복 유닛테스트, 실 DXF의 JSON 왕복,
  CLI의 JSON 출력과 `--pretty`, `--space`와 `--no-trim`이 실제로 출력을 바꾸는지 확인하는 CLI
  테스트(`--no-trim`은 테스트가 직접 group code로 작성한 5줄짜리 DXF 사용), SAB 솔리드가
  `entities`와 블록 레코드에서 같은 와이어프레임을 얻는지 확인하는 테스트.

### Fixed

- SAB(ACIS BinaryFile, version 2)로 저장된 3DSOLID/REGION의 와이어프레임 추출이 `parse()` 중에
  LibreDWG의 `dwg_convert_SAB_to_SAT1`을 살아있는 엔티티에 호출해 `Dwg_Data`를 제자리에서 바꾸던
  것. 두 번째 순회(`tables.block_records`)가 반쯤 변환된 엔티티를 읽어 와이어프레임을 잃었고,
  당시 있던 쓰기 경로에서는 같은 변이가 다시 써낸 파일의 솔리드를 전부 깨뜨렸다(실측
  `lib/libredwg/test/test-data/2007/ATMOS-DC22S.dwg`). 이제 `libredwg-sys`의 새 심
  `uncad_3dsolid_sab_to_sat_text`가 엔티티의 얕은 복사본에서 변환하므로 `parse()`는 `Dwg_Data`에
  부작용이 없다.
- `scripts/sync-libredwg-vendor.sh`로 `vendor/libredwg/`를 갱신한 뒤 증분 빌드가 낡은 C 오브젝트와
  `bindings.rs`를 그대로 재사용하던 문제. `build.rs`가 벤더 디렉터리 전체를 `cargo:rerun-if-changed`로
  등록한다.
- `uncad::tables::convert_tables`가 `pub`으로 노출되어, 전역 락을 우회하는 `*mut Dwg_Data` 진입점이
  공개 API에 새어 나가던 것. `pub(crate)`로 내림.

### Changed

- `Tables::{layers, block_records, mlinestyles}`가 `HashMap`에서 `BTreeMap`으로 바뀜(공개 필드 타입
  변경). 순회와 JSON 키 순서가 결정적이 되어 같은 파일은 항상 같은 JSON을 낸다.
- `MTextEntity::rotation` rustdoc이 코드와 달리 `x_axis_dir`에서 유도한다고 적혀 있던 것을 "현재 항상
  0"으로 정정.

### Docs

- `lib/libredwg` submodule은 빌드가 아니라 테스트 픽스처의 전제조건임을 README/ARCHITECTURE/CAVEATS와
  테스트 주석에 반영. 벤더 파일 수를 112개로 정정, `samples/README.md`가 존재하지 않는
  `tests/dxf_minimal.rs`를 가리키던 것 수정, CAVEATS의 테스트 수와 CI 잡 목록 갱신, `config.h`의 옛
  경로·버전·조직명 정리.
- 실측으로 확인한 업스트림 동작 기록: LibreDWG의 DXF 리더는 R2007 도면을 DXF로 써낸 파일에서 엔티티
  60개 중 1개만 복원한다(CAVEATS "DXF 읽기" 절).

## [0.1.0] - 2026-08-24

- crates.io 최초 공개 (`libredwg-sys`, `uncad`, `uncad-cli`).
