# 아키텍처

## 크레이트 구조

```
lib/libredwg/            LibreDWG 업스트림(github.com/LibreDWG/libredwg)을 가리키는 git submodule.
                         빌드에는 쓰이지 않는다 -- vendor/ 복사본의 원본이자, 실 파일 테스트
                         픽스처(test/test-data/)의 출처. 수정 없이 그대로 둠.
crates/
  libredwg-sys/          raw FFI: build.rs가 vendor/libredwg/src/*.c(아래 "빌드" 절)를 cc
                         크레이트로 직접 컴파일(autotools 없이) + bindgen으로 바인딩 생성.
    shim/                uncad_shim.c -- opaque 타입 너머의 엔티티 포인터를 꺼내는 접근자 +
                         dynapi로 도달 못 하는 중첩 구조체(MULTILEADER 리더 라인)를 순회해서
                         평평한 배열로 넘겨주는 함수 + 3DSOLID의 SAB→SAT 변환을 원본을
                         건드리지 않고 복사본에서 수행하는 함수.
    vendor/libredwg/     실제 컴파일되는 업스트림 C 소스 부분집합 (아래 "빌드" 절)
    vendor-config/       config.h -- autotools 생성 산출물을 대신하는 손으로 쓴 파일
    examples/            smoke.rs -- raw FFI 수동 확인용 (아래 "테스트 구조" 절)
  uncad/                 안전한 API. dynapi.rs(리플렉션 헬퍼) -> convert.rs(raw Dwg_Data* ->
                         render_model.rs의 RenderEntity) -> tables.rs(LAYER/BLOCK_RECORD) ->
                         color.rs(ACI/BYLAYER 해석) -> svg.rs(to_svg())/png.rs(to_png())/
                         json.rs(to_json()) -> acis.rs(3DSOLID 실험적 와이어프레임) 순으로
                         레이어가 쌓인다. 읽기 전용 -- DWG/DXF 쓰기 경로는 없다.
    tests/               공개 API 통합 테스트 (dxf_pipeline.rs, acis_sab.rs)
    examples/            dump.rs / blocks.rs -- 수동 확인용
  uncad-cli/             CLI 바이너리 (uncad 명령)
    tests/               documented_invocations.rs -- README/--help가 광고하는 호출 전부
```

## 빌드: autotools 대신 `cc` 크레이트, 그리고 submodule 대신 vendor/ 복사본

`build.rs`가 `crates/libredwg-sys/vendor/libredwg/src/*.c`를 직접 컴파일한다(`configure`/
`autoreconf`/`libtool` 불필요). `crates/libredwg-sys/vendor-config/config.h`가 autotools의
생성 산출물을 대신하는 손으로 쓴 파일이다.

**왜 `lib/libredwg` submodule을 직접 안 쓰고 크레이트 안에 vendor/ 복사본을 따로 두는가**:
`cargo package`/`cargo publish`는 크레이트 디렉터리(`crates/libredwg-sys/`) 밖의 파일을 절대
포함하지 않고, crates.io에서 이 크레이트를 받는 소비자는 `.git`도 submodule도 아예 없다 --
`repo_root/lib/libredwg`를 참조하는 build.rs는 로컬 워크스페이스 안에서만 동작하고 발행된
크레이트에서는 100% 빌드 실패한다(실제로 `cargo publish --dry-run`으로 확인했던 실패
모드). `crates/libredwg-sys/vendor/libredwg/`는 이 크레이트가 실제 컴파일에 쓰는 파일만
(`.c` 24개 + 그게 실제로 `#include`하는 헤더/`.spec`/`.inc`/codepage 테이블 전체, 총
112개 파일 ~24MB/gzip 3MB -- `git ls-files crates/libredwg-sys/vendor | wc -l`) submodule에서
그대로 복사해 git으로 커밋해둔 것 -- 수정 없이
원본 그대로(`docs/THIRD_PARTY_NOTICES.md` 참고), 다만 담긴 파일 집합이 submodule 전체가
아니라 실제 사용 파일의 부분집합이라는 차이가 있다. `lib/libredwg` submodule 자체는 여전히
남아있다 -- 업스트림 갱신 시 diff 대상, 그리고 실 파일 기반 회귀 테스트(`uncad`의 `png.rs`,
`tests/dxf_pipeline.rs`, `tests/acis_sab.rs`와 `uncad-cli`의
`tests/documented_invocations.rs`)가 `lib/libredwg/test/test-data/`의 픽스처를 읽는다. 즉
submodule은 `cargo build`가 아니라 `cargo test`의 전제조건이다.

