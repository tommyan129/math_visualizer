//! `describe`: command list without a name, syntax and examples with one.
//! Tool descriptions stay short; details live here (design.md "MCP tool").

use dsl::names;

const INDEX: &str = "\
쓸 수 있는 명령 (6a단계). describe(name)으로 자세히:
  define     함수 정의·수정 (같은 이름에 다시 쓰면 수정)
  split      구간별 함수 (경계점, 조각마다 식)
  매개변수   p(t) -> (x, y): (식, 식), 범위
  극좌표     h(\\theta) -> polar(r), polar(r, \\theta) scene
  방정식     C(x, y): 좌변 = 우변 (음함수 곡선)
  family     family(본문; 0 <= k <= n): 같은 도형을 k마다 반복
  3D         g(x, y) -> z 곡면, q(t) -> (x, y, z), s(u, v) -> (x, y, z), 3D(x, y, z) scene
  도함수     f@n(x) -> y, 역함수 ~f(x) -> y, 정의 안 한 f@n은 서버 기호 미분
  특징점     E: extrema(f), roots, intersect, yint, inflections, argmax, argmin, breaks, tangents
  계산 과정  C: calc(f, g, h): 식 변형 단계, 바뀐 부분 색칠, 수치 검증 (적분, 극한)
  검증       계산 과정, f@n, ~f의 수치 검증 (apply 응답에 결과)
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
아직 없음: Claude가 적는 정확한 값 라벨, 특징점 하나 번호로 가리키기(E_[2]), 3D 임계점(6b), 3D 음함수 곡면";

const SPLIT: &str = "\
define:
    f(x; a, b) -> y:
        split x at a, b            # 경계점은 적은 순서가 오름차순
        -1,     x < a
        x^2,    a <= x <= b
        3 - x,  x > b
    g(x) -> y:
        split r = x^2 at 1         # split 식에 이름을 붙이면 조각에서 그 이름을 씀
        r,      r < 1
        1/r,    r >= 1
- 정의 머리 다음 줄부터 들여써서 split 한 줄, 그 아래 '값, 구간' 줄들. otherwise는 없음.
- 구간은 split 식(또는 그 이름)과 경계점으로만 씀 (and / or / not, 이어 쓰기 가능).
- 서버가 값 계산 없이 겹침 검사: 경계점이 나누는 조각 (-∞, a), {a}, (a, b), ... 하나를 두 줄이 차지하면 오류.
  아무 줄도 차지하지 않은 조각은 정의 안 됨.
- 경계점에 상수가 있으면 순서가 곧 조건: a < b를 브라우저 슬라이더가 넘지 못함. 같은 경계점 두 번은 오류.
- 실제 값에서 두 줄이 겹치면(예: 다른 경로로 a > b) 그 점은 정의 안 됨 + 경고 [W-split-overlap].
- 경계가 서로 다른 식 둘 이상이면 split을 두 단계로 (조각 값에 다른 split 함수를 부름).
- y = f(x) 함수와 극좌표 함수에 씀.";

const DERIV: &str = "\
define:
    f(x; a) -> y: a*x^3
    f@1(x; a) -> y: 3*a*x^2          # 화면엔 f'. f@2 → f'', f@4 → f⁽⁴⁾
    ~f(x; a) -> y: (x/a)^(1/3)        # 화면엔 f⁻¹
    T(x; a, c) -> y: f@2(c)*(x - c)   # 정의 안 한 f@2: 서버가 기호 미분으로 채움
- f@n, ~f를 정의하려면 같은 축의 일반 함수 f가 먼저 있어야 함. del f 하면 f@n, ~f도 함께 지워짐.
- 정의 안 한 f@n을 부르거나 특징점이 도함수를 쓰면 서버가 f의 식을 미분해서 씀 (응답에 '자동: 도함수'로 식을 알림).
  이름은 비어 있어서 나중에 f@n을 정의하면 그걸 씀. 미분 규칙이 없는 식(gamma, !, prod 등)이면 [E-deriv] → f@n 직접 정의.
- ~f는 서버가 만들지 않음: 부르려면 정의해야 함. 미분 횟수는 1..8.";

