# 알려진 제한 / 주의사항

## 엔티티 타입 커버리지

`parse()`/`to_svg()`가 지원하는 타입: LINE, CIRCLE, ARC, ELLIPSE, LWPOLYLINE, TEXT, POINT,
SOLID, RAY, XLINE, INSERT(재귀적 블록 참조 렌더링 포함), ATTRIB, ATTDEF, VIEWPORT, 3DFACE,
SPLINE, MTEXT, POLYLINE3D, DIMENSION(7개 하위타입 전부 하나의 타입으로 통합 -- ALIGNED/
ANG2LN/ANG3PT/DIAMETER/LINEAR/ORDINATE 6개는 JS baseline도 인식하던 것, ARC_DIMENSION은
JS baseline이 인식하지 않던 7번째지만 나머지 6개와 완전히 같은 `DIMENSION_COMMON` 레이아웃
(같은 `block` 핸들 필드)을 공유해서 새 렌더링 로직 없이 기존 메커니즘만 한 케이스 더 인식
하도록 확장 -- MULTILEADER/MLINE 같은 "새 기능"이 아니라 낮은 리스크의 기계적 확장), HATCH(경계
패스가 polyline이든 line/arc/ellipse/spline 엣지 리스트든 전부 지원. 패턴 채우기도 실제로
재현함 -- `Dwg_HATCH_DefLine`을 읽어 SVG `<pattern>` 타일로 렌더링, 자세한 내용과 발견된 버그는
아래 "HATCH 패턴 채우기" 섹션 참고. 솔리드 채우기는 반투명 색으로 채움. 그라디언트 채우기도
SVG `linearGradient`/`radialGradient`로 근사 렌더링함 -- 실 파일로 검증 안 됨, 아래 "HATCH
그라디언트 채우기" 섹션 참고), 3DSOLID(실험적 --
`docs/ARCHITECTURE.md`의 ACIS 와이어프레임
섹션 참고. B-rep 자체를 해석하지 않는 근사치이며, ACIS 데이터를 읽거나 변환하지 못하는 솔리드는
그냥 미지원으로 리포트됨), LEADER(단순 LEADER만.
정점을 잇는 폴리라인 + 옵션인 첫 정점 화살표만 그림, 스플라인 경로/텍스트 박스 크기 등 렌더링에
안 쓰이는 필드는 JS baseline과 마찬가지로 모델에 없음), POLYLINE_2D(LWPOLYLINE과 같은
`LwPolylineEntity` 셰이프를 재사용 -- `RenderEntity::XLine`이 `RayEntity`를 재사용하는 것과 같은
패턴. `to_svg()` 렌더링도 LWPOLYLINE과 완전히 같은 코드 경로 공유, JS baseline도 동일),
MULTILEADER/MLINE/REGION/POLYLINE_PFACE/TOLERANCE/ACAD_TABLE/WIPEOUT/LIGHT(**전부 실험적,
JS baseline에 없던 새 기능** -- 아래 별도 문단 참고).

아직 안 됨: ACAD_PROXY_ENTITY 하나뿐. 다른 프로그램이 만든, 이 라이브러리가 모르는
커스텀 엔티티의 프록시 표현이라 애초에 렌더링 가능한 고정된 지오메트리가 없다 --
`proxy_id`/`class_id`/직렬화된 엔티티 바이트 데이터만 있지, 좌표나 형태가 없다. `Unknown`
으로 남겨두는 것 자체가 정확한 표현이고, 여기에 "지원"을 추가해도 실질적으로 달라지는 게
없다(`Unknown`도 실제 dxfname을 보존하므로 CLI summary 카운트에는 이미 정확한 이름 "ACAD_
PROXY_ENTITY"로 잡힌다). `parse()` 단계에서 미지원 타입은 `RenderEntity::Unknown`(실제
dxfname 보존, 조용히 버리지 않음)으로, `to_svg()`에서는 `unsupported_entity_types`로
리포트된다.

**엔티티 커버리지 상황 요약**: `to_svg()`는 이제 JS baseline의 `toSVG()`가 실제로 `case`를
갖고 있던 모든 타입(LEADER/POLYLINE_2D 포팅 완료 시점, 2026-08-05)을 커버한 데 더해,
JS baseline이 아예 다루지 않았던 8개 타입(MULTILEADER, MLINE, REGION, POLYLINE_PFACE,
TOLERANCE, ACAD_TABLE, WIPEOUT, LIGHT)까지 새 기능으로 추가했다. 유일하게 안 되는 건
지오메트리 자체가 없는 ACAD_PROXY_ENTITY, 그리고 JS baseline도 파싱 안 하던
REGION/LIGHT/POLYLINE_PFACE/ARC_DIMENSION 4개 중 REGION/LIGHT/POLYLINE_PFACE는
새 기능으로 이미 추가했고 ARC_DIMENSION은 기존 DIMENSION 메커니즘의 저위험 확장으로
추가했다(둘 다 위 커버리지 목록 참고) -- 결과적으로 파싱 가능한 모든 엔티티 타입 중
지오메트리가 있는 타입은 전부 다뤄진 상태다.

### 8개 타입은 예외적으로 JS baseline에 없던 새 기능이다

(MULTILEADER, MLINE, REGION, POLYLINE_PFACE, TOLERANCE, ACAD_TABLE, WIPEOUT, LIGHT)

`entityConverter.ts`도 `toSVG()`도 MULTILEADER를 다루지 않았다 -- 포팅할 JS 동작 자체가
없었다. 사용자 요청으로 새로 구현했고, 스코프를 의도적으로 좁게 잡았다: `ctx.leaders[].
lines[].points`(리더 라인/스플라인 기하 정보만, 스플라인은 SPLINE/HATCH 곡선 엣지와 같은
직선 근사)만 그리고, `ctx.content`(MTEXT/블록 콘텐츠)는 추출도 렌더링도 하지 않는다 --
3DSOLID의 와이어프레임 전용 렌더링, VIEWPORT의 프레임 전용 렌더링과 같은 "최선을 다한
근사치" 선례를 따른 것. 화살표는 각 리더 라인 끝에 항상 그린다 -- 실제
`LEADER_Line.flags`의 "화살표 표시" 비트는 이 기능이 쓰는 지오메트리 전용 C 셔틀
(`uncad_multileader_get_lines`, `libredwg-sys/shim/uncad_shim.c`)에서 추출하지 않기
때문.