**submodule 갱신 절차**: `git submodule update --remote` 등으로 `lib/libredwg` 포인터를
옮긴 뒤에는 `scripts/sync-libredwg-vendor.sh`를 실행해서 vendor/ 복사본을 다시 만들어야
한다(실제 `#include` 그래프를 새로 추적해서 파일 목록을 재생성함). 그 다음
`cargo build --workspace`로 컴파일이 여전히 되는지 확인 -- 새 `.c`/헤더 파일이 필요해졌으면
컴파일 에러로 바로 드러난다(`build.rs`가 `vendor/libredwg/` 디렉터리 전체를
`cargo:rerun-if-changed`로 등록해두어서, 복사본이 바뀌면 증분 빌드에서도 C 재컴파일과 bindgen이
다시 돈다 -- `cargo clean` 불필요. 2026-09-15 전에는 shim/과 vendor-config/만 등록되어 있어
재벤더링 뒤 낡은 오브젝트를 그대로 쓰는 함정이 있었다). `build.rs`의 드리프트 감지 두 단계도
참고: (1)
`vendor/libredwg/src/*.c` 파일 개수를 `LIBREDWG_SOURCES` 기대값과 비교(둘이 다르면 vendor
복사본 자체가 손상/불일치 -- `panic!`), (2) `lib/libredwg` submodule이 체크아웃되어 있으면
(로컬 개발/CI에서만, 발행된 크레이트 소비자에게는 없음) 그 `.c` 파일 개수를 vendor 복사본과
교차 검증(다르면 재동기화가 필요하다는 `cargo:warning`만 내고 빌드는 계속 진행). bindgen이
생성하는 바인딩도 헤더가 바뀌었으면 같이 재검증(`cargo build --workspace`가 컴파일 에러로
알려줌)할 것.

## 테스트 구조: 유닛 / 통합 / 예제 세 계층

Rust 표준 배치를 그대로 따른다. 새 테스트를 어디에 놓을지는 **무엇에 접근해야 하는가**로 갈린다.

| 위치 | 컴파일 단위 | 접근 범위 | 용도 |
|---|---|---|---|
| `src/*.rs`의 `#[cfg(test)] mod tests` | 크레이트 내부 | private 포함 전체 | 픽스처 파일 없이 합성 데이터로 검증되는 순수 로직 |
| `tests/*.rs` | 파일마다 독립 크레이트 | 공개 API만 | 실 파일로 도는 end-to-end |
| `examples/*.rs` | 독립 바이너리 | 공개 API만 | 수동 확인 도구 + 사용 예시 |

**유닛(`#[cfg(test)]`)** -- private 헬퍼를 직접 부를 수 있다는 것이 이 자리의 존재 이유다. 외부 파일이
필요 없는 순수 함수(색상 해석, SVG 생성, SAT 파싱, outlier-trim 클러스터링 등)는 전부 여기 있고,
`cargo test`가 도는 테스트의 대부분을 차지한다. 예외가 하나 있다: `png.rs`의
`to_png_renders_a_real_dwg_to_a_valid_png`는 실 DWG를 읽는 end-to-end인데도 private `png_dimensions`를
써야 해서 유닛 자리에 있다.

**통합(`tests/`)** -- 파일마다 별개 크레이트로 컴파일되어 공개 API만 보이므로, 검증 범위가 "발행된
크레이트를 받은 사람이 할 수 있는 일"과 정확히 일치한다. 픽스처는 `lib/libredwg/test/test-data/`에서
읽는다(위 "빌드" 절 -- submodule은 `cargo build`가 아니라 `cargo test`의 전제조건이다).
`uncad-cli`의 `tests/`는 라이브러리가 아니라 빌드된 바이너리를 실제로 실행한다.

**예제(`examples/`)** -- 어서션이 없다. 대신 `cargo test`와 `cargo clippy --workspace --all-targets`가
이들을 컴파일하므로, 공개 API 시그니처가 깨지면 CI에서 빌드 에러로 드러난다. 즉 "실행되는 문서"이자
컴파일 가드다. 셋 다 인자로 받은 파일 경로로 동작한다 -- `samples/`에 아무 DWG/DXF나 넣고 돌려보는
용도다(`samples/README.md`).