const FEATURE: &str = "\
define:
    f(x) -> y: x^3 - 3*x
    E: extrema(f)                      # 극대·극소 (f'의 부호가 바뀌는 점)
    M: extrema(f;; max)                # 칸: 대상; 범위; 종류 (빈칸 가능)
    R: roots(f; -1 < x < 2)            # 근 (부호 변화 + 접하는 근)
    I: inflections(f)                  # 변곡점 (f''의 부호 변화; x^4의 0은 아님)
    P: intersect(f, g)                 # 교점. 둘째 칸에 y = 2*x 같은 식도 됨
    Y: yint(f)
    A: argmax(f; -2 <= x <= 3)         # 구간 위 최댓값 (argmin: 최솟값), 양 끝 있는 구간 필수
    B: breaks(f;; hole)                # 구멍, 수직 점근선, 점프, 정의역 끝점, 뾰족점 (종류: hole, asymptote, jump, endpoint, cusp)
    Tg: tangents(p;; vertical)         # 매개변수·극좌표 곡선의 수평·수직 접선 (종류: horizontal, vertical)
    Pm: extrema(f;; min)
    L(x) -> y: f@1(Pm_[1])*(x - Pm_[1]) + Pm_[2]    # 점이 하나면 값 [x, y]로 씀
- 특징점은 define 안의 이름 붙은 정의 하나: 토글, del cascade, req, scene show, step +E가 다른 정의와 같음.
- 대상 함수의 상수를 머리에 적어야 함: f(x; a)면 E(; a): extrema(f). 범위·식 칸의 상수도.
- 점은 브라우저가 찾음 (범위가 없으면 계산 범위 = 화면 ± 한 화면). get_state와 apply 응답에 좌표가 옴.
- 값으로 쓸 때(E_[1], E_[2], arrow(v; E)) 점이 정확히 하나여야 함: 아니면 그 식은 정의 안 됨 + [W-feature-value].
- 결과 하나를 번호로 가리키기(E_[2])는 아직 없음: 범위 칸으로 좁혀서 하나만 남게.
- 극좌표 tangents의 좌표는 (r, θ).";

const CALC: &str = "\
define:
    f(x) -> y: (x+1)^2 - (x-1)^2
    g(x) -> y: (x^2 + 2*x + 1) - (x^2 - 2*x + 1)
    h(x) -> y: 4*x
    C: calc(f, g, h)                   # 이웃한 단계끼리 바뀐 부분 색칠 + 같은지 수치 검증
    s(x) -> y: sqrt(x^2)
    t(x) -> y: x
    D: calc(s, t; x > 0)               # 칸: 단계; 범위; 종류. 범위 안의 점에서만 검증
    p(x) -> y: x^2
    P(x) -> y: x^3/3
    I: calc(p, P;; integral)           # P' = p를 수치 미분으로 확인 (상수 차이는 무시)
    L: calc(p, P;; limit)              # 극한: '검증 안 됨'
scene(1):
    show: f, C
    step:
        show: +g                       # 패널의 C가 켜진 단계까지 한 줄씩 늘어남
    step:
        show: +h
- 계산 과정의 각 줄은 먼저 정의로 선언하고 calc로 묶음 (note 문자열의 식은 검증 안 됨). 단계는 같은 축의 일반·극좌표 함수.
- integral: 첫 두 단계가 적분 관계 (둘째의 도함수 = 첫째), 그 뒤 단계는 둘째의 식 변형.
- 단계 함수의 상수를 머리에 적어야 함: C(; a): calc(f, g). del g 하면 C도 지워짐.
- 검증 결과는 apply 응답의 '검증 C:' 줄과 get_state에. describe(검증)에 세부.";

const VERIFY: &str = "\
수치 검증 (브라우저가 계산, 서버가 응답으로 전달):
- calc 이웃 단계: 같은 값인지 (integral은 둘째의 수치 미분 = 첫째, limit은 '검증 안 됨').
- Claude가 정의한 f@n: 바로 아래 차수(f@(n-1), 없으면 f)의 수치 미분과 비교. 서버가 만든 도함수는 검증 대상 아님.
- Claude가 정의한 ~f: f(~f(x)) = x (~f가 정의된 점에서).
- 점: 범위 안 16개 (범위 없으면 -5..5, 극좌표는 0..2π), 깨지는 점 후보 근처는 피함.
  상수는 현재 값 + 슬라이더 범위 안의 무작위 2벌 → 한 상수 값에서만 우연히 맞는 오류도 잡음.
- 허용 오차: 식 변형·역함수 1e-9, 1계 수치 미분 1e-6 (상대, 값이 1보다 작으면 절대), 2계 이상 1e-4.
- 결과: 일치 / 불일치 (첫 반례: 입력·상수·양쪽 값) / 조건부 일치 (한쪽만 정의된 점이 있음, 정의역이 다름 → 범위 칸으로)
  / 검증 불가 (둘 다 정의된 점이 4개 미만) / 검증 안 됨 (극한).