**실제 파일로 검증 안 됨**: 스팟 체크에 쓴 실 AutoCAD 도면 9개 중 MULTILEADER를 포함한
파일이 하나도 없어서, `cargo build`/`cargo clippy` 클린과 코드 리뷰(`dwg.h`/`dynapi.c`의
`Dwg_MLEADER_AnnotContext`/`Dwg_LEADER_Node`/`Dwg_LEADER_Line` 실제 필드 레이아웃과
셔틀 코드를 직접 대조)로만 확인했다 -- 실 파일에서의 동작은 아직 미확인.

MULTILEADER의 중첩 구조(`entity.ctx.leaders[].lines[].points[]`)는 dynapi의 평평한
`dwg_dynapi_entity_value`로 도달할 수 없고(최상위 entity 필드만 이름으로 노출), 이 구조체들
(`Dwg_MLEADER_AnnotContext` 등)을 Rust에서 직접 bindgen하는 것도 HATCH_Path/PathSeg가 이미
겪은 것과 같은 bindgen 구조체 코드생성 실패 위험이 있어(`crates/libredwg-sys/build.rs`의
블록리스트 주석 참고) 피했다. 대신 `uncad_shim.c`에 전용 C 함수를 추가해 실제 `dwg.h` 구조체
레이아웃이 그대로 보이는 vendored C 소스 안에서 순회하고, 평평한 `(x,y,z)` 배열만 Rust로
넘긴다.