```bash
cargo run -p uncad --example dump <file>            # 엔티티 덤프
cargo run -p uncad --example blocks <file> [이름]   # 블록 레코드 목록/내용
cargo run -p libredwg-sys --example smoke <file>    # raw FFI로 읽어 오브젝트 수만 출력
```

**단언 원칙**: 실 파일 테스트는 기대값을 고정하지 않는다. 라운드트립(도면 자신이 기댓값)이나 "옵션이
결과를 실제로 바꾸는가" 같은, 파일이 바뀌어도 성립하는 성질만 단언한다. 이 프로젝트 자신의 출력에서
베낀 숫자를 박아둔 예전 테스트(`crates/uncad/tests/core.rs`)가 다른 파일로는 기대값을 재생성할 방법이
없어 픽스처와 함께 통째로 삭제된 전례가 있다 -- `samples/README.md` 참고.

현재 무엇이 얼마나 돌고 있는지(테스트 개수, 파일별 커버리지 내역, 아직 자동화되지 않은 것)는
`docs/CAVEATS.md`의 "파일 기반 회귀 테스트는 소수" 절에 있다. 여기서는 배치 규칙만 다룬다.

## FFI 경계: opaque 타입 + dynapi 리플렉션

`Dwg_Object`의 `tio` 필드는 ~90개 `Dwg_Entity_*`/`Dwg_Object_*` 타입을 묶는 C union인데, 이게
bindgen의 구조체 코드생성을 실패시킨다(clang 자체는 문제없이 파싱/`sizeof()` 계산함 -- bindgen의
레이아웃 코드생성 단계만 실패). 그래서 `Dwg_Data`/`Dwg_Object`와 그 하위 타입들을 전부
`.opaque_type()`으로 지정해뒀다.

이 프로젝트는 애초에 엔티티 필드에 raw struct 접근을 할 계획이 없었다 -- 항상
`dwg_dynapi_entity_value`/`dwg_dynapi_common_value`(LibreDWG 자체의 문자열 필드명 기반
리플렉션 API, 런타임 타입/범위 체크 포함)를 거치기로 설계했기 때문에, opaque 처리는 오히려
원래 설계와 자연스럽게 맞아떨어진다. `uncad::dynapi`가 `dynapi_field::<T>`/
`get_common_field::<T>`/`get_array_field::<T>` 제네릭 헬퍼로 이걸 감싸고, debug 빌드에서는
dynapi가 보고하는 실제 필드 크기와 요청한 Rust 타입 크기를 비교하는 assert를 넣어서 타입
매핑 실수를 조용한 데이터 오염이 아니라 즉시 패닉으로 드러낸다.

**같은 bindgen 구조체 코드생성 실패가 개별 타입에서도 재발한다**: `Dwg_Object`의 tio union
전체를 opaque 처리해도, 그 바깥에 있는 개별 nested struct 타입(`Dwg_HATCH_Path`/
`Dwg_HATCH_PathSeg`/`Dwg_HATCH_ControlPoint`, `Dwg_MLINE_vertex`)을 따로 allowlist하면
똑같은 실패가 재발한 적이 여러 번 있다 -- allowlist만 하면 `layout_tests()` assert가 항상
실패하는 자기모순적 크기(예: `1usize - 96usize`)를 내는 걸로 알아챌 수 있다. 표준 대응:
`crates/libredwg-sys/build.rs`에서 `.blocklist_type()`으로 막고, `src/lib.rs`에 `dwg.h`와
정확히 같은 필드 순서/타입으로 손으로 `#[repr(C)]` 구조체를 다시 쓴 뒤, clang이 실제로
계산한 `sizeof()`와 대조하는 컴파일타임 어서션을 추가한다(`build.rs`가 blocklist 전에
bindgen 스스로 생성했던 `layout_tests()`가 그 진짜 크기를 알려준다). 참조 struct가 필요
없는 필드(예: `Dwg_MLINE_vertex.lines`)는 실제 타입 대신 `*mut c_void`로 남겨서 연쇄적으로
더 많은 타입을 손으로 옮겨 적을 필요를 피한다.

