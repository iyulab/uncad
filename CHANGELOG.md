# Changelog

이 프로젝트의 눈에 띄는 변경을 기록한다. 형식은 [Keep a Changelog](https://keepachangelog.com/ko/1.1.0/),
버전은 [Semantic Versioning](https://semver.org/lang/ko/)을 따른다.

## [Unreleased]

### Fixed

- SAB(ACIS BinaryFile, version 2)로 저장된 3DSOLID/REGION이 있는 도면을 `write_dxf`/`write_dwg`로
  다시 쓰면 솔리드가 깨지던 버그. `parse()`가 와이어프레임 추출을 위해 LibreDWG의
  `dwg_convert_SAB_to_SAT1`을 살아있는 엔티티에 호출해 `Dwg_Data`를 제자리에서 바꾸고 있었고,
  LibreDWG의 두 인코더는 그 결과(`version == 1` + 평문 SAT)를 이미 난독화된 것으로 간주해 그대로
  써냈다. 이제 `libredwg-sys`의 새 심 `uncad_3dsolid_sab_to_sat_text`가 엔티티의 얕은 복사본에서
  변환하므로 `parse()`는 `Dwg_Data`를 건드리지 않는다. 실측 파일
  `lib/libredwg/test/test-data/2007/ATMOS-DC22S.dwg`로 회귀 테스트 추가(`crates/uncad/tests/acis_sab.rs`).
- `scripts/sync-libredwg-vendor.sh`로 `vendor/libredwg/`를 갱신한 뒤 증분 빌드가 낡은 C 오브젝트와
  `bindings.rs`를 그대로 재사용하던 문제. `build.rs`가 벤더 디렉터리 전체를 `cargo:rerun-if-changed`로
  등록한다.
- `uncad::tables::convert_tables`가 `pub`으로 노출되어, 전역 락을 우회하는 `*mut Dwg_Data` 진입점이
  공개 API에 새어 나가던 것. `pub(crate)`로 내림.

### Changed

- `dwg_to_dxf`가 DWG 입력만 받는다는 사실을 문서화(뒤의 심이 `dwg_read_file`을 고정 호출). DXF를 다시
  쓰려면 `parse()` + `write_dxf()`.
- `MTextEntity::rotation` rustdoc이 코드와 달리 `x_axis_dir`에서 유도한다고 적혀 있던 것을 "현재 항상
  0"으로 정정.

### Added

- 테스트: DWG→DWG 왕복과 `write_dwg`의 기존 파일 덮어쓰기 거부(`crates/uncad/tests/write_dwg.rs`),
  `--space`와 `--no-trim`이 실제로 출력을 바꾸는지 확인하는 CLI 테스트(`--no-trim`은 테스트가 직접
  group code로 작성한 5줄짜리 DXF 사용), DXF 왕복 테스트를 개수 비교에서 엔티티 타입 순서 비교로 강화.

### Docs

- `lib/libredwg` submodule은 빌드가 아니라 테스트 픽스처의 전제조건임을 README/ARCHITECTURE/CAVEATS와
  테스트 주석에 반영. 벤더 파일 수를 112개로 정정, `samples/README.md`가 존재하지 않는
  `tests/dxf_minimal.rs`를 가리키던 것 수정, CAVEATS의 테스트 수와 CI 잡 목록 갱신, `config.h`의 옛
  경로·버전·조직명 정리.
- 실측으로 확인한 업스트림 동작 기록: `dwg_write_dxf`는 SAB 솔리드를 제자리에서 SAT1로 바꿈,
  `write_dwg`는 R2007 소스를 R2010으로 써냄, LibreDWG의 DXF 리더는 그 R2007 도면의 DXF에서 엔티티
  60개 중 1개만 복원.

## [0.1.0] - 2026-08-24

- crates.io 최초 공개 (`libredwg-sys`, `uncad`, `uncad-cli`).
