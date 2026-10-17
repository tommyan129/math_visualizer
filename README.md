# math_visualizer

대학 수학 공부용 수식 시각화 도구. Claude가 MCP로 DSL 요청을 보내면 Rust 서버가 해석·검증해서 상태 모델로 확정하고, 브라우저가 계산·샘플링·렌더링합니다.

수식 입력법을 따로 익히지 않아도 Claude가 대신 그려주고, 강조·scene·step·note로 시각 자료를 조작해 이해를 돕는 것이 목표입니다.

## 구조

```
Claude ──MCP(HTTP)──▶ server (Rust) ──WebSocket──▶ browser (TypeScript)
                      해석·검증·이력              계산·샘플링·렌더링
```

- **서버는 수식 값을 계산하지 않습니다.** 파싱, 이름 해석, 타입 검사, 구조 기반 비용 판단, 기호 미분(AST 변환)만 하고 확정된 AST를 브라우저로 보냅니다.
- **호스트 eval 없음.** 식은 자체 parser를 거치고, 브라우저 codegen은 검증된 AST 노드 종류로만 JS를 조립합니다. 내장 함수는 화이트리스트이며 인터프리터는 fuel·시간·깊이·할당 상한을 가집니다.
- **요청 기록은 추가만 됩니다.** 모든 `apply`는 원자적으로 적용되고 SQLite에 남아 되돌리기·복원에 쓰입니다.

| 경로 | 내용 |
|---|---|
| `crates/dsl` | lexer, parser, AST, 이름 해석, 타입 검사(벡터·행렬), 인터프리터, 기호 미분 |
| `crates/model` | 확정된 상태 모델, WebSocket 메시지 (ts-rs로 TS 타입 생성) |
| `crates/server` | axum HTTP/WebSocket, rmcp MCP 엔드포인트, SQLite 요청 로그 |
| `web` | TypeScript + Vite (UI 프레임워크 없음), Web Worker 계산, Canvas 2D 렌더러, MathJax |

## 기능

- 함수 정의 `define`, 구간별 함수 `split`, 매개변수·극좌표 곡선, 음함수 방정식, `family` 반복
- 도함수 `f@n`, 역함수 `~f`, 정의하지 않은 도함수는 서버가 기호 미분으로 채움
- 특징점: 극값, 근, 교점, 변곡점, 최대·최소, 불연속점(구멍·점근선·점프 등)
- `calc` 계산 과정: 식 변형 단계, 바뀐 부분 색칠, 수치 검증
- 벡터·행렬, `span`, `arrow`
- 강조(넓이, 부등식 영역, 식의 일부 색칠), 슬라이더 상수, scene·step·note, animate

예:

```
define:
    f(x) -> y: x^3 - 3*x
    E: extrema(f)
    R: roots(f; -1 < x < 2)
```

문법 전체는 MCP의 `describe` 도구로 조회할 수 있습니다.

## 실행

필요: Rust (rustup), Node.js

```sh
cd web && npm install && npm run build && cd ..
cargo run -p server
```

- MCP: `http://127.0.0.1:7800/mcp`
- 브라우저: `http://127.0.0.1:7801/` (`web/dist` 제공)
- DB: `data/mv.db`

환경 변수: `MV_PUBLIC_ADDR`, `MV_PRIVATE_ADDR`, `MV_DB`, `MV_WEB_DIST`

Claude Code에 연결:

```sh
claude mcp add --transport http math-visualizer http://127.0.0.1:7800/mcp
```

프론트엔드 개발 서버: `cd web && npm run dev` (`/ws`, `/api`를 `127.0.0.1:7801`로 프록시)

## 테스트

```sh
cargo test --workspace     # TS 바인딩(web/src/generated)과 web 테스트 fixture도 다시 생성
cd web && npm run typecheck && npm test
```