- apply는 브라우저 결과를 최대 3초 기다림. 이번 요청이 건드린 정의의 검증과 일치가 아닌 검증이 응답에 옴.";

const PARAMETRIC: &str = "\
define:
    p(t) -> (x, y): (cos(t), sin(t)), 0 <= t <= 2*\\pi
    q(t; a) -> (x, y): matmul(R(t), [a, 0]), 0 <= t <= \\pi     # 길이가 출력 축 개수인 벡터도 됨
    D(t; k) -> (x, y): (k, t), 0 <= t <= f(k)                   # x = k의 세로 선분
- 매개변수 범위는 필수 (빠지면 오류). 범위 끝에는 상수·함수 호출 가능, t는 안 됨.
- 출력 축 (x, y)가 좌표계: 2D(x, y) scene에 들어감. 출력 축 세 개는 3D (describe 3D).
- 출력 칸 하나에는 숫자 하나: 매개변수 함수 호출이나 벡터를 칸 하나에 넣으면 오류.
- 매개변수 함수는 식 안에서 부를 수 없음 (출력 전체 자리 p(2*t)만 가능).
- 아직 강조 대상이 될 수 없음.";

const POLAR: &str = "\
define:
    h(\\theta) -> polar(r): 1 + cos(\\theta)                       # 범위 생략 = 0 <= θ <= 2π
    s(\\theta) -> polar(r): \\theta/4, 0 <= \\theta <= 6*\\pi
scene(1):
    space: polar(r, \\theta)       # 극좌표 scene (show의 함수가 극좌표면 생략 가능)
    show: h
    view: r <= 3                    # 반지름 3까지 보이는 정사각형 창
    clip: r <= 2 and 0 <= \\theta <= \\pi
- 극좌표 함수는 polar(r, \\theta) scene에만 들어감 (기본 scene도 따로 생김). 2D(x, y) 함수는 못 넣음.
- view는 반지름만 (r <= 3), clip은 r, θ 부등식. θ는 0..2π (또는 한 바퀴 아래로 읽음).
- 다른 함수에서 h(t)로 부르면 r 값. split 본문 가능. 아직 강조 대상이 될 수 없음.";

const EQUATION: &str = "\
define:
    C(x, y; a): x^2 + y^2 = a^2
    L(x, y): x*y = 1
- '->' 없이 ':' 오른쪽에 '='가 있으면 방정식. 입력 두 개가 좌표계 (C는 2D(x, y)).
- 값이 없어서 부를 수 없음 (C(1, 2)는 오류). 아직 강조 대상이 될 수 없음.
- 닫힌 곡선의 안쪽을 칠하려면 대상 없는 highlight: highlight(; x^2 + y^2 <= a^2).";

const EACH: &str = "\
define:
    D(t; a, h, n) -> (x, y): family((a + k*h, t), 0 <= t <= f(a + k*h); 0 <= k <= n)   # 리만 합 칸 구분선
    O(x, y; m): family(x^2 + y^2 = k^2; 1 <= k <= m)                                    # 동심원
    F(x) -> y: family(sin(x + k*\\pi/4); 0 <= k <= 3)
    G(x, y) -> matrix: [-y, x]
    V(; N): family(arrow(G(mod(k, N) - N/2, floor(k/N) - N/2)/N; [mod(k, N) - N/2, floor(k/N) - N/2]); 0 <= k <= N^2 - 1)   # 벡터장 G
- family(본문; 아래 <= k <= 위): 본문을 정수 k마다 하나씩 그림 (화면엔 Ш). 본문은 한 줄 정의 본문 그대로
  (식 / '식, 범위' / '좌변 = 우변' / arrow(v; P)). 곡선, 매개변수, 극좌표, 방정식, arrow에 씀 (span은 안 됨).
- 격자에 화살표 깔기: k 하나로 열 mod(k, N), 행 floor(k/N) (N×N개, 최대 500개).
- k는 정수. 범위에 쓰인 상수는 정수 슬라이더가 됨 (sum과 같음). 개수는 현재 값으로 정해짐 (최대 500개).
- 정의 하나라서 토글·삭제·req도 하나. 식 안에서 부를 수 없고, 아직 강조 대상이 될 수 없음.";

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
- 길이 2 벡터는 2D scene(어느 축 이름이든)에, 길이 3은 3D scene에. 기본 scene은 2D(x, y) / 3D(x, y, z).
  3D에서는 평행하지 않은 두 벡터가 원점을 지나는 평면, 세 벡터가 일차독립이면 공간 전체.