MLINE도 `entityConverter.ts`/`toSVG()` 둘 다 다룬 적 없는 새 기능이다. MLINE은 실제로는
`num_lines`개의 평행선(벽 스타일 다중선)이고, 이제(2026-08-07) 각 선의 실제 오프셋 거리를
MLINE이 참조하는 MLINESTYLE 오브젝트에서 읽어와 진짜 오프셋 폴리라인으로 그린다 -- 이전에는
MLINESTYLE을 파싱하지 않아서 각 정점의 `vertex`(중심선)만 잇는 단일 폴리라인으로 근사했었다.
`Dwg_MLINE_vertex`(정점 배열의 원소 타입)도 MULTILEADER의 서브타입들과 같은 bindgen
구조체 코드생성 실패를 겪어서 `crates/libredwg-sys/src/lib.rs`에 `dwg.h`와 정확히 같은
레이아웃으로 직접 손으로 정의했다 -- clang의 진짜 `sizeof()`(96바이트)와 대조하는
컴파일타임 어서션 포함. (Linux/gcc Docker 빌드로 발견한 것: `Dwg_MLINE_vertex`를
블록리스트하는 것만으로는 부족했다 -- Windows/MSVC와 달리 Linux 타겟에서는 bindgen이
`Dwg_MLINE_line`도 함께 생성하면서 블록리스트된 `_dwg_MLINE_vertex`를 참조하는 컴파일
에러가 났다. `Dwg_MLINE_line`도 같이 블록리스트해서 해결 -- 아래 "Windows/Linux
크로스플랫폼" 섹션의 다른 bindgen 차이와 같은 종류.)

**MULTILEADER와 마찬가지로 실제 파일로 검증 안 됨**: 스팟 체크에 쓴 9개 파일 중 MLINE을
포함한 파일이 여전히 하나도 없다(2026-08-07 재확인) -- 아래 "MLINESTYLE 파싱" 섹션 참고.

REGION은 `dwg.h`에서 `typedef Dwg_Entity__3DSOLID Dwg_Entity_REGION`으로, `dynapi.c`의
`"REGION"` 엔트리도 `"3DSOLID"`와 완전히 같은 필드 테이블(`_dwg_3DSOLID_fields`)을 가리키는
-- 말 그대로 바이트 단위로 동일한 구조체다. 그래서 `acis.rs`의 기존 3DSOLID ACIS
와이어프레임 추출 로직을 그대로 재사용할 수 있었다 -- 단, dynapi가 호출 시 전달한 타입
이름과 오브젝트의 실제 dxfname을 엄격히 대조하기 때문에(`dwg_dynapi_entity_value`가
`obj->name`과 인자로 받은 이름이 다르면 조용히 실패), `"3DSOLID"`를 하드코딩할 수 없어
`extract_wireframe()`이 dxfname을 인자로 받도록 바꿨다. 렌더링도 3DSOLID와 완전히 같은
등각 와이어프레임 경로 공유(`render_wireframe` 헬퍼로 통합).

POLYLINE_PFACE("polyface mesh")는 LibreDWG 자신의 전용 접근자
(`dwg_ent_polyline_pface_get_points`)가 `dwg_api.h`에 `/* not implemented. use the dynapi
instead */`라고 명시되어 있어, POLYLINE_3D 때처럼 전용 C 함수를 그대로 호출할 수 없었다.
대신 `VERTEX_PFACE`(정점 위치)/`VERTEX_PFACE_FACE`(최대 4개 정점 인덱스로 이루어진 면
레코드) 서브엔티티 체인을 `get_first_owned_subentity`로 직접 순회해서, 각 면의 정점
인덱스를 잇는 와이어프레임 엣지로 변환한다 -- REGION과 마찬가지로 3DSOLID의 등각 렌더링
경로를 그대로 재사용(폴리페이스 메시도 본질적으로 3D 형상이라는 점에서 동일 취급).

**REGION/POLYLINE_PFACE도 실제 파일로 검증 안 됨**: 스팟 체크에 쓴 9개 파일 중 어느 쪽도
포함한 파일이 없었다 -- `cargo build`/`cargo test`/`cargo clippy`가 MSVC/Linux(Docker)
양쪽에서 클린하다는 것과 `dwg.h`/`dynapi.c` 실제 레이아웃 대조로만 확인했다.

TOLERANCE는 ATTRIB/TEXT와 완전히 같은 셰이프(위치 + 텍스트)로 그린다 -- 다만 `text_value`는
GD&T feature-control-frame 전용 포맷 코드(`%%v` 류)를 그대로 담고 있고, 이 프로젝트는 그걸
파싱/스트리핑하지 않는다(MTEXT의 `strip_mtext_formatting`과 달리 전용 스트리퍼가 없음) --
읽을 수는 있지만 진짜 GD&T 기호로 렌더링되지는 않는, "정직하지만 불완전한" 근사치.

ACAD_TABLE은 `dwg.h`에서 INSERT와 거의 같은 필드 셰이프(`ins_pt`/`scale`/`rotation`/
`block_header`)를 갖고 있고, 자기 자신의 `flag_for_table_value` 필드 주석이 "0x06(블록 있음)
은 보통 항상 세팅된다"고 명시하고 있어서, `num_cols`/`num_rows`/`col_widths`/`row_heights`
등에서 셀 그리드를 직접 계산하는 대신 INSERT/DIMENSION과 똑같이 `block_header`가 가리키는
캐시된 블록을 `render_block_ref`로 그린다 -- 셀 내용을 재구성하는 게 아니라 AutoCAD 자신이
이미 계산해 둔 지오메트리를 그대로 재사용하는 것이므로, 이 8개 중에서는 REGION만큼이나
구조적으로 신뢰도가 높다. dynapi 필드 테이블 조회 이름은 `"TABLE"`이지 실제 DXF 이름인
`"ACAD_TABLE"`이 아니다(REGION/3DSOLID와 같은 종류의 이름 불일치 -- `convert.rs`의
`DWG_TYPE_TABLE` 케이스 주석 참고).

WIPEOUT은 이 8개 중 유일하게 **"실 파일로 검증 안 됨"을 넘어서는 추가 리스크**가 있다:
`pt0 + u*uvec + v*vvec` 픽셀-공간-투월드 변환이 표준 DXF 이미지 엔티티 관례(삽입점 + U/V
픽셀 벡터 + 그 픽셀 공간 안의 클립 정점)에 대한 일반 지식에 기반한 것이지, LibreDWG 자신의
소스 코드로 확인된 게 아니다(LibreDWG는 이미지를 렌더링하지 않고 필드만 읽고 쓰므로, 대조할
참조 구현이 이 코드베이스 안에 아예 없음). `docs/ARCHITECTURE.md`나 `dwg.h`의 필드 주석
어디에도 이 변환식 자체를 확인해 줄 근거가 없다 -- `crates/uncad/src/convert.rs`의
`wipeout_boundary` 함수와 `WipeoutEntity`의 doc 주석 참고. 렌더링은 채우지 않고 외곽선만
그린다(다른 지오메트리를 가리는 불투명한 흰 박스가 되는 리스크를 피하기 위함).

LIGHT는 이 8개 중 유일하게 JS baseline이 파싱조차 안 하던 타입이다(TOLERANCE/ACAD_TABLE/
WIPEOUT은 최소한 파싱은 됐었음). 광원은 애초에 "그려지는" 지오메트리가 없어서(2D 평면도에서
안 보이는 게 정상), `position`에 작은 원 마커 + (거리/스팟 조명일 때만) `target`까지 점선을
그리는 임의의 placeholder다 -- VIEWPORT의 프레임 전용 렌더링과 같은 정신이지만, VIEWPORT는
적어도 실제 뷰포트 경계라는 의미가 있는 반면 LIGHT 마커는 "여기 광원이 있다"는 사실 말고는
AutoCAD의 실제 시각적 표현과 무관하다.

**TOLERANCE/ACAD_TABLE/WIPEOUT/LIGHT도 실제 파일로 검증 안 됨**: 스팟 체크에 쓴 9개 파일
중 넷 다 포함한 파일이 없었다 -- MSVC/Linux(Docker) 양쪽 `cargo build`/`cargo test`/
`cargo clippy` 클린과 `dwg.h`/`dynapi.c` 레이아웃 대조로만 확인했다.

(2026-08-05, 실제 AutoCAD 도면 9개로 스팟 체크: LEADER(8/9 파일)와 POLYLINE_2D(2/9 파일)가
발견된 미지원 타입 중 가장 흔했고, 둘 다 이후 포팅 완료. ACAD_PROXY_ENTITY는 1개 파일에서만
등장 -- 왜 계속 미지원인지는 위 "아직 안 됨" 문단 참고. 나머지 지원 타입은 전부 정상
파싱/렌더링됨.)

## DXF 읽기는 LibreDWG 자체의 한계를 그대로 물려받음

`dxf_read_file()`은 LibreDWG 자체 문서가 "대부분의 오브젝트에서 동작"이라고 밝히고 있어 DWG
읽기만큼 완전하지 않다(LWPOLYLINE이 포함된 실제 `.dxf` 파일에서 조용히 빠지는 걸 확인한 적
있음 -- ARC/ELLIPSE는 정상 동작). 이 프로젝트가 손댈 수 있는 부분이 아니라 업스트림 한계다.

## LWPOLYLINE/POLYLINE3D의 "closed" 판정

`dwg.h`의 필드 주석은 flag의 512번 비트를 "closed"라고 적어두었지만, 실제 렌더링 로직은 1번
비트(`flag & 1`, 표준 DXF group-70 관례)를 확인한다 -- `crates/uncad/src/convert.rs`의
`LWPOLYLINE_CLOSED_FLAG` 상수 주석 참고. 어느 쪽이 실제로 맞는지 검증할 실제 AutoCAD 참조가
없어서, 이 프로젝트의 오래된 JS/WASM predecessor가 이미 검증 없이 그렇게 구현했던 걸 그대로
포팅했다 -- 임의로 "고치지" 않았다.

## MTEXT 회전은 항상 0

`dwg.h`는 `x_axis_dir` 필드가 "회전을 정의한다"고 적어두었지만, `x_axis_dir`에서 각도를
계산하는 게 기술적으로는 더 정확해 보여도, 검증된 근거(실제 AutoCAD 참조) 없이 predecessor와
다르게 구현하지 않고 `0`으로 고정했다.

## LAYER 색상: `Dwg_Color.rgb`는 못 믿고, `color_index`도 그대로는 못 믿음

`Tables`는 레이어별로 `Dwg_Color.rgb`를 노출하지 않는다. 구버전 `lib/libredwg` 기준으로는
이 필드가 실제 레이어 색상과 무관하게 항상 `0xFFFFFF`(상수 placeholder)로 나오는 걸 실측
확인했었다(22개 실제 파일 대조, 원래 JS/WASM predecessor 시절). BYLAYER 색상 해석은
`color_index`를 통해서만 하고 `rgb`는 절대 신뢰하지 않는다는 원칙은 지금도 유효하다 --
`crates/uncad/src/color.rs`의 `bylayer_resolves_through_layer_colorindex_not_layer_rgb`
회귀 테스트가 이 버그를 잡는다.

2026-08-13 서브모듈을 최신 upstream으로 올린 뒤(`e405fcff` -> `a8ce2489`)로는 `rgb`가 더는
항상 `0xFFFFFF`가 아니다. 2026-08-14, `samples/`의 9개 실 파일을 PNG로 변환해서 눈으로 보니
전부 검정 하나로만 나오는 걸 발견(SVG 단계부터 이미 그랬음, PNG 변환 자체의 문제는 아니었음)
-- 원인을 `lib/libredwg/src/bits.c`의 `bit_read_CMC`까지 추적:

이 9개 파일은 전부 AC1018(R2004) 네이티브 포맷이고, LAYER 색상이 `method=0xc3`(TRUECOLOR)로
저장돼 있는데 `rgb` 값이 `0x000001`~`0x000008`처럼 비정상적으로 작다. `bit_read_CMC`는 읽은
뒤 `color->index = dwg_find_color_index(color->rgb)`로 ACI 팔레트 역매칭을 시도하는데, 이런
작은 `rgb`는 어떤 팔레트 엔트리와도 정확히 일치하지 않아 매칭 실패 시의 sentinel `256`을
반환한다. `ACI_PALETTE`는 인덱스 256 자리에 `0`(검정)을 담고 있어서, `layer_color_hex()`가
`aci_to_hex(256)`을 그대로 호출하면 모든 레이어가 검정으로 뭉개진다.

`rgb`의 하위 바이트는 무작위가 아니라 레이어 이름/파일에 걸쳐 결정론적이다 (`"0"` 레이어는
9개 파일 전부에서 항상 `rgb=..07`, AutoCAD 기본 레이어의 실제 ACI 7 관례와 일치; `"...TITL"`은
항상 `5`, `"...BOLD"`는 항상 `6`, `"...FINE"`은 항상 `8`) -- ACI 인덱스 값이 그대로 `rgb`
필드에 들어간 패턴이다. `lib/libredwg` 자체의 `bit_downconvert_CMC`(`bits.c:4069-4071`)에도
정반대 변환 경로에서 동일한 `if (index==256) index = rgb & 0xff;` 폴백이 이미 존재한다 --
이 프로젝트가 쓰는 순수 읽기 경로(`bit_read_CMC`)에만 빠져있던 것.

`crates/uncad/src/tables.rs`의 `resolve_layer_color_index`가 그 폴백을 읽기 경로에
미러링한다. 9개 파일 전부 렌더링해서 AutoCAD 기본/AIA 레이어 색상 관례(지붕=노랑, 문/창=초록,
외곽선=파랑, 배관=하늘색 등)와 부합하는 것으로 눈으로 확인했지만, **실제 AutoCAD로 연 화면과
직접 대조하지는 못했다**(환경에 AutoCAD 없음) -- 이 프로젝트의 다른 색상 버그들과 달리
"AutoCAD 스크린샷 대조"가 아니라 "렌더링 결과의 그럴듯함"으로만 검증됐다는 차이가 있다.
`bit_downconvert_CMC`와 동일한 한계도 그대로 물려받는다: 팔레트에 우연히 안 맞는 진짜 임의의
트루컬러도 작은 ACI 인덱스로 오인할 수 있다.

## (수정됨) SPLINE 컨트롤 포인트가 잘못된 stride로 읽히던 버그

2026-08-05, 사용자 요청으로 `samples/`의 9개 실 파일을 SVG로 변환해서 렌더링 결과를 직접
스크린샷으로 검토하다가 발견: SPLINE 위주 도면 2개(`AutoCADSamples1.dwg`,
`AutoCADSamples5.dwg`)에서 도면 전체를 가로지르는 말도 안 되는 대각선들이 렌더링되고
있었다. 원인은 `crates/uncad/src/dynapi.rs`의 `SplineControlPoint`가 실제 `dwg.h`의
`Dwg_SPLINE_control_point`(`parent` 포인터 + `x,y,z,w` 4개 double, 40바이트)와 달리 앞의
`parent` 포인터 필드 없이 `x,y,z,w`만(32바이트)으로 정의돼 있던 것 -- `get_array_field`가
`ctrl_pts` 배열을 순회할 때 실제 40바이트가 아니라 32바이트 간격으로 원소를 읽어서, 두
번째 컨트롤 포인트부터는 이전 원소의 끝부분 + 다음 원소의 `parent` 포인터 앞부분이
뒤섞인 값을 읽고 있었다(포인터 비트 패턴을 `f64`로 잘못 해석한 값이 정상 좌표 사이에
끼어들며 갈수록 어긋남). fit points(`num_fit_pts`/`fit_pts`)가 있는 SPLINE은 렌더링이
그쪽을 우선하므로 영향이 없었고, fit points 없이 컨트롤 포인트만 있는 SPLINE에서만
증상이 나타나 발견이 늦어졌다.

이 버그는 별개로 보였던 두 증상을 동시에 설명한다 -- (1) 도면 전체를 가로지르는 와일드한
대각선(포인터 비트 패턴이 큰 값으로 해석된 경우), (2) 좌표가 수백 자리 0으로 시작하는
비정상적으로 긴 문자열로 직렬화되던 문제(포인터 비트 패턴이 subnormal 범위로 해석된 경우
-- Rust의 `f64` `Display`는 JS의 `Number.prototype.toString`과 달리 과학적 표기법으로
전환하지 않기 때문). (2)는 `svg.rs`의 `clean()` 헬퍼로 별도 방어 처리도 해뒀지만(포맷팅
레벨의 안전망으로 유지), 근본 원인은 여기 `SplineControlPoint`의 레이아웃이었다.

`get_array_field`의 기존 debug assert(dynapi가 보고하는 필드 크기와 요청 타입 크기를
비교)는 이 버그를 못 잡는다 -- 그 assert는 "배열을 가리키는 포인터 필드" 자체의 크기(8바이트,
항상 일치)만 검사하지, 배열 **원소**의 stride는 전혀 검증하지 않기 때문이다. 재발 방지로
`SplineControlPoint`의 크기를 clang의 실제 `sizeof()`(40바이트)와 대조하는 컴파일타임
어서션을 추가했다(HATCH_Path 등 손으로 쓴 다른 구조체들과 같은 패턴, `libredwg-sys/src/lib.rs`
참고). `SplineControlPoint`처럼 raw 포인터 배열을 stride 기반으로 읽는 다른 구조체를 새로
추가할 때는(예: 향후 확장 시) 이 클래스의 버그를 반드시 염두에 둘 것 -- **C 구조체 필드
목록에 `parent` 백포인터가 있으면 절대 생략하지 말 것.**

## HATCH 패턴 채우기 (2026-08-06)

`Dwg_HATCH_Path/PathSeg/ControlPoint`와 같은 클래스의 bindgen 구조체 코드생성 실패가
`Dwg_HATCH_DefLine`(패턴 정의 라인: `angle`/`pt0`/`offset`/`dashes`)에도 재발했다 -- 이번엔
원인이 조금 다르다: `DefLine`을 직접 allowlist하면 그 자체는 문제없이 생성되지만, `DefLine.parent`
필드가 `struct _dwg_entity_HATCH *`로 선언돼 있어서 bindgen이 `_dwg_entity_HATCH` 자체도
실제(비-opaque) 타입으로 생성해야 하고, `_dwg_entity_HATCH`가 바로 그 실패를 겪는다(이 크레이트
어디도 `_dwg_entity_HATCH`를 직접 allowlist한 적이 없었다 -- HATCH 필드는 전부 dynapi의 `void*`로
읽는다). `Dwg_HATCH_Path/PathSeg/ControlPoint` 때와 동일하게 블록리스트 + 손으로 구조체 재작성으로
해결했고(`parent`를 `*mut c_void`로 남겨서 `_dwg_entity_HATCH`를 끌어들이지 않음), clang의 실제
`sizeof()`(64바이트)와 대조하는 컴파일타임 어서션도 추가했다(`libredwg-sys/src/lib.rs`).

**렌더링 알고리즘**: SVG `<pattern>` 엘리먼트를 defline 하나당 하나씩(`<defs>`에 축적, `to_svg`가
최종적으로 한 번에 방출) 생성해서, HATCH 경계 path를 `fill="url(#...)"`로 채운다 -- 직접 폴리곤
클리핑을 구현하는 대신 SVG 자체의 fill 메커니즘에 위임(경계가 여러 개의 loop/island를 가져도
기존 `fill-rule="evenodd"` path가 그대로 처리). 의도적으로 단순화한 부분 두 가지(`svg.rs`의
`render_hatch_pattern_line` 문서 주석 참고): (1) `offset`의 라인-방향 평행 성분(벽돌쌓기 스타일
스태거링에 쓰임)은 무시하고 수직 성분(간격)만 반영, (2) 타일 안의 선을 `base_point`가 정확히
얹히는 타일 경계가 아니라 타일 중앙에 그려서 `<pattern>`의 기본 타일-경계 클리핑이 선을 반토막
내는 걸 피함(무한 반복 패턴의 시각적 외관상 위치가 반 타일 어긋나는 건 무의미).

**실제로 발견/수정한 버그**: 처음엔 표준 DXF 패턴 채우기 문서(그룹 52 `angle`/41 `scale_spacing`이
그룹 78 서브레코드의 definition-line 데이터 위에 추가로 적용된다는 서술)를 따라
`HatchEntity.pattern_angle`/`pattern_scale`을 각 `HatchPatternLine`에 다시 곱해서 적용했는데,
`samples/AutoCADSamples7.dwg`로 렌더링해보니 패턴이 전혀 안 보였다(작은 문/가구 심볼 안의 헤치가
텅 비어 보임). 원인 조사 결과: `pattern_angle`이 정확히 90도인 HATCH의 defline 자신의 `angle`도
독립적으로 정확히 90도였다 -- LibreDWG가 파싱하는 defline 데이터에는 이미 52/41이 반영돼 있다는
뜻이다(0도짜리 원본 패턴에 90도를 또 더했다면 180도가 됐어야 함). 수치로도 확인: `pattern_scale`
60을 defline의 ~6.5유닛 간격에 다시 곱하면 ~390유닛이 되는데, 그 헤치가 채워야 할 도형 자체가
~90유닛밖에 안 돼서 타일 하나 안에 선이 한 개도 안 들어가는 상황이었다(그래서 렌더링 결과가
비어 보였음). `pattern_angle`/`pattern_scale` 재적용을 완전히 제거하고 defline의 `angle`/
`base_point`/`offset`/`dash_pattern`을 이미 최종(절대) 값으로 그대로 쓰도록 고치니 동일 파일에서
실제로 조밀한 크로스해치/타일 패턴이 정상적으로 나타났다(스크린샷으로 확인). 이 발견에 따라
`HatchEntity`에서 `pattern_angle`/`pattern_scale` 필드 자체를 제거했다(안 쓰는 필드를 남겨두지
않음). **표준 DXF 스펙 문서를 그대로 믿지 말고, 실제 LibreDWG가 무엇을 파싱해서 주는지 실 파일로
검증할 것** -- 이 프로젝트 전반의 "실 파일 대조 없이는 검증된 게 아니다" 원칙이 새로 구현하는
기능에도 동일하게 적용된 사례.

**검증**: `cargo test --workspace` 35개 테스트(순수 알고리즘 유닛테스트 대거 추가 -- `acis.rs`
SAT 파서 6개, `svg.rs` 클러스터링/HATCH 엣지 근사/stroke-width 치환/변환 합성 등 23개) 전부
통과. `samples/`의 9개 실 파일 전부 크래시 없이 렌더링 확인, `AutoCADSamples7.dwg`(HATCH 432개)
와 `AutoCADSamples5.dwg`(입면도 렌더링 -- 나무/지붕/그림자 텍스처가 전부 HATCH 기반)를 헤드리스
브라우저 스크린샷으로 시각 확인.

**아직 검증 안 된 부분**: 위에서 이미 언급한 두 가지 의도적 단순화(브릭 스태거링 미반영, 타일
위상 근사) 외에, `pattern_type`(0=user-defined/1=predefined/2=custom)에 따라 defline 데이터의
의미가 실제로 달라지는지는 확인하지 못했다 -- 스팟 체크한 파일들에서는 구분 없이 동일한 해석이
잘 맞았지만, 세 종류를 명시적으로 다르게 처리해야 하는 파일을 만나면 재검토가 필요할 수 있다.
**실측**: `samples/`의 9개 파일에 있는 ~2500개 HATCH 전부 `pattern_type=1`(predefined)이었다
(0/2는 한 번도 등장하지 않음, 2026-08-07 재확인) -- 세 값을 구분해야 하는 실제 사례가 아직
한 번도 없었다는 뜻이라, 이 캐비어트는 "틀렸을 수도 있다"보다는 "테스트할 기회가 아직 없었다"에
가깝다.

## HATCH 그라디언트 채우기 (2026-08-07, 실 파일로 검증 안 됨)

`is_gradient_fill` 자체가 위 재확인에서 9개 파일의 ~2500개 HATCH 전부 `0`으로 나왔다 --
gradient-fill HATCH를 포함한 파일이 이 프로젝트의 스팟 체크 셋에 하나도 없다는 뜻이다. 그래서
아래 구현은 `dwg.h`의 필드 주석과 일반적인 DXF 그라디언트 지식에 기반한 것이지, 이 프로젝트
다른 대부분의 로직처럼 실제 AutoCAD 렌더링과 대조해서 확정된 게 아니다 -- MULTILEADER/MLINE 등
JS baseline에 없던 다른 "새 기능"들과 같은 리스크 등급.

**bindgen**: `Dwg_HATCH_Color`(그라디언트 색상 스탑 하나 -- `shift_value` + `Dwg_Color`)가
`Dwg_HATCH_DefLine`과 완전히 같은 `parent: struct _dwg_entity_HATCH *` 캐스케이드를 겪어서
같은 방식(블록리스트 + 손으로 구조체 재작성, clang 실제 `sizeof()` 64바이트와 대조하는
컴파일타임 어서션)으로 해결했다.

**렌더링**: `gradient_name`(`SPHERICAL`/`HEMISPHERICAL`/`CURVED`/`LINEAR`/`CYLINDER`)을
이진 선택으로 단순화했다 -- `SPHERICAL`/`HEMISPHERICAL`은 SVG `radialGradient`로(중심에서
바깥으로 퍼지는 AutoCAD의 실제 느낌과 유사), 나머지(`CURVED`/`CYLINDER` 포함, 인식 못하는
이름도)는 `linearGradient`로 근사한다 -- `CURVED`/`CYLINDER`도 AutoCAD에서 방향성은 있지만
방사형은 아니라서, 곡률은 놓치더라도 선형 근사가 방사형 근사보다 낫다고 판단했다. 색상 두
스탑은 `single_color_gradient`가 꺼져 있으면 `colors[]`를 `shift_value`로 정렬해서 양 끝을
쓰고, 켜져 있으면(색상 하나만 저장됨) `gradient_tint`로 흰색 쪽으로 블렌드한 값을 두 번째
스탑으로 만든다(`color::tint_toward_white`, 단순 RGB 채널별 선형 블렌드 -- AutoCAD가 실제로
같은 공식을 쓰는지는 검증 못함, "그럴듯한 근사"임을 명시). `gradient_shift`(DXF 461, "Centered"
옵션)는 `HatchPatternLine.offset`의 평행 성분 무시와 같은 종류의 의도적 단순화로 적용하지
않았다.

**검증**: `cargo test --workspace` 45개(그라디언트 색상 해석/정렬/틴트 관련 신규 유닛테스트
포함) 전부 통과, `cargo clippy --workspace --all-targets` 클린(새 경고 없음), 9개 fixture
전부 크래시 없이 렌더링되고 `is_gradient_fill=0`이 실측대로 gradient def를 하나도 생성하지
않는 것까지 확인(정상 -- 그라디언트가 없는 파일이니 없는 게 맞음). **렌더링 결과 자체가 실제
그라디언트 채우기와 시각적으로 일치하는지는 그라디언트 포함 파일을 구하기 전까지는 확인할
방법이 없다.**

## MLINESTYLE 파싱 -- MLINE이 이제 진짜 오프셋 라인을 그림 (2026-08-07, 실 파일로 검증 안 됨)

MLINE은 이전엔 MLINESTYLE을 파싱하지 않아서(위 "MULTILEADER/MLINE/..." 문단 참고) 중심선
하나만 그리는 정직하지만 불완전한 근사였다. 이제 MLINE의 `mlinestyle` 핸들을 해석해서
`Tables::mlinestyles`(MLINESTYLE 이름 -> 각 평행선의 `offset` 목록)에서 찾고, 각 정점의
`point + miter_direction * offset`으로 실제 오프셋 폴리라인을 그린다 -- `miter_direction`은
LibreDWG가 이미 미터 각도까지 반영해서 계산해 둔 벡터라, 삼각함수 없이 스칼라 곱셈 하나로
끝난다. 스타일을 못 찾으면(핸들이 비어있거나 해석 실패) `offset=0.0` 하나만 있는 걸로 취급해서
예전과 정확히 같은 중심선 렌더링으로 자연스럽게 폴백한다(별도 분기 없이 같은 함수
`mline_offset_points`를 재사용).

**bindgen**: `Dwg_MLINESTYLE_line`(평행선 정의 -- `offset`/`color`/`lt_index`/`lt_ltype`)이
HATCH의 두 손수-정의 타입과 완전히 같은 `parent: struct _dwg_object_MLINESTYLE *` 캐스케이드를
겪어서 같은 방식(블록리스트 + 손으로 구조체 재작성, clang 실제 `sizeof()` 80바이트와 대조하는
컴파일타임 어서션)으로 해결했다.

**실측**: 9개 fixture 전부 여전히 MLINE을 포함하지 않는다(재확인) -- MULTILEADER와 같은
처지로, 이 기능도 실제 AutoCAD 파일/렌더링과 대조된 적이 없다. `offset`/`miter_direction`을
곱하는 계산 자체는 `svg.rs`의 `mline_offset_points_*` 유닛테스트 2개로 검증했지만(0 오프셋이
중심선과 같음, 0이 아닌 오프셋이 `miter_direction` 방향으로 정확히 변위됨), 이건 산수가 맞는지
확인한 것이지 이 계산식 자체(LibreDWG의 `miter_direction`을 이런 식으로 쓰는 게 AutoCAD의 실제
MLINE 지오메트리와 일치하는지)가 맞는지 확인한 게 아니다 -- `dwg.h`의 필드 이름/의미론에 대한
합리적 해석이지, 실제 렌더링으로 확정된 사실은 아니다.

**검증**: `cargo test --workspace` 47개 전부 통과, `cargo clippy --workspace --all-targets`
클린, 9개 fixture 전부 크래시 없이 렌더링되고 출력이 이전(MLINESTYLE 파싱 전)과 바이트 단위로
동일함을 확인(정상 -- MLINE이 없는 파일들이니 동일해야 함).

## DWG/DXF 쓰기 지원 (2026-08-07)

`CadDatabase`가 이제 `write_dwg(path)`/`write_dxf(path)` 메서드로 자기 자신을 다시
DWG/DXF로 저장할 수 있다 -- `parse()`가 채운 뒤 즉시 해제하던 LibreDWG의 실제 `Dwg_Data`를
이제 `CadDatabase` 안에 살려서 들고 있다가, 그걸 그대로 LibreDWG 자신의 인코더(`dwg_write_file`)
/새 shim(`uncad_write_dxf`)에 넘기는 방식이다. `entities`/`tables`(렌더링 전용, 손실 있는
Rust 투영)에서 역변환하는 게 아니다 -- `docs/ARCHITECTURE.md`의 "두 계층 모델" 섹션 참고.
`dwg_to_dxf(path, path)` 자유 함수는 그대로 남아있다 -- `CadDatabase`를 만들 필요 없는 순수
파일→파일 변환 용도.

**DWG 쓰기는 R_2004 이하에서만 안정적** -- LibreDWG 자신의 `README`: "Write support only
works for earlier versions until r2004. Rewriting most DWG's <= r2004 usually works fine."
이 프로젝트가 손댈 수 있는 부분이 아니라 업스트림 인코더 자체의 경계다.

**실측 (9개 fixture 전부 AC1018/R_2004, 그래서 실제로 검증 가능했던 몇 안 되는 케이스)**:
`write_dwg`가 9개 중 **5개는 성공**(AutoCADSamples1/2/3/4/8, 재파싱한 엔티티 타입별 개수가
원본과 정확히 일치 확인), **4개는 `DWG_ERR_INVALIDDWG`(코드 2048)로 실패**
(AutoCADSamples5/6/7/9). 실패 원인이 HATCH 개수 같은 명백한 패턴은 아니다 -- 확인해보니
AutoCADSamples9는 HATCH가 아예 0개인데도 실패하고, 실패한 파일들과 성공한 파일들의 HATCH
개수 범위가 겹친다(성공한 AutoCADSamples2는 188개, 실패한 AutoCADSamples6은 95개). 네
파일 다 `dwg_read_file`로는 문제없이 읽혔고(엔티티 변환/SVG 렌더링 전부 정상) `dwg_write_file`의
`dwg_encode` 단계에서만 실패한다 -- 즉 우리 쪽 `Dwg_Data` 보관/전달 방식의 문제가 아니라,
LibreDWG 자신의 인코더가 이 파일들에 있는 (아직 특정 못 한) 무언가를 다시 못 쓴다는 뜻이다.
"usually works fine"이라는 업스트림 표현 그대로 -- 항상은 아니다. 실패해도 크래시하지 않고
`WriteError::Critical(2048)`을 정상적으로 반환한다. **DXF 쓰기는 이 4개 포함 9개 전부
성공** -- `write_dxf`가 실패하는 게 아니라 `dwg_encode`(DWG 전용 경로)만의 문제.

**`dwg_write_file`은 기존 파일을 덮어쓰지 않는다** -- 대상 경로에 이미 파일이 있으면
`stat()`으로 확인한 뒤 덮어쓰지 않고 `DWG_ERR_IOERROR`를 반환한다(`WriteError::Critical`로
나타남). 이 크레이트는 이 동작을 그대로 노출한다 -- 자동으로 지우지 않음(업스트림의 안전장치
존중). DXF 쓰기(`write_dxf`/`dwg_to_dxf`)는 이 제약이 없다 -- 그냥 `fopen(path, "wb")`라서
기존 파일을 덮어쓴다.

**`write_dwg`가 `&mut self`를 받는 이유**: `dwg_write_file`의 공개 시그니처는
`const Dwg_Data*`지만, LibreDWG의 `dwg.c`를 직접 읽어보면 내부에서
`dwg_encode((Dwg_Data*)dwg, &dat)`로 const를 캐스팅해서 버리고 실제로 내부 상태를 변경한다.
공개 시그니처의 약속을 그대로 믿는 건 안전하지 않다고 판단해서 `&self`가 아니라 `&mut self`로
받는다.

**교차검증**: 같은 소스 파일에 대해 `db.write_dxf(path)` 결과와 기존
`dwg_to_dxf(원본경로, path)` 결과가 9개 fixture 전부 바이트 단위로 완전히 동일함을 확인
(각각 2.1MB~78MB 범위) -- 새 shim(`uncad_write_dxf`, 이미 열려있는 `Dwg_Data`를 씀)과 기존
shim(`uncad_write_dxf_file`, 파일을 새로 읽음)이 서로 다른 경로로 같은 결과에 도달한다는
뜻이라 유용한 회귀 가드다. `dxf_read_file`로 채운 `Dwg_Data`도 `write_dwg`가 정상 동작함을
확인(DWG -> DXF -> DWG 왕복, 엔티티 개수 일치) -- DXF로 읽은 도면도 다시 DWG로 저장 가능.

## 스레드 세이프티

`docs/ARCHITECTURE.md` 참고 -- `uncad` 크레이트는 안전하지만 `libredwg-sys`를 직접 쓰면 직접
직렬화해야 한다.

## 파일 기반 회귀 테스트는 하나뿐 (폭넓은 실 파일 커버리지는 여전히 없음)

`cargo test --workspace`는 58개 테스트를 돈다(2026-08-14 기준: `color.rs` 11개 -- ACI/BYLAYER
색상 해석 + 그라디언트용 `tint_toward_white`, `acis.rs` 6개 -- SAT 레코드 파싱/포인터 해석/
와이어프레임 추출, `convert.rs` 6개 -- HATCH 그라디언트 색상 해석/스탑 정렬/`gradient_name`
분류, `svg.rs` 28개 -- outlier-trim 클러스터링, HATCH 엣지 근사, MTEXT 포맷팅 스트리핑,
stroke-width 치환, transform 합성, HATCH 패턴 채우기, MLINE 오프셋 계산, TEXT/ATTRIB 회전
transform, 비유한(non-finite) 좌표 방어, 블록 참조 재귀의 콤비네이토리얼 폭증 방지 등,
`tables.rs` 3개 -- LAYER TRUECOLOR-256-sentinel 폴백). 대부분은 fixture 파일 없이 합성
데이터로 검증 가능한 순수 함수 유닛테스트라 실제로 유용한 회귀 가드다 -- 예를 들어
`dominant_cluster_box`가 합성 박스 집합에서 지배적 클러스터를 정확히 골라내는지,
`parse_sat_records`가 `End-of-ACIS-data` 마커 이후를 제대로 자르는지 등은 실 DWG 파일 없이도
확정적으로 검증 가능하고 실제로 그렇게 하고 있다.

**예외 하나**: `png.rs`의 `to_png_renders_a_real_dwg_to_a_valid_png`는 실제 DWG 파일
(`lib/libredwg/test/test-data/2000/circle.dwg`, `libredwg-sys`가 이미 빌드에 요구하는
git submodule 소속이라 `samples/`와 달리 항상 커밋된 채로 존재) 하나로 `parse()` ->
`to_svg()` -> `to_png()` 전체 파이프라인을 실행해 유효한 PNG가 나오는지 확인하는 진짜
end-to-end 테스트다. 다만 이건 "크래시 없이 유효한 출력이 나온다"를 파일 하나로 확인하는
스모크 테스트 수준이지, 엔티티 타입별 렌더링 정확성을 폭넓게 검증하는 게 아니다.

**여전히 없는 것**: 실제 DWG/DXF 파일 기반 `parse()`/`to_svg()`/`dwg_to_dxf()`의 폭넓은
end-to-end 검증(여러 파일에 걸친 엔티티 카운트 parity, 렌더링 바이트 단위 비교 등)은
자동화되어 있지 않다 -- `samples/`가 완전히 gitignore 처리되어 있어서(라이선스 확인 없이
아무 파일이나 넣고 쓰라는 의도적 선택), 커밋된 채로 CI가 참조할 수 있는 파일이 없다.
`samples/README.md` 참고. 이 커버리지를 되살리려면 라이선스가 명확한 파일을 다시 커밋하고,
이 프로젝트 코드 자신의 출력이 아닌 독립적인 참조로 기대값을 검증해야 한다.

**`cargo clippy` 현재 상태**: 2026-08-14부터 `.github/workflows/ci.yml`의 별도 `lint` 잡이
모든 push/PR에서 `cargo fmt --check`와 `cargo clippy --workspace --all-targets -- -D
warnings`를 돌려 클린 여부를 자동으로 추적한다(그 전까지는 `cargo build`/`cargo test`만
실행되고 있었다). `libredwg-sys`의 bindgen 생성 `bindings.rs`(이 프로젝트가 손으로 쓴 코드가
아니라 빌드마다 새로 생성됨)가 내는 무해한 경고 두 종류(`useless_transmute`,
`missing_safety_doc`)는 `-D warnings`가 생성 코드까지 실패시키지 않도록
`crates/libredwg-sys/src/lib.rs`에서 크레이트 레벨로 `#![allow(...)]` 처리했다 -- 이 프로젝트가
직접 손으로 쓴 코드에서 나는 새 경고는 여전히 CI를 실패시킨다. 아래 여러 "검증" 항목에 남아있는
"clippy 클린" 서술은 그 기능을 구현했던 시점의 기록이다.

## Windows/Linux 크로스플랫폼

Linux(`x86_64-unknown-linux-gnu`)에서도 빌드/테스트 전부 통과 확인됨(로컬 Docker `rust:latest`
+ `libclang-dev`로 검증, 그리고 이제 `.github/workflows/ci.yml`의 `ubuntu-latest` 잡으로
모든 push/PR에서 자동 재검증됨). 2026-08-14부터 `ci.yml`에 `windows-latest` 잡도 추가되어
이 프로젝트가 실제로 개발되는 플랫폼(MSVC)의 빌드도 CI에서 검증되기 시작했다 -- 그 전까지는
로컬 수동 빌드로만 확인되고 있었다. 아래 3건은 그 Linux 이식성 검증 과정에서 발견/수정한
실제 버그다:

- `config.h`가 `SIZEOF_WCHAR_T`를 Windows 값(2)으로 하드코딩해서 Linux에서 `BITCODE_TU`가
  잘못된 포인터 타입으로 해석되던 것.
- `dwg_object_get_fixedtype`/`dwg_read_file` 등 몇몇 LibreDWG 함수의 실제 C 반환 타입(`int`)과
  관련 enum의 bindgen 추론 타입이 플랫폼에 따라 다르게 갈리던 것(`docs/ARCHITECTURE.md`의
  "크로스플랫폼 enum 폭 문제" 참고).
- `Dwg_MLINE_vertex`를 블록리스트해도 Linux 타겟에서는 bindgen이 `Dwg_MLINE_line`을 추가로
  생성해서 블록리스트된 타입을 참조하는 컴파일 에러를 내던 것 -- Windows/MSVC에서는 나타나지
  않았음(위 "MULTILEADER/MLINE/REGION/POLYLINE_PFACE" 문단 참고).

**32비트 타겟은 아예 지원 대상이 아니고, 이제 빌드 시점에 그렇게 실패한다**: `config.h`가
`SIZEOF_SIZE_T`를 8(64비트)로 고정해두었는데, 이웃 필드 `SIZEOF_WCHAR_T`와 달리 플랫폼 분기가
없다. 이 값은 LibreDWG의 `MAX_MEM_ALLOC` 할당 크기 안전장치(`bits.h`)와 `bits.c`의 여러
워드-정렬 고속 경로 읽기에 쓰이는데, 32비트 타겟(`i686-*`, `wasm32-*`, `arm-*` 등)에서는 이
가정이 깨져서 정렬되지 않은 읽기와, 공격자가 통제하는 DWG 크기 필드에 대해 지나치게 관대한
할당 크기 게이트로 이어질 수 있다 -- 컴파일은 되지만 조용히 잘못된 동작을 하는, 진단하기 가장
어려운 종류의 버그다. 지금까지는 이 프로젝트가 x86_64 전용으로만 빌드/검증되어 왔을 뿐 이를
강제하는 장치가 없었다. `crates/libredwg-sys/build.rs`가 이제 빌드 시작 시
`CARGO_CFG_TARGET_POINTER_WIDTH`를 확인해서 64비트가 아니면 명확한 메시지와 함께 즉시
`panic!`한다 -- 32비트 타겟에서 잘못된 값으로 조용히 컴파일되는 것보다 빌드 실패가 낫다는
판단. 32비트를 실제로 지원하려면 `config.h`에 `SIZEOF_WCHAR_T`처럼 플랫폼 분기를 추가하고
그 타겟에서 실제로 빌드/테스트해서 검증해야 한다 -- 아직 안 한 일이다.

셋 다 업스트림 C 헤더/bindgen 자체에 이미 있던 잠재적 이식성 문제였고, MSVC에서는 우연히
드러나지 않았을 뿐이다. Docker로 로컬 Linux 빌드를 매번 확인하는 습관이 이 3건 전부를
커밋 전에 잡아낸 이유이기도 하다.