**크로스플랫폼 enum 폭 문제**: `dwg_object_get_fixedtype`의 실제 C 선언은 `int`를 반환하는데
(`DWG_OBJECT_TYPE`이 아니라) -- `dwg_api.h` 자체에 있는 선언 불일치다. bindgen이 추론하는
`DWG_OBJECT_TYPE`의 내부 표현이 플랫폼별 clang의 C enum 기본 정수 타입 선택에 좌우되는데,
실제로 MSVC 타겟에서는 `i32`, `x86_64-unknown-linux-gnu`(gcc)에서는 `u32`로 갈렸다(Docker
`rust:latest`로 실제 컴파일해서 확인). `fixedtype` 값을 다루는 모든 FFI 호출부에서 호출 즉시
`as DWG_OBJECT_TYPE`으로 캐스팅해서 이 문제를 흡수한다 -- 이후 코드는 항상 하나의 정규 타입만
비교하면 된다.

## 스레드 세이프티

LibreDWG C 라이브러리는 스레드 세이프하지 않다(`loglevel` 등 non-reentrant 전역 상태). `uncad`
크레이트는 모든 FFI 진입점을 프로세스 전역 `Mutex`(poison 시 복구해서 계속 사용)로 직렬화해서
안전한 공개 API를 제공한다. `libredwg-sys`를 직접 쓴다면 이 제약을 스스로 지켜야 한다 -- 동시
호출 시 `STATUS_HEAP_CORRUPTION`으로 재현된 적 있음.

## 모델: `RenderEntity`/`Tables` 하나뿐 -- `Dwg_Data`는 `parse()` 안에서만 산다

`CadDatabase`는 `entities`(모델/페이퍼 스페이스가 소유한 엔티티)와 `tables`(LAYER, 모든
BLOCK_RECORD, MLINESTYLE)만 들고 있는 순수 Rust 값이다(`Debug`/`Clone`/`PartialEq`/
`serde::Serialize`/`Deserialize`, 직접 생성 가능). LibreDWG가 `dwg_read_file`/`dxf_read_file`로
채운 `Dwg_Data`는 `parse()` 안에서 두 번 순회(`convert_entities`, `convert_tables`)된 직후
`dwg_free`로 해제되고 반환값에 남지 않는다. 즉 "DWG/DXF -> 공통 model -> 여러 출력"의 허브는 이
Rust 모델이고, 출력은 `to_json()`(모델을 serde로 그대로 직렬화, `json.rs`), `to_svg()`,
`to_png()`(SVG를 래스터화) 셋이다.

이 모델은 의도적으로 손실이 있다 -- 렌더링에 필요한 필드만 남긴다(선종류·선굵기·레이어 on/off·
텍스트 스타일·오브젝트 사전·헤더 변수 등은 없음). 그래서 DWG/DXF를 다시 써내는 용도로는 쓸 수
없고, 이 프로젝트는 쓰기를 제공하지 않는다(0.1.0의 `write_dwg`/`write_dxf`/`dwg_to_dxf`는
2026-09-15에 제거 -- `CHANGELOG.md`). 2026-08-07부터 제거 전까지는 쓰기를 위해 `Dwg_Data`를
`CadDatabase` 수명 동안 살려 두는 "두 계층" 구조였다.

`libredwg-sys`의 C 빌드에는 그래도 인코더(`encode.c` 등)가 포함되고 `config.h`의 `USE_WRITE`도
켜져 있다: `dwg.c`가 `dxf_read_file()`을 `USE_WRITE`로 가드하고, `in_dxf.c`가 `encode.c`의
핸들 후처리 헬퍼를 쓰며, `out_dxf.c`가 3DSOLID 와이어프레임에 필요한 `dwg_convert_SAB_to_SAT1`을
담고 있기 때문이다. Rust로 바인딩되는 쓰기 진입점은 없다(`dwg_write_file`은 allowlist에서 뺐고
DXF 쓰기 심은 삭제).

## 엔티티 모델과 블록 기반 순회

`CadDatabase::entities`는 전체 오브젝트를 fixedtype으로 분류하는 전역 스캔이 아니라, 다음
순서로 만들어진다:

1. `BLOCK_HEADER` 오브젝트를 순회하며 이름이 `*Model_Space` 또는 `*Paper_Space*`(대소문자 무시)와
   일치하는 것만 고른다.