- 보이는 범위만 그림, scene clip이 있으면 그 안만. 그린 차원은 apply 응답과 get_state에 나옴.";

const ARROW: &str = "\
define:
    p(; a): arrow(v)                    # 원점에서 v까지
    q(; c): arrow([1, 2*c]; [c, c^2])   # 점 P에서 출발: 시작점은 ';' 뒤, 위치벡터로 (y = x^2의 접선 방향)
- 시작점과 벡터는 같은 길이. 2D scene에는 길이 2, 3D scene에는 길이 3.
- 함수처럼 토글·삭제·req 대상. 기본 scene에서 켜진 채 시작.";

const THREE_D: &str = "\
define:
    g(x, y) -> z: x^2 - y^2                                          # 곡면 (입력 둘 + 출력 축)
    q(t; a) -> (x, y, z): (cos(t), sin(t), a*t), 0 <= t <= 4*\\pi     # 공간 곡선
    s(u, v) -> (x, y, z): (cos(u)*sin(v), sin(u)*sin(v), cos(v)), 0 <= u <= 2*\\pi, 0 <= v <= \\pi
    S: span([1, 0, 0], [0, 1, 1])                                    # 원점을 지나는 평면
    w: arrow([1, 2, 3])
scene(1):
    space: 3D(x, y, z)                         # show가 3D 정의뿐이면 생략 가능
    show: g, q
    view: -3 <= x <= 3 and -3 <= y <= 3 and -2 <= z <= 2   # 계산 범위 (빠진 축은 -5..5), 처음 카메라가 전부 보이게 놓임
    clip: x^2 + y^2 <= 4                       # x, y, z 부등식, 밖은 안 그림
H1 = highlight(g; x^2 + y^2 < 1)               # 곡면의 그 부분을 강조 색으로
- 곡면 g는 view의 x, y 범위 위에서 그리고 z가 view의 z 범위를 벗어나면 잘림 (범위 상자는 화면에 안 그림). 범위를 붙이지 않음.
  다른 식에서 g(x, 1)처럼 숫자로 부를 수 있음. 2D scene에는 못 넣음 (등고선은 아직 없음).
