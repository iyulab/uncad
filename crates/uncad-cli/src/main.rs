//! Port of `src/cli.mjs`, DWF/DWFx branch dropped (scope decision: this
//! Rust port is DWG/DXF only). Same argv surface, same summary/`-o`
//! dispatch, same Korean-language usage/messages.

use std::collections::HashMap;
use std::path::Path;
use std::process::ExitCode;
use uncad::{RenderEntity, Space, ToPngOptions, ToSvgOptions};

struct Args {
    input: Option<String>,
    output: Option<String>,
    space: String,
    outlier_trim: bool,
    scale: String,
    help: bool,
}

fn usage() {
    eprintln!(
        r#"uncad - DWG/DXF 파일 파싱 CLI

사용법:
  uncad <input.dwg>                 요약 정보 출력 (엔티티 타입 개수)
  uncad <input> -o <output.svg>     이미지(SVG)로 추출
  uncad <input> -o <output.png>     이미지(PNG)로 추출 (SVG를 거쳐 래스터화)
  uncad <input> -o <output.dxf>     DXF로 저장
  uncad <input> -o <output.dwg>     DWG로 저장 (R_2004 이하만 안정적 -- docs/CAVEATS.md 참고)

옵션 (SVG/PNG 추출 전용):
  --space <model|paper|all>   렌더링할 스페이스 (기본: model)
                               model = 실제 도면, paper = 도곽/타이틀블록,
                               all = 예전 동작(전부 하나로 합쳐서 렌더링)
  --no-trim                   이상치 좌표를 뷰박스 계산에서 자동 제외하지 않음

옵션 (PNG 추출 전용):
  --scale <factor>             SVG의 viewBox 기준 배율 (기본: 1.0, 예: 2.0 = 2배 해상도)

예:
  uncad drawing.dwg
  uncad drawing.dwg -o drawing.svg
  uncad drawing.dwg -o drawing.svg --space paper
  uncad drawing.dwg -o drawing.png --scale 2
  uncad drawing.dwg -o drawing.dxf
  uncad drawing.dxf -o drawing.dwg"#
    );
}

fn parse_args(argv: &[String]) -> Args {
    let mut args = Args {
        input: None,
        output: None,
        space: "model".to_string(),
        outlier_trim: true,
        scale: "1".to_string(),
        help: false,
    };
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "-o" | "--output" => {
                i += 1;
                args.output = argv.get(i).cloned();
            }
            "--space" => {
                i += 1;
                if let Some(v) = argv.get(i) {
                    args.space = v.clone();
                }
            }
            "--no-trim" => args.outlier_trim = false,
            "--scale" => {
                i += 1;
                if let Some(v) = argv.get(i) {
                    args.scale = v.clone();
                }
            }
            "-h" | "--help" => args.help = true,
            other if args.input.is_none() => args.input = Some(other.to_string()),
            _ => {}
        }
        i += 1;
    }
    args
}

fn summarize(counts: &HashMap<&str, usize>) -> String {
    let mut entries: Vec<_> = counts.iter().collect();
    entries.sort_by(|a, b| b.1.cmp(a.1));
    entries
        .into_iter()
        .map(|(t, c)| format!("  {t}: {c}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn count_by_type(entities: &[RenderEntity]) -> HashMap<&str, usize> {
    let mut counts = HashMap::new();
    for e in entities {
        *counts.entry(e.type_name()).or_insert(0) += 1;
    }
    counts
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = parse_args(&argv);

    if args.help || args.input.is_none() {
        usage();
        return if args.help {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    let input = args.input.as_ref().unwrap();

    // `uncad::parse()` hands back a bare LibreDWG error code (e.g. "code
    // 4096") for a nonexistent path or a wrong-extension file -- accurate,
    // but not something a user can act on without cross-referencing
    // dwg.h. Check the obvious cases ourselves first for a message that
    // says what's actually wrong.
    match std::fs::metadata(input) {
        Ok(meta) if meta.is_dir() => {
            eprintln!("오류: 입력 경로가 파일이 아니라 디렉터리입니다: '{input}'");
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!("오류: 입력 파일을 열 수 없습니다: '{input}' ({e})");
            return ExitCode::FAILURE;
        }
        Ok(_) => {}
    }

    let mut db = match uncad::parse(input) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("오류: '{input}' 파싱 실패 ({e}) -- 유효한 DWG/DXF 파일인지 확인하세요");
            return ExitCode::FAILURE;
        }
    };

    let Some(output) = &args.output else {
        println!("파일: {input}");
        println!("엔티티 ({}개):", db.entities.len());
        println!("{}", summarize(&count_by_type(&db.entities)));
        return ExitCode::SUCCESS;
    };

    let out_ext = Path::new(output)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let mut unsupported: Vec<String> = Vec::new();
    match out_ext.as_str() {
        "svg" => {
            let space = match parse_space(&args.space) {
                Ok(space) => space,
                Err(e) => {
                    eprintln!("오류: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let result = db.to_svg(ToSvgOptions {
                space,
                outlier_trim: args.outlier_trim,
                ..Default::default()
            });
            if let Err(e) = std::fs::write(output, &result.svg) {
                eprintln!("오류: {e}");
                return ExitCode::FAILURE;
            }
            unsupported = result.unsupported_entity_types;
        }
        "png" => {
            let space = match parse_space(&args.space) {
                Ok(space) => space,
                Err(e) => {
                    eprintln!("오류: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let scale = match args.scale.parse::<f32>() {
                Ok(v) if v > 0.0 && v.is_finite() => v,
                _ => {
                    eprintln!(
                        "오류: --scale 값은 0보다 큰 유한한 숫자여야 합니다 (받은 값: '{}')",
                        args.scale
                    );
                    return ExitCode::FAILURE;
                }
            };
            let result = match db.to_png(ToPngOptions {
                svg: ToSvgOptions {
                    space,
                    outlier_trim: args.outlier_trim,
                    ..Default::default()
                },
                scale,
            }) {
                Ok(result) => result,
                Err(e) => {
                    eprintln!("오류: {e}");
                    return ExitCode::FAILURE;
                }
            };
            if let Err(e) = std::fs::write(output, &result.png) {
                eprintln!("오류: {e}");
                return ExitCode::FAILURE;
            }
            unsupported = result.unsupported_entity_types;
        }
        "dxf" => {
            if let Err(e) = uncad::dwg_to_dxf(input, output) {
                eprintln!("오류: {e}");
                return ExitCode::FAILURE;
            }
        }
        "dwg" => {
            if let Err(e) = db.write_dwg(output) {
                eprintln!("오류: {e}");
                return ExitCode::FAILURE;
            }
        }
        other => {
            eprintln!("오류: 지원하지 않는 출력 확장자 '.{other}' (.svg, .png, .dxf, .dwg만 지원)");
            return ExitCode::FAILURE;
        }
    }

    println!("저장됨: {output}");
    if !unsupported.is_empty() {
        eprintln!(
            "경고: 지원하지 않는 엔티티 타입이라 이미지에서 빠짐: {}",
            unsupported.join(", ")
        );
    }
    ExitCode::SUCCESS
}

fn parse_space(value: &str) -> Result<Space, String> {
    match value {
        "model" => Ok(Space::Model),
        "paper" => Ok(Space::Paper),
        "all" => Ok(Space::All),
        other => Err(format!(
            "지원하지 않는 --space 값 '{other}' (model, paper, all 중 하나)"
        )),
    }
}
