//! `describe`: command list without a name, syntax and examples with one.
//! Tool descriptions stay short; details live here (design.md "MCP tool").

use dsl::names;

const INDEX: &str = "\
쓸 수 있는 명령 (4단계). describe(name)으로 자세히:
  define     함수 정의·수정 (같은 이름에 다시 쓰면 수정)
  행렬       벡터·행렬 값, matmul 등 연산, -> matrix 함수
  span       span(v, w): 원점을 지나는 직선·평면
  arrow      arrow(v), arrow(v; P): 화살표
  del        정의·강조·style·변수·scene 삭제 (참조하는 것도 함께 지워짐)
  scene      보기 프리셋: 좌표계, 처음 켜질 것, clip, view
  step       scene 안의 순서: 켜고 끄기, 상수 값, animate, 카메라, note
  note       step 설명 문구 ({식}, $LaTeX$)
  constant   슬라이더 조건 선언 (범위, 기본값, 간격, 정수)
  highlight  강조: 함수 아래 넓이, 부등식 영역, 식의 일부 색칠
  주소       식의 일부 가리키기 f.(b*x).coef
  style      강조 모양 (기본 style, 사용자 style)
  변수       이름 = 값 (부등식, 문자열)
  req        사용자 요청사항 (get_requests로 확인)
  식       연산자, 우선순위, 괄호
  이름     이름 규칙, 그리스 문자, 첨자
  함수     기본 함수 목록
  sum      sum / prod
  상수     슬라이더 상수 규칙
  오류     오류 형식
아직 없음: split, 매개변수·극좌표·음함수, 다변수, 특징점(roots 등)";

const MATRIX: &str = "\
define:
    A: [[1, 2],
        [3, 4]]                 # 행렬 (행마다 [..], 여러 줄 가능, 행 길이 같아야 함)
    v(; a): [1, a]              # 열벡터. 상수를 쓰면 함수처럼 ';' 뒤에 적음
    R(\\theta) -> matrix: [[cos(\\theta), -sin(\\theta)], [sin(\\theta), cos(\\theta)]]
    f(x; a) -> y: matmul(R(x), v)_[2]
- [1, 2, 3]은 열벡터, [[1, 2, 3]]은 1×3 행렬(행벡터). 원소는 A_[1, 2], v_[2] (1부터).
- 연산: A + B, A - B, 2*A, A/2는 연산자. 곱은 matmul(A, B), 거듭제곱 pow(A, n) (n이 음수면 역행렬의 거듭제곱),
  inv(A), transpose(A), det(A), tr(A). A^2, A*B는 오류 (pow, matmul로).
- 크기는 서버가 값 계산 없이 검사: 크기가 안 맞는 matmul·덧셈, 정사각이 아닌 inv/det/pow는 오류.
- 벡터·행렬과 -> matrix 함수는 그리지 않음. 다른 정의 안에서 이름(v)이나 호출(R(t))로 씀.
- 참조하면 그 상수도 적어야 함 (v가 a를 쓰면 S(; a): span(v, w)). 벡터 원소는 숫자만 ([v, w]는 오류).";

const SPAN: &str = "\
define:
    S(; a): span(v, w)       # 벡터 1개 이상, 길이 같음
- 원점을 지나는 직선·평면·점. 어느 것인지는 브라우저가 현재 값으로 정함 (슬라이더로 바뀜):
  평행하지 않은 두 벡터는 2D에서 평면 전체, 평행하면 직선, 영벡터만이면 원점.
- 함수처럼 토글·삭제·req 대상. 벡터를 지우면 span도 지워짐.
- 길이 2 벡터는 2D scene(어느 축 이름이든)에, 길이 3은 3D에만 (6단계). 기본 scene은 2D(x, y).
- 보이는 범위만 그림, scene clip이 있으면 그 안만. 그린 차원은 apply 응답과 get_state에 나옴.";

const ARROW: &str = "\
define:
    p(; a): arrow(v)                    # 원점에서 v까지
    q(; c): arrow([1, 2*c]; [c, c^2])   # 점 P에서 출발: 시작점은 ';' 뒤, 위치벡터로 (y = x^2의 접선 방향)
- 시작점과 벡터는 같은 길이. 2D scene에는 길이 2만.
- 함수처럼 토글·삭제·req 대상. 기본 scene에서 켜진 채 시작.";

