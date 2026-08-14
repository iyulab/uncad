# 아키텍처

## 크레이트 구조

```
lib/libredwg/            LibreDWG C 소스 -- 진짜 업스트림(github.com/LibreDWG/libredwg)을 가리키는
                         git submodule. 수정 없이 그대로 씀.
crates/
  libredwg-sys/          raw FFI: build.rs가 lib/libredwg/src/*.c를 cc 크레이트로 직접 컴파일
                         (autotools 없이) + bindgen으로 바인딩 생성. shim/uncad_shim.c는 공개
                         헤더에 없는 내부 함수(dwg_write_dxf 등)를 감싸는 C 실드 + dynapi로
                         도달 못 하는 중첩 구조체(MULTILEADER 리더 라인 등)를 순회해서 평평한
                         배열로 넘겨주는 전용 함수들.
  uncad/                 안전한 API. dynapi.rs(리플렉션 헬퍼) -> convert.rs(raw Dwg_Data* ->
                         render_model.rs의 RenderEntity) -> tables.rs(LAYER/BLOCK_RECORD) ->
                         color.rs(ACI/BYLAYER 해석) -> svg.rs(to_svg()) -> acis.rs(3DSOLID
                         실험적 와이어프레임) 순으로 레이어가 쌓인다.
  uncad-cli/             CLI 바이너리 (uncad 명령)
```

## 빌드: autotools 대신 `cc` 크레이트

`build.rs`가 `lib/libredwg/src/*.c`를 직접 컴파일한다(`configure`/`autoreconf`/`libtool` 불필요).
`crates/libredwg-sys/vendor-config/config.h`가 autotools의 생성 산출물을 대신하는 손으로 쓴 파일이다.

**드리프트 감지**: `build.rs`는 `lib/libredwg/src/*.c` 파일 개수를 하드코딩된 기대값과 비교해서
다르면 빌드를 그대로 `panic!`시킨다. `lib/libredwg` submodule 포인터를 옮길 때(`git submodule
update --remote` 등) 새로 추가/삭제된 `.c` 파일이 있으면 컴파일 목록이 조용히 stale해지는 걸
막기 위함 -- panic 메시지가 나오면 새 파일 목록을 확인하고 `LIBREDWG_SOURCES`/
`EXCLUDED_JSON_SOURCES`를 갱신한 뒤 진행해야 한다. bindgen이 생성하는 바인딩도 헤더가 바뀌었으면
같이 재검증(`cargo build --workspace`가 컴파일 에러로 알려줌)할 것.

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

## 두 계층 모델: `Dwg_Data`(진짜 허브) vs `RenderEntity`/`Tables`(렌더링 전용 투영)

`CadDatabase`는 사실 두 개의 서로 다른 "모델"을 동시에 들고 있다:

- **`dwg: Box<Dwg_Data>`** (비공개 필드) -- LibreDWG 자신이 `dwg_read_file`/`dxf_read_file`로
  채운, 완전하고 왕복 가능한(round-trip-faithful) 원본 그대로다. `parse()`는 예전(2026-08-07
  이전)엔 변환 직후 이걸 바로 `dwg_free`했지만, 이제는 `CadDatabase`의 수명 동안 살려서
  들고 있다가 `write_dwg`/`write_dxf`가 그대로 다시 써낸다(`Drop for CadDatabase`가 해제).
  새 무손실 Rust 모델을 따로 만들 필요가 없었던 이유가 이거다 -- LibreDWG 자신의 구조체가
  이미 그 역할을 한다.
- **`entities`/`tables`** (공개 필드) -- `to_svg()` 렌더링에 필요한 필드만 남긴, 의도적으로
  손실 있는 Rust 투영. 아래 "엔티티 모델과 블록 기반 순회" 절에서 설명하는 게 전부 이쪽이다.
  `write_dwg`/`write_dxf`는 이 모델을 전혀 안 거친다 -- 여기서 역변환하면 애초에 렌더링에
  안 쓰이는 필드(전체 테이블, 오브젝트 사전, 헤더 변수, 스타일 정의 등)가 다 빠진 반쪽짜리
  DWG/DXF가 나올 것이다.

즉 사용자가 원래 그렸던 "여러 입력 포맷 -> 공통 model -> 여러 출력 포맷" 허브 구조에서, 진짜
허브는 `RenderEntity`가 아니라 `Dwg_Data`다. `RenderEntity`는 그 허브 위에 얹힌, SVG 전용 파생
뷰(view)일 뿐이다. `docs/CAVEATS.md`의 "DWG/DXF 쓰기 지원" 섹션에 실측 결과(R_2004 제약,
9개 fixture 중 5개만 `write_dwg` 성공 등)가 있다.

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

`extract_wireframe(entity_ptr, dxfname)`가 `dxfname`을 인자로 받는 이유: REGION은 `dwg.h`에서
`Dwg_Entity__3DSOLID`의 typedef라 3DSOLID와 완전히 같은 구조체/dynapi 필드 테이블을 쓰지만,
`dwg_dynapi_entity_value`가 호출 시 넘긴 이름과 오브젝트의 실제 `dxfname`을 엄격히 대조해서
다르면 조용히 실패한다(`obj->name`이 "REGION"인데 "3DSOLID"를 넘기면 모든 필드 읽기가
실패) -- 두 타입이 구조적으로 동일해도 이름은 하드코딩할 수 없다. `docs/CAVEATS.md`의
"MULTILEADER/MLINE/REGION/POLYLINE_PFACE" 문단 참고.