2. 그 블록이 소유한 엔티티만 `get_first_owned_entity`/`get_next_owned_entity`(타입 무관 범용
   이터레이터, LibreDWG의 실제 EXPORT 함수)로 순회한다.

INSERT가 참조하는 "블록 정의" 안의 엔티티는 그 블록 자신의 `BLOCK_RECORD.entities`로만 접근
가능하고 최상위 `entities`에는 절대 안 들어간다 -- 전역 스캔 방식은 이걸 구분하지 못해서 실제로
없어야 할 엔티티가 새어 들어오는 버그를 낸 적이 있다.

INSERT의 ATTRIB은 최상위 `entities`에는 중복으로 들어가지만(도면에 실제로 그려지므로), 그
INSERT가 속한 블록 자신의 `entities` 목록에는 중복되지 않는다.

`BLOCK_HEADER.name`은 익명 블록(DIMENSION 캐시용 `*D` 등)에서 줄임 이름만 담고 있다 -- 진짜
구분되는 이름(`*D30` 등)은 그 블록이 소유한 `BLOCK` 엔티티 자신의 `name` 필드에만 있어서,
`BLOCK_HEADER`의 `block_entity` 핸들 필드로 직접 그 `BLOCK` 엔티티를 찾아 이름을 읽어야 한다.
(`tables::resolve_block_name`이 INSERT/DIMENSION 둘 다에서 이 로직을 공유한다.)

## 3DSOLID/REGION ACIS 와이어프레임 (`acis.rs`)

`crates/uncad/src/acis.rs`는 범용 ACIS/B-rep 파서가 아니라, ACIS SAT(v1, ASCII) 텍스트에서
`edge` 레코드 하나당 두 끝점을 잇는 직선만 뽑아내는 최소 와이어프레임 추출기다. Spatial의
공개 "SAT Save File Format" 문서(예: paulbourke.net/dataformats/sat에 오래 미러링된 버전)를
레코드/필드 *의미*를 이해하는 참고 자료로만 썼을 뿐, 그 문서의 코드나 텍스트를 그대로
재사용하지 않았다 -- 독자적으로 새로 작성한 구현이다. 곡선 엣지는 현(chord)으로 근사하고,
면/서피스는 아예 해석하지 않는다(항상 와이어프레임만 나옴, 채워진 solid는 안 나옴).

SAB(v2, 바이너리)로 저장된 솔리드는 SAT 텍스트로 먼저 변환해야 하는데, LibreDWG의
`dwg_convert_SAB_to_SAT1`은 엔티티를 **제자리에서** 바꾼다(`version`을 1로, `encr_sat_data`에
평문 SAT를 채우고 `acis_data`는 SAB 바이트 그대로 둠). `parse()`는 같은 솔리드를 두 번
읽으므로(`convert_entities`의 모델 스페이스 순회, 그다음 `convert_tables`의 블록 레코드 순회)
살아있는 엔티티에 호출하면 두 번째 읽기가 `version == 1` 분기에서 SAB 바이너리를 SAT 텍스트로
파싱해 와이어프레임을 잃는다(제거된 쓰기 경로에서는 같은 변이가 쓰기 결과까지 깨뜨렸다 --
2026-09-15 실 파일로 확인, `docs/CAVEATS.md`의 "3DSOLID SAB 변환" 절). 그래서 `libredwg-sys`의
`uncad_3dsolid_sab_to_sat_text` 심이 엔티티의 얕은 복사본에서 변환을 돌리고 텍스트만 돌려준다
-- `parse()`는 `Dwg_Data`에 아무 부작용도 남기지 않는다.

`extract_wireframe(entity_ptr, dxfname)`가 `dxfname`을 인자로 받는 이유: REGION은 `dwg.h`에서
`Dwg_Entity__3DSOLID`의 typedef라 3DSOLID와 완전히 같은 구조체/dynapi 필드 테이블을 쓰지만,
`dwg_dynapi_entity_value`가 호출 시 넘긴 이름과 오브젝트의 실제 `dxfname`을 엄격히 대조해서
다르면 조용히 실패한다(`obj->name`이 "REGION"인데 "3DSOLID"를 넘기면 모든 필드 읽기가
실패) -- 두 타입이 구조적으로 동일해도 이름은 하드코딩할 수 없다. `docs/CAVEATS.md`의
"MULTILEADER/MLINE/REGION/POLYLINE_PFACE" 문단 참고.