const DEFINE: &str = "\
define: 블록 안에 한 줄에 정의 하나 (스페이스 4칸 들여쓰기).
  define:
      f(x; a, b) -> y: a*x^2 + b*x + 1
      g(x; a, b) -> y: f(x) + 1
- 괄호 안 ';' 앞은 입력 변수, 뒤는 상수(슬라이더). '->' 뒤는 출력 축.
- 축 이름이 좌표계: g(t) -> y는 2D(t, y)에 그려짐 (2D(x, y)와 별개, 기본 scene도 따로).
- 식에 쓰는 상수는 전부 ';' 뒤에 적음. 부르는 함수가 쓰는 상수도 적어야 함 (g가 f를 부르면 a, b).
- 같은 이름을 다시 정의하면 수정. f에 상수가 추가·제거되면 부르는 함수에 서버가 자동으로 반영하고 응답에 알림.
- 자기 자신이나 서로를 부르는 정의(재귀)는 안 됨.";

const DEL: &str = "\
del f        f를 지움
del f, g     여러 개
del scene(1) Claude가 만든 scene (기본 scene은 못 지움)
- f를 부르는 함수도 연쇄로 지워지고 응답에 모두 나옴.
- 상수는 직접 지우지 않음. 쓰는 함수가 없어지면 자동으로 사라짐.";

const EXPR: &str = "\
우선순위 (위가 먼저): 호출 f(x) > ! > ^ (오른쪽부터) > 단항 - > * / > + - > 비교 > not > and > or
- 곱셈 '*' 필수: 2*x (2x는 오류). 나눗셈 a/b*c = (a/b)*c.
- -x^2 = -(x^2). ^ 오른쪽은 숫자·이름·괄호 하나: x^(n+1), 2^(-1), x^{1/3}.
- () 보이는 괄호, {} 안 보이는 묶음. 절댓값 abs(x), 근호 sqrt(x).
- 함수 거듭제곱은 sin(x)^2 (sin^2(x)는 오류). 숫자 1.5, 0.5, 1e-3 (.5, 5.는 오류).
- 같음 ==, 같지 않음 not (a == b). 비교는 같은 방향으로 이어 씀: 0 <= x < 5.";

const NAMES: &str = "\
- 이름은 글자로 시작, 숫자 포함 가능, 한글 가능: T1, 넓이.
- 그리스 문자·특수 문자는 LaTeX 이름: \\alpha \\theta \\Delta (θ 같은 유니코드는 오류).
- 예약어: \\pi \\e (\\i 복소수, \\infty는 1단계에서 값으로 못 씀).
- 아래첨자는 중괄호 필수: x_{1}, a_{n} (x_1은 오류). 장식: \\bar{x} \\hat{x} \\vec{v} \\tilde{x} \\dot{x}.";

const SUM: &str = "\
sum(식; 아래 <= 인덱스 <= 위), prod(...)도 같은 모양.
  f(x; n) -> y: sum(x^k/k!; 0 <= k <= n)
- 인덱스는 정수. 범위에 쓰인 상수는 정수 슬라이더가 됨.
- 범위에 x를 쓰면 (k <= x) 정수 x에서만 값이 있어 점으로 그려짐.";

const CONSTANTS: &str = "\
- 상수는 전역, 이름이 같으면 슬라이더 공유. 처음 쓰이면 자동 생성 (값 1, 범위 -10..10; 정수는 5, 0..20).
- 사용자가 슬라이더 값과 범위를 바꿀 수 있음. 현재 값은 get_state로 확인.
- 조건은 constant: 블록으로 선언 (describe constant).";

const CONSTANT_BLOCK: &str = "\
constant:
    a: range=(-5, 5), default=1, step=0.1
    n: integer, range=(1, 50), default=4
- 종류는 맨 앞에 real(생략 시) 또는 integer. integer는 범위·default·step이 모두 정수 (step 생략 시 1).
- 값은 숫자만 (2*\\pi 같은 식은 안 됨, 6.2832로). step 생략 시 범위 폭의 약 1/100.
- 상수는 여전히 함수 괄호 ';' 뒤에 적어야 함. 쓰는 함수가 없는 상수를 선언하면 오류.
- sum 범위에 쓰인 상수를 real로 선언하면 오류.";