- 매개변수 곡면은 매개변수마다 범위 하나 (쉼표로, 순서 자유, 둘 다 필수). 범위 끝에 매개변수는 안 됨.
- 화면(사용자 조작): 자유 카메라. 왼쪽 끌기 = 화면과 평행 이동, 오른쪽 끌기 = 시선 돌리기, W/A/S/D = 이동, Space/Shift = 위/아래, 휠 = 가리킨 곳으로 다가가기·물러나기 (사용자가 설정에서 바꿀 수 있음).
- 대상 없는 강조(입체 영역)는 3D scene에 넣을 수 없음. 곡면의 특징점(extrema)은 6b, 곡면의 f@n·~f·calc·between은 없음.";

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
        "split" | "구간별" => SPLIT.into(),
        "매개변수" | "parametric" => PARAMETRIC.into(),
        "극좌표" | "polar" => POLAR.into(),
        "방정식" | "음함수" | "equation" | "implicit" => EQUATION.into(),
        "family" | "반복" => EACH.into(),
        "3D" | "3d" | "곡면" | "surface" | "다변수" => THREE_D.into(),
        "도함수" | "역함수" | "f@n" | "~f" | "derivative" | "inverse" => DERIV.into(),
        "계산 과정" | "계산" | "calc" | "integral" | "limit" => CALC.into(),
        "검증" | "수치 검증" | "verify" | "verification" => VERIFY.into(),
        "특징점" | "feature" | "features" | "roots" | "intersect" | "yint" | "extrema" | "inflections" | "argmax" | "argmin" | "breaks" | "tangents" => FEATURE.into(),
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
        let stage5 = "define:\n    R(\\theta) -> matrix: [[cos(\\theta), -sin(\\theta)], [sin(\\theta), cos(\\theta)]]\n    f(x; a, b) -> y:\n        split x at a, b\n        -1,     x < a\n        x^2,    a <= x <= b\n        3 - x,  x > b\n    g(x) -> y:\n        split r = x^2 at 1\n        r,      r < 1\n        1/r,    r >= 1\n    p(t) -> (x, y): (cos(t), sin(t)), 0 <= t <= 2*\\pi\n    q(t; a) -> (x, y): matmul(R(t), [a, 0]), 0 <= t <= \\pi\n    D1(t; k) -> (x, y): (k, t), 0 <= t <= g(k)\n    h(\\theta) -> polar(r): 1 + cos(\\theta)\n    s(\\theta) -> polar(r): \\theta/4, 0 <= \\theta <= 6*\\pi\n    C(x, y; a): x^2 + y^2 = a^2\n    L(x, y): x*y = 1\n    D(t; a, w, n) -> (x, y): family((a + j*w, t), 0 <= t <= g(a + j*w); 0 <= j <= n)\n    O(x, y; m): family(x^2 + y^2 = j^2; 1 <= j <= m)\n    F(x) -> y: family(sin(x + j*\\pi/4); 0 <= j <= 3)\n    G(x, y) -> matrix: [-y, x]\n    V(; N): family(arrow(G(mod(j, N) - N/2, floor(j/N) - N/2)/N; [mod(j, N) - N/2, floor(j/N) - N/2]); 0 <= j <= N^2 - 1)\nscene(1):\n    space: polar(r, \\theta)\n    show: h\n    view: r <= 3\n    clip: r <= 2 and 0 <= \\theta <= \\pi\nH1 = highlight(; x^2 + y^2 <= a^2)\n";
        dsl::interp::apply(&Default::default(), stage5, &Default::default(), 5).unwrap();
        let stage5b = "define:\n    f(x; a) -> y: a*x^3\n    f@1(x; a) -> y: 3*a*x^2\n    ~f(x; a) -> y: (x/a)^(1/3)\n    T(x; a, c) -> y: f@2(c)*(x - c)\n    g(x) -> y: x^3 - 3*x\n    h(x) -> y: 2*x\n    p(t) -> (x, y): (cos(t), sin(t)), 0 <= t <= 2*\\pi\n    E: extrema(g)\n    M: extrema(g;; max)\n    R: roots(g; -1 < x < 2)\n    I: inflections(g)\n    P: intersect(g, h)\n    P2: intersect(g, y = 2*x)\n    Y: yint(g)\n    A: argmax(g; -2 <= x <= 3)\n    B: breaks(g;; hole)\n    Tg: tangents(p;; vertical)\n    Pm: extrema(g;; min)\n    L(x) -> y: g@1(Pm_[1])*(x - Pm_[1]) + Pm_[2]\n    V: arrow([1, 0]; Pm)\n";
        dsl::interp::apply(&Default::default(), stage5b, &Default::default(), 6).unwrap_or_else(|d| panic!("{d:?}"));
        let stage5c = "define:\n    f(x) -> y: (x+1)^2 - (x-1)^2\n    g(x) -> y: (x^2 + 2*x + 1) - (x^2 - 2*x + 1)\n    h(x) -> y: 4*x\n    C: calc(f, g, h)\n    s(x) -> y: sqrt(x^2)\n    t(x) -> y: x\n    D: calc(s, t; x > 0)\n    p(x) -> y: x^2\n    P(x) -> y: x^3/3\n    I: calc(p, P;; integral)\n    L: calc(p, P;; limit)\nscene(1):\n    show: f, C\n    step:\n        show: +g\n    step:\n        show: +h\n";
        dsl::interp::apply(&Default::default(), stage5c, &Default::default(), 7).unwrap_or_else(|d| panic!("{d:?}"));
        let stage6 = "define:\n    g(x, y) -> z: x^2 - y^2\n    q(t; a) -> (x, y, z): (cos(t), sin(t), a*t), 0 <= t <= 4*\\pi\n    s(u, v) -> (x, y, z): (cos(u)*sin(v), sin(u)*sin(v), cos(v)), 0 <= u <= 2*\\pi, 0 <= v <= \\pi\n    S: span([1, 0, 0], [0, 1, 1])\n    w: arrow([1, 2, 3])\nscene(1):\n    space: 3D(x, y, z)\n    show: g, q\n    view: -3 <= x <= 3 and -3 <= y <= 3 and -2 <= z <= 2\n    clip: x^2 + y^2 <= 4\nH1 = highlight(g; x^2 + y^2 < 1)\n";
        dsl::interp::apply(&Default::default(), stage6, &Default::default(), 8).unwrap_or_else(|d| panic!("{d:?}"));
        assert!(describe(Some("3D")).contains("s(u, v) -> (x, y, z)"));
        assert!(describe(Some("calc")).contains("calc(f, g, h)"));
        assert!(describe(Some("검증")).contains("f(~f(x)) = x"));
        assert!(describe(Some("특징점")).contains("extrema"));
        assert!(describe(Some("f@n")).contains("~f"));
    }
}