const HIGHLIGHT: &str = "\
이름 = highlight(대상; 범위; style; memo)   이름 필수, 대상 외에는 빈칸 가능
  H1 = highlight(f; 0 <= x <= 2; blue; \"정적분 넓이\")   # f와 x축 사이
  H2 = highlight(; between(f, g) and 0 <= x <= 2; green)   # 대상 없음: 영역 칠하기
  H3 = highlight(; f(x) < y < g(x) and y < 1)
  H4 = highlight(f.(b*x).coef;; orange)                    # 식 속의 b만 색칠
- 범위는 부등식 (and / or / not). 범위 생략 = 대상 전체 (무한하면 보이는 범위만).
- 대상 없는 highlight는 범위 칸이 곧 칠할 영역 (x, y, 상수, 함수 호출).
- between(f, g) = f(x) < y < g(x) or g(x) < y < f(x). 인자는 함수 이름만.
- 부분식 대상은 식 색칠만, 범위 없음. 겹치는 채우기는 반투명으로 섞임.
- style 생략 시 자동 색 (만들 때 고정). 가리키던 함수·부분식·변수·style이 없어지면 강조도 지워지고 응답에 나옴.
- 다시 대입하면 수정, del H1로 삭제. 기본 scene에서 강조는 꺼진 채로 시작: 보여주려면 scene의 show나 step의 show: +H1.";

const ADDRESS: &str = "\
식의 일부를 모양으로 가리킴 (위치 번호가 아님):
  f.(b*x)          항 (부호 포함: a - b*x의 둘째 항은 f.(-b*x))
  f.(b*x).coef     입력 변수가 없는 인수 (b)     .var  입력 변수가 든 인수 (x)
  g.den  g.num     분모, 분자          .base  .exp  거듭제곱의 밑, 지수
  h.(sin(3*x)).arg 함수 인자
- 괄호는 무시하고 비교. 같은 모양이 두 곳이면 오류 → 더 큰 항을 가리킨 뒤 내려가기.
- 함수를 수정해도 같은 모양을 다시 찾음. 사라지면 그 강조는 지워짐.";

const STYLE: &str = "\
기본 style: blue orange green red purple yellow gray (축까지 채우기)
사용자 style:
  style up:
      color: orange        # 기본 색 이름 또는 \"#rrggbb\"
      fill: above          # axis(축까지) above(y > f) below(y < f) region(영역)
      opacity: 0.25        # 채우기 투명도 0..1 (기본 0.16)
      width: 2             # 경계선 px (기본 1.5)
- 이름에 '_'는 못 씀 (아래첨자). style을 바꾸면 쓰는 강조도 바뀜.";

const VARS: &str = "\
이름 = 값. 값은 부등식, 문자열 등 아무거나. 참조라서 다시 대입하면 쓰는 강조도 바뀜.
  R1 = 0.9 < x < 1.1
  memo1 = \"x = 1 근처에서 선이 끊겨 보임\"
  H1 = highlight(f; R1;; memo1)
- 함수 식 안에서는 못 씀 (범위·memo 칸에서만). 문자열 이스케이프는 \\\" 와 \\\\ 만.";

const REQ: &str = "\
req(대상; 범위; memo)   사용자가 브라우저에서 남기는 요청사항. 대상만 필수.
  req(f; 0.9 < x < 1.1; \"선이 끊겨 보임\")   req(g.den;; \"부호가 반대 같음\")   req(H1)
- 대상: 함수, 강조, 부분식, scene(1), scene(1).step(3). 요청 당시 단계에 고정 (대상이 지워져도 남음).
- get_requests: 대기 중인 요청사항 + 당시 대상 + 변경 상태(그대로 / 수정됨 / 삭제됨, 간접 원인 포함).
  처리한 것은 get_requests(done=[번호])로 표시.";

const SCENE: &str = "\
scene(번호): 보기 프리셋. 정의는 전역이고 scene은 어떻게 보일지만 정함.
  scene(1):
      space: 2D(x, y)                       # 좌표계 (show의 함수가 한 좌표계면 생략 가능)
      show: f, H1                           # 처음 켜질 함수·강조 (나머지는 꺼짐, 사용자가 토글)
      hide: H2                              # 공개 전까지 목록에서도 숨김 (예측 후 확인)
      clip: -10 <= x <= 10                  # 이 범위 밖은 안 그림 (일반 부등식 가능, 사용자가 조정)
      view: -5 <= x <= 5 and -3 <= y <= 3   # 처음 카메라 (축별 구간, 경계는 숫자 식)
      step:
          show: +T1, -H1
- 새 scene(n)을 만들면 브라우저가 그리로 이동. 같은 번호로 다시 쓰면 블록 전체 교체(수정).
- 좌표계가 안 맞는 정의는 넣을 수 없음 (g(t) -> y는 2D(t, y) scene에만).
- 기본 scene(2D(x, y))은 함수의 좌표계마다 자동으로 생기고 없어짐. 함수는 켜지고 강조는 꺼진 채 시작.
- 참조하던 정의가 지워지면 scene은 남고 그 참조만 빠짐 (응답에 나옴). del scene(1)로 삭제.
- view와 clip이 숫자 구간인데 전혀 안 겹치면 오류.";

const STEP: &str = "\
step: scene 안에 적은 순서대로. 이전 step과 달라진 것만 적음. scene의 show·view가 step 0.
  step:
      show: +H2, -T1                    # 켜기 + / 끄기 -
      set: a = 2, b = -1                # 상수 값 (숫자, 슬라이더 범위 안)
      animate: n = 4 -> 64, 2s, loop    # 시작 -> 끝, 재생 시간(기본 2s), loop = 왕복 반복
      view: -1 <= x <= 1                # 카메라 이동
      note: \"{f}의 접선\"               # 설명 (describe note), note: + \"...\"는 이전 note 아래에 덧붙임
- step은 보기만 바꿈: 계산 과정의 식도 정의로 먼저 선언하고 step에서 켬.
- 앞으로: 그 step이 적은 것만 덮어쓰고 사용자가 바꾼 나머지는 유지. 뒤로: 지나온 step이 바꾼 것만 되돌림
  (상수는 이전 step이 정한 값으로, 정한 적이 없으면 그대로).
- animate는 step에 들어올 때 재생되고 끝값에서 멈춤. 사용자가 슬라이더를 잡으면 멈춤.
- req(scene(1).step(2);; \"...\")로 사용자가 step을 가리킬 수 있음.";

const NOTE: &str = "\
note: 문자열. 축 아래 상자에 보임. step마다 교체, note: + \"...\"는 덧붙임.
  note: \"{f}의 x = 1에서 접선 {T(x)}, 기울기 $\\frac{dy}{dx}$\"
- {식}: DSL 식으로 해석 (오타·없는 이름은 apply 오류). {f}처럼 함수 이름만 쓰면 정의 전체가 보이고 f를 고치면 따라 바뀜.
- $...$: LaTeX 그대로 (검사 안 함). 글자 그대로 쓰려면 {{, }}, $$.
- 문자열 변수도 됨 (note: m): note에 넣을 때 해석, 변수를 바꾸면 note도 바뀜.";

const ERRORS: &str = "\
오류 한 줄 형식: 줄:열 [오류 코드] 설명 → 후보
  3:12 [E-unicode] 'θ'는 쓸 수 없음 → \\theta
- 한 요청은 전부 적용되거나 전부 거절됨. 독립적인 오류는 한 번에 모두 보고됨.
- [E-unsupported]는 다음 단계 기능.";

pub fn describe(name: Option<&str>) -> String {
    let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) else {
        return INDEX.into();
    };
    match name {
        "define" | "정의" => DEFINE.into(),
        "del" | "삭제" => DEL.into(),
        "식" | "연산" | "expr" | "operators" => EXPR.into(),
        "이름" | "names" => NAMES.into(),
        "sum" | "prod" => SUM.into(),
        "상수" | "slider" => CONSTANTS.into(),
        "constant" | "constant:" => CONSTANT_BLOCK.into(),
        "highlight" | "강조" | "between" => HIGHLIGHT.into(),
        "주소" | "address" | "부분식" => ADDRESS.into(),
        "style" => STYLE.into(),
        "변수" | "variable" | "vars" => VARS.into(),
        "req" | "요청사항" | "get_requests" => REQ.into(),
        "오류" | "errors" => ERRORS.into(),
        "scene" | "space" | "clip" | "view" | "show" | "hide" => SCENE.into(),
        "step" | "set" | "animate" => STEP.into(),
        "note" => NOTE.into(),
        "행렬" | "벡터" | "matrix" | "vector" | "matmul" | "pow" | "inv" | "transpose" | "det" | "tr" => MATRIX.into(),
        "span" => SPAN.into(),
        "arrow" => ARROW.into(),
        "함수" | "functions" | "builtins" => {
            let list: Vec<String> = names::BUILTINS
                .iter()
                .map(|(n, a)| if a[0] == 1 { format!("{n}(x)") } else if *n == "log" { "log(x, b)".into() } else { format!("{n}(a, m)") })
                .collect();
            format!("기본 함수: {}\n- 밑 없는 log는 없음: ln, log10, log2, log(x, b).\n- 삼각함수는 라디안, 도는 deg(30).", list.join(", "))
        }
        other => format!("'{other}'에 대한 설명이 없음.\n{INDEX}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_and_topics() {
        assert!(describe(None).starts_with("쓸 수 있는 명령"));
        assert!(describe(Some("define")).contains("f(x; a, b) -> y"));
        assert!(describe(Some("함수")).contains("log(x, b)"));
        assert!(describe(Some("??")).contains("설명이 없음"));
    }

    /// Examples in the help text must be valid DSL.
    #[test]
    fn examples_parse() {
        let src = "define:\n    f(x; a, b) -> y: a*x^2 + b*x + 1\n    g(x; a, b) -> y: f(x) + 1\n    s(x; n) -> y: sum(x^k/k!; 0 <= k <= n)\n    h(x) -> y: sin(3*x)\n    q(x) -> y: x^2/(x - 1)\n";
        let s = dsl::interp::apply(&Default::default(), src, &Default::default(), 1).unwrap().state;
        let stage2 = "constant:\n    a: range=(-5, 5), default=1, step=0.1\n    n: integer, range=(1, 50), default=4\ndefine:\n    t(x; n) -> y: sum(x^k; 1 <= k <= n)\nstyle up:\n    color: orange\n    fill: above\n    opacity: 0.25\n    width: 2\nR1 = 0.9 < x < 1.1\nmemo1 = \"x = 1 근처에서 선이 끊겨 보임\"\nH1 = highlight(f; 0 <= x <= 2; blue; \"정적분 넓이\")\nH2 = highlight(; between(f, g) and 0 <= x <= 2; green)\nH3 = highlight(; f(x) < y < g(x) and y < 1)\nH4 = highlight(f.(b*x).coef;; orange)\nH5 = highlight(f; R1;; memo1)\nH6 = highlight(g;; up)\nA = highlight(h.(sin(3*x)).arg)\nB = highlight(q.den)\nreq(f; 0.9 < x < 1.1; \"선이 끊겨 보임\")\nreq(H1)\n";
        let s = dsl::interp::apply(&s, stage2, &Default::default(), 2).unwrap().state;
        let stage3 = "define:\n    T1(x; a) -> y: a*x\n    f2(x; n) -> y: sum(x^k; 0 <= k <= n)\nconstant:\n    n: integer, range=(1, 80), default=4\nscene(1):\n    space: 2D(x, y)\n    show: f, H1\n    hide: H2\n    clip: -10 <= x <= 10\n    view: -5 <= x <= 5 and -3 <= y <= 3\n    step:\n        show: +T1, -H1\n    step:\n        show: +H2, -T1\n        set: a = 2, b = -1\n        animate: n = 4 -> 64, 2s, loop\n        view: -1 <= x <= 1\n        note: \"{f}의 접선\"\n    step:\n        note: + \"{f}의 x = 1에서 접선 {T1(x)}, 기울기 $\\\\frac{dy}{dx}$\"\nreq(scene(1).step(2);; \"...\")\ndel scene(1)\n";
        dsl::interp::apply(&s, stage3, &Default::default(), 3).unwrap();
        let stage4 = "define:\n    A: [[1, 2],\n        [3, 4]]\n    v(; a): [1, a]\n    R(\\theta) -> matrix: [[cos(\\theta), -sin(\\theta)], [sin(\\theta), cos(\\theta)]]\n    f4(x; a) -> y: matmul(R(x), v)_[2]\n    w: [0, 1]\n    S(; a): span(v, w)\n    p(; a): arrow(v)\n    c4(x) -> y: x\n    q(; c): arrow([1, 2*c]; [c, c^2])\n    M: pow(A, -1) + inv(A) + transpose(A) + 2*A - A/2\n    d(x) -> y: det(A) + tr(A)\n";
        dsl::interp::apply(&Default::default(), stage4, &Default::default(), 4).unwrap();
    }
}
